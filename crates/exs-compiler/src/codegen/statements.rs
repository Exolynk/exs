//! Statement lowering.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Instruction};

use super::errors::ForBinding;
use super::expressions::expression_span;
use super::function::{FunctionCompiler, error, unsupported};
use crate::ast::{AssignmentTarget, Block, ElseBranch, Expression, Statement};
use crate::codegen::immediate;
use crate::codegen::standard::is_unshared_range;
use crate::diagnostic::CompileDiagnostics;

impl<'a> FunctionCompiler<'a, '_> {
    /// Compiles every statement of `block` in a new scope.
    pub(super) fn compile_block(
        &mut self,
        block: &Block<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.scopes.push(HashMap::new());
        for statement in &block.statements {
            self.compile_statement(statement)?;
        }
        self.scopes.pop();
        Ok(())
    }

    /// Compiles one statement.
    pub(super) fn compile_statement(
        &mut self,
        statement: &Statement<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        match statement {
            Statement::Let {
                name,
                type_annotation,
                value,
                span,
            } => {
                self.compile_expression(value)?;
                let local = self.store_new();
                if type_annotation.is_some() {
                    let contract = self
                        .context
                        .types
                        .resolve(type_annotation.as_ref(), *span)?;
                    self.check_binding(local, &contract, *span)?;
                }
                self.define(&name.name, local)?;
            }
            Statement::Assign {
                target: AssignmentTarget::Variable(identifier),
                value,
                span,
            } => {
                let local = self
                    .lookup(&identifier.name)
                    .ok_or_else(|| unsupported(*span, "assignments to unknown bindings"))?;
                self.compile_expression(value)?;
                self.assign_binding(&identifier.name, local);
            }
            // Like v1, assignments report the span of their target.
            Statement::Assign {
                target:
                    AssignmentTarget::Index {
                        receiver,
                        index,
                        span,
                    },
                value,
                ..
            } => self.compile_index_set(receiver, index, value, *span)?,
            Statement::Assign {
                target:
                    AssignmentTarget::Property {
                        receiver,
                        property,
                        span,
                    },
                value,
                ..
            } => self.compile_property_set(receiver, &property.name, value, *span)?,
            Statement::Return { value, span } => {
                match value {
                    Some(value) => self.compile_expression(value)?,
                    None => immediate::push(&mut self.function, exs_layout::ValueRef::NONE),
                }
                self.return_value(*span)?;
            }
            Statement::Block { block, .. } => self.compile_block(block)?,
            Statement::If {
                condition,
                then_block,
                else_branch,
                ..
            } => {
                self.compile_expression(condition)?;
                self.condition(expression_span(condition))?;
                self.instruction(Instruction::If(BlockType::Empty));
                self.compile_block(then_block)?;
                if let Some(else_branch) = else_branch {
                    self.instruction(Instruction::Else);
                    match else_branch {
                        ElseBranch::Block(block) => self.compile_block(block)?,
                        ElseBranch::If(statement) => self.compile_statement(statement)?,
                    }
                }
                self.instruction(Instruction::End);
            }
            Statement::While {
                condition, body, ..
            } => {
                self.instruction(Instruction::Block(BlockType::Empty));
                let break_depth = self.depth;
                self.instruction(Instruction::Loop(BlockType::Empty));
                let continue_depth = self.depth;
                self.compile_expression(condition)?;
                self.condition(expression_span(condition))?;
                self.instruction(Instruction::I32Eqz);
                self.instruction(Instruction::BrIf(self.depth - break_depth));
                self.loops.push((break_depth, continue_depth));
                self.compile_block(body)?;
                self.loops.pop();
                self.branch_to(continue_depth);
                self.instruction(Instruction::End);
                self.instruction(Instruction::End);
            }
            Statement::For {
                binding,
                type_annotation,
                iterable,
                body,
                span,
            } => {
                let contract = match type_annotation {
                    Some(annotation) => {
                        Some(self.context.types.resolve(Some(annotation), binding.span)?)
                    }
                    None => None,
                };
                let binding = ForBinding {
                    identifier: binding,
                    contract: contract.as_ref(),
                };
                if is_unshared_range(iterable) {
                    self.compile_range_for(binding, iterable, body, *span)?;
                } else if matches!(iterable, Expression::Variable(_))
                    && self.unobserved_ranges.contains(span)
                {
                    self.compile_range_variable_for(binding, iterable, body, *span)?;
                } else {
                    self.compile_iterable_for(binding, iterable, body, *span)?;
                }
            }
            Statement::Break { span } | Statement::Continue { span } => {
                let Some((break_depth, continue_depth)) = self.loops.last().copied() else {
                    let keyword = if matches!(statement, Statement::Break { .. }) {
                        "break"
                    } else {
                        "continue"
                    };
                    return Err(error(
                        "E0213",
                        *span,
                        format!("{keyword} is only valid inside a loop"),
                    ));
                };
                let target = if matches!(statement, Statement::Break { .. }) {
                    break_depth
                } else {
                    continue_depth
                };
                self.branch_to(target);
            }
            Statement::Expression { expression, .. } => {
                self.compile_expression(expression)?;
                self.instruction(Instruction::Drop);
            }
        }
        Ok(())
    }
}
