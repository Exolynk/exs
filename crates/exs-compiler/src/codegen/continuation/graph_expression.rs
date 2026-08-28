//! Expression lowering for continuation graphs.

use std::collections::HashMap;

use crate::ast::{
    BinaryOperator, Expression, FormattedStringPart, HostTimeOperation, MatchArmBody, MatchPattern,
    UnaryOperator,
};
use crate::codegen::diagnostics;
use crate::codegen::trait_registry::TraitOperator;
use crate::codegen::types::{NominalKind, TypeContract};
use crate::codegen::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

use super::graph::{BindingSlot, GraphBuilder, HostTimeField, Operation, expression_span};

impl<'source, 'function> GraphBuilder<'source, 'function> {
    pub(super) fn lower_expression(
        &mut self,
        expression: &'function Expression<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        match expression {
            Expression::Integer(_, _)
            | Expression::Float(_, _)
            | Expression::String(_, _)
            | Expression::Bytes(_, _)
            | Expression::Bool(_, _)
            | Expression::None(_) => {
                let destination = self.temporary(expression_span(expression))?;
                self.operations.push(Operation::Literal {
                    expression,
                    destination,
                });
                Ok(destination)
            }
            Expression::FormattedString { parts, span, .. } => {
                let mut result = self.temporary(*span)?;
                self.operations.push(Operation::String {
                    value: "",
                    destination: result,
                    span: *span,
                });
                for part in parts {
                    let value = match part {
                        FormattedStringPart::Text(value) => {
                            let destination = self.temporary(*span)?;
                            self.operations.push(Operation::String {
                                value,
                                destination,
                                span: *span,
                            });
                            destination
                        }
                        FormattedStringPart::Expression(expression) => {
                            let receiver = self.lower_expression(expression)?;
                            let destination = self.temporary(*span)?;
                            self.operations.push(Operation::InstanceCall {
                                receiver,
                                method: crate::codegen::standard::TO_STRING_METHOD,
                                method_span: *span,
                                arguments: Vec::new(),
                                targets: self
                                    .methods
                                    .trait_instance(
                                        crate::codegen::standard::TO_STRING_TRAIT,
                                        crate::codegen::standard::TO_STRING_METHOD,
                                    )
                                    .map_or_else(Vec::new, ToOwned::to_owned),
                                destination,
                                span: *span,
                            });
                            destination
                        }
                    };
                    let destination = self.temporary(*span)?;
                    self.operations.push(Operation::Binary {
                        operator: BinaryOperator::Add,
                        left: result,
                        right: value,
                        destination,
                        span: *span,
                    });
                    result = destination;
                }
                Ok(result)
            }
            Expression::Variable(identifier) => {
                if let Some(variant) = self.types.enum_variant(&identifier.name) {
                    if !variant.fields.is_empty() {
                        return Err(diagnostics(CompileDiagnostic::new(
                            "E0208",
                            identifier.span,
                            format!(
                                "enum variant `{}` expects {} arguments",
                                variant.name,
                                variant.fields.len()
                            ),
                        )));
                    }
                    let destination = self.temporary(identifier.span)?;
                    let type_identity_slot = self.temporary(identifier.span)?;
                    let variant_slot = self.temporary(identifier.span)?;
                    self.operations.push(Operation::Enum {
                        type_id: variant.type_id,
                        type_identity: variant.type_identity.clone(),
                        variant: variant.name.clone(),
                        fields: Vec::new(),
                        type_identity_slot,
                        variant_slot,
                        destination,
                        span: identifier.span,
                    });
                    return Ok(destination);
                }
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
                    arity: lifted
                        .declaration
                        .parameters
                        .len()
                        .saturating_sub(usize::from(
                            lifted
                                .declaration
                                .parameters
                                .last()
                                .is_some_and(|parameter| parameter.variadic),
                        )),
                    variadic: lifted
                        .declaration
                        .parameters
                        .last()
                        .is_some_and(|parameter| parameter.variadic),
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
                if matches!(operator, UnaryOperator::Negate)
                    && let Expression::Integer(value, operand_span) = operand.as_ref()
                {
                    let value = value
                        .checked_neg()
                        .and_then(|value| i64::try_from(value).ok())
                        .ok_or_else(|| {
                            diagnostics(CompileDiagnostic::new(
                                "E0206",
                                *operand_span,
                                "integer literal is outside the ExS signed 64-bit range",
                            ))
                        })?;
                    let destination = self.temporary(*span)?;
                    self.operations.push(Operation::Integer {
                        value,
                        destination,
                        span: *span,
                    });
                    return Ok(destination);
                }
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
                if let Some(operator_trait) = TraitOperator::from_binary(*operator) {
                    let result = self.temporary(*span)?;
                    self.operations.push(Operation::Operator {
                        operator: operator_trait,
                        left,
                        right,
                        targets: self.methods.operator(operator_trait).to_vec(),
                        destination: result,
                        span: *span,
                    });
                    if let Some(test) = operator_trait.comparison_test() {
                        self.operations.push(Operation::OrderingTest {
                            value: result,
                            test,
                            destination,
                            span: *span,
                        });
                    } else {
                        self.operations.push(Operation::Copy {
                            source: result,
                            destination,
                            span: *span,
                        });
                    }
                } else {
                    self.operations.push(Operation::Binary {
                        operator: *operator,
                        left,
                        right,
                        destination,
                        span: *span,
                    });
                }
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
            Expression::HostStream { arguments, span } => self.lower_host_stream(arguments, *span),
            Expression::HostTime {
                operation,
                arguments,
                span,
            } => self.lower_host_time(*operation, arguments, *span),
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
                if callee.name == "Error" {
                    if arguments.len() != 3 {
                        return Err(diagnostics(CompileDiagnostic::new(
                            "E0208",
                            *span,
                            format!(
                                "constructor `Error` expects 3 arguments but received {}",
                                arguments.len()
                            ),
                        )));
                    }
                    let kind = self.lower_expression(&arguments[0])?;
                    let message = self.lower_expression(&arguments[1])?;
                    let data = self.lower_expression(&arguments[2])?;
                    let destination = self.temporary(*span)?;
                    self.operations.push(Operation::Error {
                        kind,
                        message,
                        data,
                        destination,
                        span: *span,
                    });
                    return Ok(destination);
                }
                if matches!(callee.name.as_str(), "assert" | "std::test::assert") {
                    return self.lower_assert_builtin(arguments, *span);
                }
                if matches!(callee.name.as_str(), "assert_eq" | "std::test::assert_eq") {
                    return self.lower_assert_eq_builtin(arguments, *span);
                }
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
                if let Some(variant) = self.types.enum_variant(&callee.name) {
                    if variant.fields.len() != arguments.len() {
                        return Err(diagnostics(CompileDiagnostic::new(
                            "E0208",
                            *span,
                            format!(
                                "enum variant `{}` expects {} arguments but received {}",
                                variant.name,
                                variant.fields.len(),
                                arguments.len()
                            ),
                        )));
                    }
                    let mut fields = Vec::with_capacity(arguments.len());
                    for (argument, contract) in arguments.iter().zip(&variant.fields) {
                        let field = self.lower_expression(argument)?;
                        self.operations.push(Operation::ValidateSlot {
                            slot: field,
                            contract: contract.clone(),
                            span: expression_span(argument),
                        });
                        fields.push(field);
                    }
                    let destination = self.temporary(*span)?;
                    let type_identity_slot = self.temporary(*span)?;
                    let variant_slot = self.temporary(*span)?;
                    self.operations.push(Operation::Enum {
                        type_id: variant.type_id,
                        type_identity: variant.type_identity.clone(),
                        variant: variant.name.clone(),
                        fields,
                        type_identity_slot,
                        variant_slot,
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
                if !signature.accepts_arity(arguments.len()) {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0208",
                        *span,
                        format!(
                            "function `{}` expects {} arguments but received {}",
                            callee.name,
                            signature.expected_arity_description(),
                            arguments.len()
                        ),
                    )));
                }
                let mut slots = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    slots.push(self.lower_expression(argument)?);
                }
                self.pack_variadic_tail(&mut slots, &signature, *span)?;
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
                if type_name.name == "Bytes"
                    && matches!(method.name.as_str(), "from_list" | "from_utf8")
                {
                    if arguments.len() != 1 {
                        return Err(diagnostics(CompileDiagnostic::new(
                            "E0208",
                            *span,
                            format!(
                                "static method `Bytes::{}` expects 1 argument but received {}",
                                method.name,
                                arguments.len()
                            ),
                        )));
                    }
                    let value = self.lower_expression(&arguments[0])?;
                    let destination = self.temporary(*span)?;
                    self.operations.push(Operation::BytesStatic {
                        value,
                        from_utf8: method.name == "from_utf8",
                        destination,
                        span: *span,
                    });
                    return Ok(destination);
                }
                let key = format!("{}::{}", type_name.name, method.name);
                if let Some(variant) = self.types.enum_variant(&key) {
                    if variant.fields.len() != arguments.len() {
                        return Err(diagnostics(CompileDiagnostic::new(
                            "E0208",
                            *span,
                            format!(
                                "enum variant `{}` expects {} arguments but received {}",
                                variant.name,
                                variant.fields.len(),
                                arguments.len()
                            ),
                        )));
                    }
                    let mut fields = Vec::with_capacity(arguments.len());
                    for (argument, contract) in arguments.iter().zip(&variant.fields) {
                        let field = self.lower_expression(argument)?;
                        self.operations.push(Operation::ValidateSlot {
                            slot: field,
                            contract: contract.clone(),
                            span: expression_span(argument),
                        });
                        fields.push(field);
                    }
                    let destination = self.temporary(*span)?;
                    let type_identity_slot = self.temporary(*span)?;
                    let variant_slot = self.temporary(*span)?;
                    self.operations.push(Operation::Enum {
                        type_id: variant.type_id,
                        type_identity: variant.type_identity.clone(),
                        variant: variant.name.clone(),
                        fields,
                        type_identity_slot,
                        variant_slot,
                        destination,
                        span: *span,
                    });
                    return Ok(destination);
                }
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
                if !signature.accepts_arity(arguments.len()) {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0208",
                        *span,
                        format!(
                            "static method `{}::{}` expects {} arguments but received {}",
                            type_name.name,
                            method.name,
                            signature.expected_arity_description(),
                            arguments.len()
                        ),
                    )));
                }
                let mut slots = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    slots.push(self.lower_expression(argument)?);
                }
                self.pack_variadic_tail(&mut slots, &signature, *span)?;
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
            Expression::Match { value, arms, span } => self.lower_match(value, arms, *span),
        }
    }

    /// Lowers the fatal `assert(condition[, description])` standard intrinsic.
    fn lower_assert_builtin(
        &mut self,
        arguments: &'function [Expression<'source>],
        span: SourceSpan<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        if !(1..=2).contains(&arguments.len()) {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0208",
                span,
                format!(
                    "assert expects 1 or 2 arguments but received {}",
                    arguments.len()
                ),
            )));
        }
        let first = self.lower_expression(&arguments[0])?;
        let description = if let Some(description) = arguments.get(1) {
            self.lower_expression(description)?
        } else {
            self.lower_default_assertion_description(
                crate::codegen::standard::ASSERT_DEFAULT_DESCRIPTION,
                span,
            )?
        };
        let destination = self.temporary(span)?;
        self.operations.push(Operation::Assert {
            condition: first,
            actual: None,
            expected: None,
            description,
            destination,
            span,
        });
        Ok(destination)
    }

    /// Lowers the fatal `assert_eq(actual, expected[, description])` standard intrinsic.
    fn lower_assert_eq_builtin(
        &mut self,
        arguments: &'function [Expression<'source>],
        span: SourceSpan<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        if !(2..=3).contains(&arguments.len()) {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0208",
                span,
                format!(
                    "assert_eq expects 2 or 3 arguments but received {}",
                    arguments.len()
                ),
            )));
        }
        let first = self.lower_expression(&arguments[0])?;
        let second = self.lower_expression(&arguments[1])?;
        let operator = TraitOperator::from_binary(BinaryOperator::Equal).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "missing standard equality operator",
            ))
        })?;
        let comparison = self.temporary(span)?;
        let result = self.temporary(span)?;
        self.operations.push(Operation::Operator {
            operator,
            left: first,
            right: second,
            targets: self.methods.operator(operator).to_vec(),
            destination: result,
            span,
        });
        self.operations.push(Operation::OrderingTest {
            value: result,
            test: 0,
            destination: comparison,
            span,
        });
        let description = if let Some(description) = arguments.get(2) {
            self.lower_expression(description)?
        } else {
            self.lower_default_assertion_description(
                crate::codegen::standard::ASSERT_EQ_DEFAULT_DESCRIPTION,
                span,
            )?
        };
        let destination = self.temporary(span)?;
        self.operations.push(Operation::Assert {
            condition: comparison,
            actual: Some(first),
            expected: Some(second),
            description,
            destination,
            span,
        });
        Ok(destination)
    }

    /// Emits a compiler-provided String when an assertion omits its description.
    fn lower_default_assertion_description(
        &mut self,
        value: &'static str,
        span: SourceSpan<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        let destination = self.temporary(span)?;
        self.operations.push(Operation::String {
            value,
            destination,
            span,
        });
        Ok(destination)
    }

    /// Lowers one enum-pattern match through explicit branches and durable arm bindings.
    fn lower_match(
        &mut self,
        value: &'function Expression<'source>,
        arms: &'function [crate::ast::MatchArm<'source>],
        span: SourceSpan<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        let value = self.lower_expression(value)?;
        let destination = self.temporary(span)?;
        let mut exits = Vec::new();
        let mut matched = std::collections::HashSet::new();
        let mut enum_identity = None;
        let mut enum_type_name = None;
        let mut fallback = false;
        for arm in arms {
            if fallback {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0231",
                    arm.span,
                    "match arm follows wildcard fallback",
                )));
            }
            match &arm.pattern {
                MatchPattern::Wildcard(_) => {
                    fallback = true;
                    let arm_value = self.lower_match_arm(value, arm, &[])?;
                    self.operations.push(Operation::Copy {
                        source: arm_value,
                        destination,
                        span: arm.span,
                    });
                    exits.push(self.push(Operation::Goto {
                        target: 0,
                        checkpoint: false,
                        span: arm.span,
                    })?);
                }
                MatchPattern::Variant {
                    type_name,
                    variant,
                    bindings,
                    span: pattern_span,
                } => {
                    let key = format!("{}::{}", type_name.name, variant.name);
                    let variant_data = self.types.enum_variant(&key).cloned().ok_or_else(|| {
                        diagnostics(CompileDiagnostic::new(
                            "E0216",
                            *pattern_span,
                            format!("unknown enum variant `{key}`"),
                        ))
                    })?;
                    if bindings.len() != variant_data.fields.len() {
                        return Err(diagnostics(CompileDiagnostic::new(
                            "E0208",
                            *pattern_span,
                            format!(
                                "enum pattern `{key}` expects {} bindings",
                                variant_data.fields.len()
                            ),
                        )));
                    }
                    if !matched.insert(key) {
                        return Err(diagnostics(CompileDiagnostic::new(
                            "E0231",
                            *pattern_span,
                            "duplicate match variant arm",
                        )));
                    }
                    if let Some(identity) = &enum_identity {
                        if identity != &variant_data.type_identity {
                            return Err(diagnostics(CompileDiagnostic::new(
                                "E0231",
                                *pattern_span,
                                "match arms must belong to one enum",
                            )));
                        }
                    } else {
                        enum_identity = Some(variant_data.type_identity.clone());
                        enum_type_name = Some(type_name.name.clone());
                    }
                    let condition = self.temporary(*pattern_span)?;
                    let type_identity_slot = self.temporary(*pattern_span)?;
                    let variant_slot = self.temporary(*pattern_span)?;
                    self.operations.push(Operation::EnumMatches {
                        value,
                        type_identity: variant_data.type_identity,
                        variant: variant_data.name,
                        type_identity_slot,
                        variant_slot,
                        destination: condition,
                        span: *pattern_span,
                    });
                    let checked = self.temporary(*pattern_span)?;
                    let branch = self.push(Operation::Branch {
                        condition,
                        checked,
                        when_true: 0,
                        when_false: 0,
                        span: *pattern_span,
                    })?;
                    let arm_start = self.operations.len();
                    let arm_value = self.lower_match_arm(value, arm, bindings)?;
                    self.operations.push(Operation::Copy {
                        source: arm_value,
                        destination,
                        span: arm.span,
                    });
                    exits.push(self.push(Operation::Goto {
                        target: 0,
                        checkpoint: false,
                        span: arm.span,
                    })?);
                    let next_arm = self.operations.len();
                    self.set_branch_targets(branch, arm_start, next_arm, *pattern_span)?;
                }
            }
        }
        if !fallback {
            let Some(type_name) = enum_type_name else {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0232",
                    span,
                    "match requires at least one enum variant or a wildcard arm",
                )));
            };
            let missing = self
                .types
                .enum_variant_names(&type_name)
                .into_iter()
                .find(|variant| !matched.contains(&format!("{type_name}::{variant}")));
            if let Some(variant) = missing {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0232",
                    span,
                    format!("non-exhaustive match: missing `{type_name}::{variant}`"),
                )));
            }
            self.operations.push(Operation::MatchError {
                value,
                destination,
                span,
            });
        }
        let after = self.operations.len();
        for exit in exits {
            self.set_goto_target(exit, after, span)?;
        }
        Ok(destination)
    }

    /// Creates one lexical scope for arm payload bindings before lowering its expression.
    fn lower_match_arm(
        &mut self,
        value: u32,
        arm: &'function crate::ast::MatchArm<'source>,
        bindings: &'function [crate::ast::Identifier<'source>],
    ) -> Result<u32, CompileDiagnostics<'source>> {
        self.scopes.push(HashMap::new());
        let mut declared = std::collections::HashSet::new();
        for (index, binding) in bindings.iter().enumerate() {
            if !declared.insert(&binding.name) {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0204",
                    binding.span,
                    format!("duplicate match binding `{}`", binding.name),
                )));
            }
            let slot = self.temporary(binding.span)?;
            self.operations.push(Operation::EnumField {
                value,
                index: u32::try_from(index).map_err(|_| {
                    diagnostics(CompileDiagnostic::new(
                        "E0212",
                        binding.span,
                        "too many enum payload bindings",
                    ))
                })?,
                destination: slot,
                span: binding.span,
            });
            let cell = self.captured_names.contains(&binding.name);
            if cell {
                self.operations.push(Operation::CellNew {
                    value: slot,
                    destination: slot,
                    span: binding.span,
                });
            }
            let Some(scope) = self.scopes.last_mut() else {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0999",
                    binding.span,
                    "missing match binding scope",
                )));
            };
            scope.insert(binding.name.clone(), BindingSlot { slot, cell });
        }
        let result = match &arm.body {
            MatchArmBody::Expression(value) => self.lower_expression(value)?,
            MatchArmBody::Block(block) => {
                self.lower_block(block)?;
                let destination = self.temporary(block.span)?;
                self.operations.push(Operation::None {
                    destination,
                    span: block.span,
                });
                destination
            }
        };
        let _scope = self.scopes.pop();
        Ok(result)
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
        if nominal.kind != NominalKind::Object {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0216",
                type_name.span,
                format!("enum `{}` requires a variant constructor", type_name.name),
            )));
        }
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
                        enum_type_ids: Vec::new(),
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

    /// Lowers Host::stream through the internal open ABI without introducing a source helper.
    fn lower_host_stream(
        &mut self,
        arguments: &'function [Expression<'source>],
        span: SourceSpan<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        let nominal = self.types.get("HostStream").cloned().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "missing compiler-owned HostStream type",
            ))
        })?;
        let handle_contract = nominal
            .fields
            .iter()
            .find(|field| field.name == "handle")
            .map(|field| field.contract.clone())
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0999",
                    span,
                    "missing compiler-owned HostStream handle field",
                ))
            })?;
        let name = self.temporary(span)?;
        self.operations.push(Operation::String {
            value: exs_abi::HOST_STREAM_OPEN_HOST_NAME,
            destination: name,
            span,
        });
        let mut slots = Vec::with_capacity(arguments.len());
        for argument in arguments {
            slots.push(self.lower_expression(argument)?);
        }
        let argument_list = self.temporary(span)?;
        let handle = self.temporary(span)?;
        self.operations.push(Operation::HostCall {
            name,
            arguments: slots,
            argument_list,
            destination: handle,
            span,
        });
        self.operations.push(Operation::HostResume {
            destination: handle,
            span,
        });
        let destination = self.temporary(span)?;
        self.operations.push(Operation::HostStream {
            handle,
            type_id: nominal.id,
            handle_contract,
            destination,
            span,
        });
        Ok(destination)
    }

    /// Lowers one built-in Host time operation and converts its raw object into a nominal value.
    fn lower_host_time(
        &mut self,
        operation: HostTimeOperation,
        arguments: &'function [Expression<'source>],
        span: SourceSpan<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        let (host_name, type_name) = match operation {
            HostTimeOperation::Now => (exs_abi::HOST_NOW_HOST_NAME, "DateTime"),
            HostTimeOperation::Elapsed => (exs_abi::HOST_ELAPSED_HOST_NAME, "Duration"),
            HostTimeOperation::InTimezone => {
                (exs_abi::HOST_DATETIME_IN_TIMEZONE_HOST_NAME, "DateTime")
            }
            HostTimeOperation::FromComponents => {
                (exs_abi::HOST_DATETIME_FROM_COMPONENTS_HOST_NAME, "DateTime")
            }
        };
        let nominal = self.types.get(type_name).cloned().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                format!("missing compiler-owned {type_name} type"),
            ))
        })?;
        let mut fields = Vec::with_capacity(nominal.fields.len());
        for field in nominal.fields {
            fields.push(HostTimeField {
                name: field.name,
                contract: field.contract,
                slot: self.temporary(span)?,
            });
        }
        let name = self.temporary(span)?;
        self.operations.push(Operation::String {
            value: host_name,
            destination: name,
            span,
        });
        let argument_list = self.temporary(span)?;
        let mut slots = Vec::with_capacity(arguments.len());
        for argument in arguments {
            slots.push(self.lower_expression(argument)?);
        }
        let value = self.temporary(span)?;
        self.operations.push(Operation::HostCall {
            name,
            arguments: slots,
            argument_list,
            destination: value,
            span,
        });
        self.operations.push(Operation::HostResume {
            destination: value,
            span,
        });
        let destination = self.temporary(span)?;
        self.operations.push(Operation::HostTime {
            value,
            type_id: nominal.id,
            fields,
            destination,
            span,
        });
        Ok(destination)
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
