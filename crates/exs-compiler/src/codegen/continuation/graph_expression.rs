//! Expression lowering for continuation graphs.

use crate::ast::{BinaryOperator, Expression};
use crate::codegen::diagnostics;
use crate::codegen::types::TypeContract;
use crate::codegen::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

use super::graph::{BindingSlot, GraphBuilder, Operation, expression_span};

impl<'source, 'function> GraphBuilder<'source, 'function> {
    pub(super) fn lower_expression(
        &mut self,
        expression: &'function Expression<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        match expression {
            Expression::Integer(_, _)
            | Expression::Float(_, _)
            | Expression::String(_, _)
            | Expression::Bool(_, _)
            | Expression::None(_) => {
                let destination = self.temporary(expression_span(expression))?;
                self.operations.push(Operation::Literal {
                    expression,
                    destination,
                });
                Ok(destination)
            }
            Expression::Variable(identifier) => {
                let binding = self.lookup(&identifier.name, identifier.span)?;
                if binding.cell {
                    let destination = self.temporary(identifier.span)?;
                    self.operations.push(Operation::CellGet {
                        cell: binding.slot,
                        destination,
                        span: identifier.span,
                    });
                    Ok(destination)
                } else {
                    Ok(binding.slot)
                }
            }
            Expression::Closure { span, .. } => {
                let lifted = self
                    .lifted
                    .iter()
                    .find(|closure| closure.declaration.span == *span)
                    .ok_or_else(|| {
                        diagnostics(CompileDiagnostic::new(
                            "E0999",
                            *span,
                            "missing lifted closure declaration",
                        ))
                    })?;
                let layout = self
                    .frame_layouts
                    .get(&lifted.key)
                    .copied()
                    .ok_or_else(|| {
                        diagnostics(CompileDiagnostic::new(
                            "E0999",
                            *span,
                            "missing lifted closure frame layout",
                        ))
                    })?;
                let mut captures = Vec::with_capacity(lifted.captures.len());
                for name in &lifted.captures {
                    let binding = self.lookup(name, *span)?;
                    if !binding.cell {
                        return Err(diagnostics(CompileDiagnostic::new(
                            "E0999",
                            *span,
                            "closure capture was not lowered to shared Cell storage",
                        )));
                    }
                    captures.push(binding.slot);
                }
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::Closure {
                    layout,
                    arity: lifted.declaration.parameters.len(),
                    captures,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::ParallelStatic { tasks, span } => {
                let mut closures = Vec::with_capacity(tasks.len());
                for task in tasks {
                    closures.push(self.lower_expression(task)?);
                }
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::ParallelStart {
                    tasks: closures,
                    destination,
                    span: *span,
                });
                self.operations.push(Operation::ParallelTake {
                    group: destination,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::ParallelDynamic { functions, span } => {
                let functions = self.lower_expression(functions)?;
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::ParallelDynamicStart {
                    functions,
                    destination,
                    span: *span,
                });
                self.operations.push(Operation::ParallelTake {
                    group: destination,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::Unary {
                operator,
                operand,
                span,
            } => {
                let operand = self.lower_expression(operand)?;
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::Unary {
                    operator: *operator,
                    operand,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::Binary {
                operator,
                left,
                right,
                span,
            } => {
                if matches!(operator, BinaryOperator::And | BinaryOperator::Or) {
                    return self.lower_logical(
                        left,
                        right,
                        matches!(operator, BinaryOperator::Or),
                        *span,
                    );
                }
                let left = self.lower_expression(left)?;
                let right = self.lower_expression(right)?;
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::Binary {
                    operator: *operator,
                    left,
                    right,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::List { elements, span } => {
                let mut slots = Vec::with_capacity(elements.len());
                for element in elements {
                    slots.push(self.lower_expression(element)?);
                }
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::List {
                    elements: slots,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::Object { properties, span } => {
                let mut slots = Vec::with_capacity(properties.len());
                for property in properties {
                    slots.push((
                        property.key.as_str(),
                        property.key_span,
                        self.lower_expression(&property.value)?,
                    ));
                }
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::Object {
                    properties: slots,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::IsError { value, span } => {
                let value = self.lower_expression(value)?;
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::IsError {
                    value,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::Propagate { value, span } => {
                if !self.permits_error {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0218",
                        *span,
                        "? requires the current function return type to include Error or Any",
                    )));
                }
                let value = self.lower_expression(value)?;
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::Propagate {
                    value,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::HostCall {
                name,
                arguments,
                span,
            } => {
                let name = self.lower_expression(name)?;
                let mut slots = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    slots.push(self.lower_expression(argument)?);
                }
                let argument_list = self.temporary(*span)?;
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::HostCall {
                    name,
                    arguments: slots,
                    argument_list,
                    destination,
                    span: *span,
                });
                self.operations.push(Operation::HostResume {
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::Index {
                receiver,
                index,
                span,
            } => {
                let receiver = self.lower_expression(receiver)?;
                let index = self.lower_expression(index)?;
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::Index {
                    receiver,
                    index,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::Property {
                receiver,
                property,
                span,
            } => {
                let receiver = self.lower_expression(receiver)?;
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::Property {
                    receiver,
                    property: property.name.clone(),
                    property_span: property.span,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::Call {
                callee,
                arguments,
                span,
            } => {
                if let Some(binding) = self.lookup_optional(&callee.name) {
                    let closure = if binding.cell {
                        let destination = self.temporary(callee.span)?;
                        self.operations.push(Operation::CellGet {
                            cell: binding.slot,
                            destination,
                            span: callee.span,
                        });
                        destination
                    } else {
                        binding.slot
                    };
                    let mut slots = Vec::with_capacity(arguments.len());
                    for argument in arguments {
                        slots.push(self.lower_expression(argument)?);
                    }
                    let destination = self.temporary(*span)?;
                    self.operations.push(Operation::ClosureCall {
                        closure,
                        arguments: slots,
                        destination,
                        span: *span,
                    });
                    return Ok(destination);
                }
                let signature = self.signatures.get(&callee.name).cloned().ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0207",
                        callee.span,
                        format!("unknown function `{}`", callee.name),
                    ))
                })?;
                if signature.arity != arguments.len() {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0208",
                        *span,
                        format!(
                            "function `{}` expects {} arguments but received {}",
                            callee.name,
                            signature.arity,
                            arguments.len()
                        ),
                    )));
                }
                let mut slots = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    slots.push(self.lower_expression(argument)?);
                }
                let destination = self.temporary(*span)?;
                if let Some(layout) = self.frame_layouts.get(&callee.name).copied() {
                    self.operations.push(Operation::ChildCall {
                        layout,
                        arguments: slots,
                        destination,
                        span: *span,
                    });
                } else {
                    self.operations.push(Operation::DirectCall {
                        signature,
                        arguments: slots,
                        destination,
                        span: *span,
                    });
                }
                Ok(destination)
            }
            Expression::StaticMethodCall {
                type_name,
                method,
                arguments,
                span,
            } => {
                let key = format!("{}::{}", type_name.name, method.name);
                let signature = self.signatures.get(&key).cloned().ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0225",
                        method.span,
                        format!(
                            "type `{}` has no static method `{}`",
                            type_name.name, method.name
                        ),
                    ))
                })?;
                if !self.methods.is_static(&key) {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0225",
                        method.span,
                        format!(
                            "type `{}` method `{}` requires a receiver",
                            type_name.name, method.name
                        ),
                    )));
                }
                if signature.arity != arguments.len() {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0208",
                        *span,
                        format!(
                            "static method `{}::{}` expects {} arguments but received {}",
                            type_name.name,
                            method.name,
                            signature.arity,
                            arguments.len()
                        ),
                    )));
                }
                let mut slots = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    slots.push(self.lower_expression(argument)?);
                }
                let destination = self.temporary(*span)?;
                if let Some(layout) = self.frame_layouts.get(&key).copied() {
                    self.operations.push(Operation::ChildCall {
                        layout,
                        arguments: slots,
                        destination,
                        span: *span,
                    });
                } else {
                    self.operations.push(Operation::DirectCall {
                        signature,
                        arguments: slots,
                        destination,
                        span: *span,
                    });
                }
                Ok(destination)
            }
            Expression::MethodCall {
                receiver,
                method,
                arguments,
                span,
            } => {
                let receiver = self.lower_expression(receiver)?;
                let mut slots = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    slots.push(self.lower_expression(argument)?);
                }
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::InstanceCall {
                    receiver,
                    method: &method.name,
                    method_span: method.span,
                    arguments: slots,
                    targets: self
                        .methods
                        .instance(&method.name)
                        .unwrap_or_default()
                        .to_vec(),
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::TypedObject {
                type_name,
                properties,
                span,
            } => self.lower_typed_object(type_name, properties, *span),
        }
    }

    /// Lowers one short-circuiting Boolean expression into explicit continuation branches.
    pub(super) fn lower_logical(
        &mut self,
        left: &'function Expression<'source>,
        right: &'function Expression<'source>,
        is_or: bool,
        span: SourceSpan<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        let left = self.lower_expression(left)?;
        let destination = self.temporary(span)?;
        let checked = self.temporary(span)?;
        let branch = self.push(Operation::Branch {
            condition: left,
            checked,
            when_true: 0,
            when_false: 0,
            span,
        })?;

        let right_start = self.operations.len();
        let right = self.lower_expression(right)?;
        let right_checked = self.temporary(span)?;
        let right_branch = self.push(Operation::Branch {
            condition: right,
            checked: right_checked,
            when_true: 0,
            when_false: 0,
            span,
        })?;
        let right_true = self.operations.len();
        self.operations.push(Operation::Boolean {
            value: true,
            destination,
            span,
        });
        let right_true_exit = self.push(Operation::Goto {
            target: 0,
            checkpoint: false,
            span,
        })?;
        let right_false = self.operations.len();
        self.operations.push(Operation::Boolean {
            value: false,
            destination,
            span,
        });
        let right_false_exit = self.push(Operation::Goto {
            target: 0,
            checkpoint: false,
            span,
        })?;

        let short_start = self.operations.len();
        self.operations.push(Operation::Boolean {
            value: is_or,
            destination,
            span,
        });
        let short_exit = self.push(Operation::Goto {
            target: 0,
            checkpoint: false,
            span,
        })?;
        let after = self.operations.len();

        if is_or {
            self.set_branch_targets(branch, short_start, right_start, span)?;
        } else {
            self.set_branch_targets(branch, right_start, short_start, span)?;
        }
        self.set_branch_targets(right_branch, right_true, right_false, span)?;
        self.set_goto_target(right_true_exit, after, span)?;
        self.set_goto_target(right_false_exit, after, span)?;
        self.set_goto_target(short_exit, after, span)?;
        Ok(destination)
    }

    /// Lowers nominal construction while preserving direct-lowering field validation order.
    pub(super) fn lower_typed_object(
        &mut self,
        type_name: &'function crate::ast::Identifier<'source>,
        properties: &'function [crate::ast::ObjectProperty<'source>],
        span: SourceSpan<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        let nominal = self.types.get(&type_name.name).cloned().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0216",
                type_name.span,
                format!("unknown type `{}`", type_name.name),
            ))
        })?;
        for property in properties {
            if !nominal
                .fields
                .iter()
                .any(|field| field.name == property.key)
                || properties
                    .iter()
                    .filter(|other| other.key == property.key)
                    .count()
                    > 1
            {
                let value = self.temporary(property.key_span)?;
                self.operations.push(Operation::None {
                    destination: value,
                    span: property.key_span,
                });
                self.operations.push(Operation::ValidateSlot {
                    slot: value,
                    contract: TypeContract {
                        builtin_mask: 0,
                        nominal_type_ids: Vec::new(),
                    },
                    span: property.key_span,
                });
                return Ok(value);
            }
        }

        let object = self.temporary(span)?;
        self.operations.push(Operation::TypedObject {
            type_id: nominal.id,
            destination: object,
            span,
        });
        for property in properties {
            let field = nominal
                .fields
                .iter()
                .find(|field| field.name == property.key)
                .cloned()
                .ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0999",
                        property.key_span,
                        "missing resolved nominal field",
                    ))
                })?;
            let value = self.lower_expression(&property.value)?;
            self.operations.push(Operation::ValidateSlot {
                slot: value,
                contract: field.contract,
                span: property.span,
            });
            self.operations.push(Operation::PropertySet {
                receiver: object,
                property: property.key.clone(),
                property_span: property.key_span,
                value,
                span: property.span,
            });
        }
        for field in &nominal.fields {
            if properties.iter().any(|property| property.key == field.name) {
                continue;
            }
            let value = self.temporary(span)?;
            self.operations.push(Operation::None {
                destination: value,
                span,
            });
            self.operations.push(Operation::ValidateSlot {
                slot: value,
                contract: field.contract.clone(),
                span,
            });
            self.operations.push(Operation::PropertySet {
                receiver: object,
                property: field.name.clone(),
                property_span: span,
                value,
                span,
            });
        }
        Ok(object)
    }

    /// Allocates one durable frame slot.
    pub(super) fn temporary(
        &mut self,
        span: SourceSpan<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        let slot = self.next_slot;
        self.next_slot = self.next_slot.checked_add(1).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                span,
                "too many continuation frame slots",
            ))
        })?;
        Ok(slot)
    }

    /// Resolves one lexical binding to its durable slot and storage representation.
    pub(super) fn lookup(
        &self,
        name: &str,
        span: SourceSpan<'source>,
    ) -> Result<BindingSlot, CompileDiagnostics<'source>> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0205",
                    span,
                    format!("unknown binding `{name}`"),
                ))
            })
    }

    /// Resolves one lexical binding without producing an unknown-name diagnostic.
    pub(super) fn lookup_optional(&self, name: &str) -> Option<BindingSlot> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }
}
