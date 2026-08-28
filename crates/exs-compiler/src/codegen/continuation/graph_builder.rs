//! Statement, block, branch, and loop lowering for continuation graphs.

use std::collections::HashMap;

use crate::ast::{AssignmentTarget, Expression, Statement};
use crate::codegen::diagnostics;
use crate::codegen::function::FunctionSignature;
use crate::codegen::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

use super::graph::{BindingSlot, GraphBuilder, LoopBuilderContext, Operation, operation_span};

impl<'source, 'function> GraphBuilder<'source, 'function> {
    /// Replaces a variadic source argument tail with one compiler-owned List slot.
    pub(super) fn pack_variadic_tail(
        &mut self,
        arguments: &mut Vec<u32>,
        signature: &FunctionSignature,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        if !signature.variadic {
            return Ok(());
        }
        let elements = arguments.split_off(signature.arity);
        let destination = self.temporary(span)?;
        self.operations.push(Operation::List {
            elements,
            destination,
            span,
        });
        arguments.push(destination);
        Ok(())
    }

    /// Lowers one source statement into contiguous graph states and explicit branch edges.
    pub(super) fn lower_statement(
        &mut self,
        statement: &'function Statement<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        match statement {
            Statement::Let {
                name,
                type_annotation,
                value,
                ..
            } => {
                if self
                    .scopes
                    .last()
                    .is_some_and(|scope| scope.contains_key(&name.name))
                {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0204",
                        name.span,
                        format!("duplicate binding `{}`", name.name),
                    )));
                }
                let value = self.lower_expression(value)?;
                let binding = self.temporary(name.span)?;
                if let Some(annotation) = type_annotation {
                    let contract = self.types.resolve(Some(annotation), name.span)?;
                    self.operations.push(Operation::WireDecode {
                        source: value,
                        destination: binding,
                        schema: self.types.wire_schema(&contract),
                        span: name.span,
                    });
                } else {
                    self.operations.push(Operation::Copy {
                        source: value,
                        destination: binding,
                        span: name.span,
                    });
                }
                let cell = self.captured_names.contains(&name.name);
                if cell {
                    self.operations.push(Operation::CellNew {
                        value: binding,
                        destination: binding,
                        span: name.span,
                    });
                }
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(
                        name.name.clone(),
                        BindingSlot {
                            slot: binding,
                            cell,
                        },
                    );
                }
            }
            Statement::Assign { target, value, .. } => match target {
                AssignmentTarget::Variable(name) => {
                    let destination = self.lookup(&name.name, name.span)?;
                    let value = self.lower_expression(value)?;
                    if destination.cell {
                        self.operations.push(Operation::CellSet {
                            cell: destination.slot,
                            value,
                            span: name.span,
                        });
                    } else {
                        self.operations.push(Operation::Copy {
                            source: value,
                            destination: destination.slot,
                            span: name.span,
                        });
                    }
                }
                AssignmentTarget::Index {
                    receiver,
                    index,
                    span,
                } => {
                    let receiver = self.lower_expression(receiver)?;
                    let index = self.lower_expression(index)?;
                    let value = self.lower_expression(value)?;
                    self.operations.push(Operation::IndexSet {
                        receiver,
                        index,
                        value,
                        span: *span,
                    });
                }
                AssignmentTarget::Property {
                    receiver,
                    property,
                    span,
                } => {
                    let receiver = self.lower_expression(receiver)?;
                    let value = self.lower_expression(value)?;
                    self.operations.push(Operation::PropertySet {
                        receiver,
                        property: property.name.clone(),
                        property_span: property.span,
                        value,
                        span: *span,
                    });
                }
            },
            Statement::Return { value, span } => {
                let value = match value {
                    Some(value) => self.lower_expression(value)?,
                    None => {
                        let destination = self.temporary(*span)?;
                        self.operations.push(Operation::None {
                            destination,
                            span: *span,
                        });
                        destination
                    }
                };
                self.operations
                    .push(Operation::Return { value, span: *span });
            }
            Statement::Expression { expression, .. } => {
                let _value = self.lower_expression(expression)?;
            }
            Statement::Block { block, .. } => self.lower_block(block)?,
            Statement::If {
                condition,
                then_block,
                else_branch,
                span,
            } => self.lower_if(condition, then_block, else_branch.as_ref(), *span)?,
            Statement::While {
                condition,
                body,
                span,
            } => self.lower_while(condition, body, *span)?,
            Statement::For {
                binding,
                type_annotation,
                iterable,
                body,
                span,
            } => self.lower_for(binding, type_annotation.as_ref(), iterable, body, *span)?,
            Statement::Break { span } => self.lower_loop_branch(*span, true)?,
            Statement::Continue { span } => self.lower_loop_branch(*span, false)?,
        }
        Ok(())
    }

    /// Lowers a conditional statement using true and false state targets.
    pub(super) fn lower_if(
        &mut self,
        condition: &'function Expression<'source>,
        then_block: &'function crate::ast::Block<'source>,
        else_branch: Option<&'function crate::ast::ElseBranch<'source>>,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let condition = self.lower_expression(condition)?;
        let checked = self.temporary(span)?;
        let branch = self.push(Operation::Branch {
            condition,
            checked,
            when_true: 0,
            when_false: 0,
            span,
        })?;
        let then_start = self.operations.len();
        self.lower_block(then_block)?;
        let skip_else = self.push(Operation::Goto {
            target: 0,
            checkpoint: false,
            span,
        })?;
        let else_start = self.operations.len();
        if let Some(else_branch) = else_branch {
            match else_branch {
                crate::ast::ElseBranch::Block(block) => self.lower_block(block)?,
                crate::ast::ElseBranch::If(statement) => self.lower_statement(statement)?,
            }
        }
        let after = self.operations.len();
        self.set_branch_targets(branch, then_start, else_start, span)?;
        self.set_goto_target(skip_else, after, span)
    }

    /// Lowers a while loop with explicit break and continue branch targets.
    pub(super) fn lower_while(
        &mut self,
        condition: &'function Expression<'source>,
        body: &'function crate::ast::Block<'source>,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let condition_start = self.operations.len();
        let condition = self.lower_expression(condition)?;
        let checked = self.temporary(span)?;
        let branch = self.push(Operation::Branch {
            condition,
            checked,
            when_true: 0,
            when_false: 0,
            span,
        })?;
        let body_start = self.operations.len();
        self.loops.push(LoopBuilderContext {
            continues: Vec::new(),
            breaks: Vec::new(),
        });
        self.lower_block(body)?;
        let back_edge = self.push(Operation::Goto {
            target: self.state_id(condition_start, span)?,
            checkpoint: true,
            span,
        })?;
        let exit = self.operations.len();
        self.set_branch_targets(branch, body_start, exit, span)?;
        let context = self.loops.pop().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "missing active continuation loop",
            ))
        })?;
        for branch in context.breaks {
            self.set_goto_target(branch, exit, span)?;
        }
        for branch in context.continues {
            self.set_goto_target(branch, condition_start, span)?;
        }
        let _back_edge = back_edge;
        Ok(())
    }

    /// Lowers a for loop through one async-capable IteratorStep at a time.
    pub(super) fn lower_for(
        &mut self,
        binding: &'function crate::ast::Identifier<'source>,
        type_annotation: Option<&'function crate::ast::TypeAnnotation<'source>>,
        iterable: &'function Expression<'source>,
        body: &'function crate::ast::Block<'source>,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let iterable = self.lower_expression(iterable)?;
        let snapshot = self.temporary(span)?;
        self.operations.push(Operation::IterSnapshot {
            iterable,
            destination: snapshot,
            span,
        });
        let condition_start = self.operations.len();
        let step = self.temporary(span)?;
        self.operations.push(Operation::InstanceCall {
            receiver: snapshot,
            method: "next",
            method_span: span,
            arguments: Vec::new(),
            targets: self
                .methods
                .trait_instance("Iterator", "next")
                .map_or_else(Vec::new, ToOwned::to_owned),
            destination: step,
            span,
        });
        let item = self.temporary(binding.span)?;
        let checked = self.temporary(span)?;
        let type_identity_slot = self.temporary(span)?;
        let done_variant_slot = self.temporary(span)?;
        let item_variant_slot = self.temporary(span)?;
        let done = self
            .types
            .enum_variant("IteratorStep::Done")
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0999",
                    span,
                    "missing prelude IteratorStep::Done variant",
                ))
            })?;
        let item_variant = self
            .types
            .enum_variant("IteratorStep::Item")
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0999",
                    span,
                    "missing prelude IteratorStep::Item variant",
                ))
            })?;
        let branch = self.push(Operation::IteratorBranch {
            step,
            item,
            checked,
            type_identity_slot,
            done_variant_slot,
            item_variant_slot,
            type_identity: done.type_identity.clone(),
            done_variant: done.name.clone(),
            item_variant: item_variant.name.clone(),
            when_item: 0,
            when_done: 0,
            span,
        })?;
        let body_start = self.operations.len();
        if let Some(annotation) = type_annotation {
            let contract = self.types.resolve(Some(annotation), binding.span)?;
            self.operations.push(Operation::WireDecode {
                source: item,
                destination: item,
                schema: self.types.wire_schema(&contract),
                span: binding.span,
            });
        }
        let cell = self.captured_names.contains(&binding.name);
        if cell {
            self.operations.push(Operation::CellNew {
                value: item,
                destination: item,
                span: binding.span,
            });
        }
        self.scopes.push(HashMap::from([(
            binding.name.clone(),
            BindingSlot { slot: item, cell },
        )]));
        self.loops.push(LoopBuilderContext {
            continues: Vec::new(),
            breaks: Vec::new(),
        });
        for statement in &body.statements {
            self.lower_statement(statement)?;
        }
        let _scope = self.scopes.pop();
        let increment = self.operations.len();
        self.operations.push(Operation::Goto {
            target: self.state_id(condition_start, span)?,
            checkpoint: true,
            span,
        });
        let exit = self.operations.len();
        self.set_iterator_targets(branch, body_start, exit, span)?;
        let context = self.loops.pop().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "missing active continuation loop",
            ))
        })?;
        for branch in context.breaks {
            self.set_goto_target(branch, exit, span)?;
        }
        for branch in context.continues {
            self.set_goto_target(branch, increment, span)?;
        }
        Ok(())
    }

    /// Lowers break or continue into a branch patched by the enclosing loop.
    pub(super) fn lower_loop_branch(
        &mut self,
        span: SourceSpan<'source>,
        is_break: bool,
    ) -> Result<(), CompileDiagnostics<'source>> {
        if self.loops.is_empty() {
            let keyword = if is_break { "break" } else { "continue" };
            return Err(diagnostics(CompileDiagnostic::new(
                "E0213",
                span,
                format!("{keyword} is only valid inside a loop"),
            )));
        }
        let branch = self.push(Operation::Goto {
            target: 0,
            checkpoint: !is_break,
            span,
        })?;
        let Some(target) = self.loops.last_mut() else {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "missing active continuation loop",
            )));
        };
        if is_break {
            target.breaks.push(branch);
        } else {
            target.continues.push(branch);
        }
        Ok(())
    }

    /// Lowers one lexical block and drops its name bindings after its final state.
    pub(super) fn lower_block(
        &mut self,
        block: &'function crate::ast::Block<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.scopes.push(HashMap::new());
        for statement in &block.statements {
            self.lower_statement(statement)?;
        }
        let _scope = self.scopes.pop();
        Ok(())
    }

    /// Appends one operation and returns its zero-based graph state index.
    pub(super) fn push(
        &mut self,
        operation: Operation<'source, 'function>,
    ) -> Result<usize, CompileDiagnostics<'source>> {
        let state = self.operations.len();
        let _state = self.state_id(state, operation_span(&operation))?;
        self.operations.push(operation);
        Ok(state)
    }

    /// Converts a graph index to the Wasm i32 state domain.
    pub(super) fn state_id(
        &self,
        state: usize,
        span: SourceSpan<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        u32::try_from(state).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                span,
                "too many continuation states for one function",
            ))
        })
    }

    /// Patches one conditional branch after both block starts are known.
    pub(super) fn set_branch_targets(
        &mut self,
        state: usize,
        when_true: usize,
        when_false: usize,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let when_true = self.state_id(when_true, span)?;
        let when_false = self.state_id(when_false, span)?;
        let Some(Operation::Branch {
            when_true: target_true,
            when_false: target_false,
            ..
        }) = self.operations.get_mut(state)
        else {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "missing continuation conditional branch",
            )));
        };
        *target_true = when_true;
        *target_false = when_false;
        Ok(())
    }

    /// Patches one IteratorStep branch after its body and exit starts are known.
    pub(super) fn set_iterator_targets(
        &mut self,
        state: usize,
        when_true: usize,
        when_false: usize,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let when_true = self.state_id(when_true, span)?;
        let when_false = self.state_id(when_false, span)?;
        let Some(Operation::IteratorBranch {
            when_item: target_true,
            when_done: target_false,
            ..
        }) = self.operations.get_mut(state)
        else {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "missing continuation IteratorStep branch",
            )));
        };
        *target_true = when_true;
        *target_false = when_false;
        Ok(())
    }

    /// Patches one explicit graph jump target.
    pub(super) fn set_goto_target(
        &mut self,
        state: usize,
        target: usize,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let target = self.state_id(target, span)?;
        let Some(Operation::Goto {
            target: destination,
            ..
        }) = self.operations.get_mut(state)
        else {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "missing continuation graph jump",
            )));
        };
        *destination = target;
        Ok(())
    }
}
