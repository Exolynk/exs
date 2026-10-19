//! Statement and loop lowering for one ExS function.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Instruction};

use crate::ast::{AssignmentTarget, Block, Expression, Statement};
use crate::codegen::diagnostics;
use crate::codegen::standard::is_unshared_range;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

use super::analysis::condition_span;
use super::{FunctionCompiler, LoopContext};

impl<'a, 'module> FunctionCompiler<'a, 'module> {
    /// Compiles statements in one lexical block.
    pub(super) fn compile_block(
        &mut self,
        block: &Block<'a>,
        new_scope: bool,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if new_scope {
            self.scopes.push(HashMap::new());
        }
        for statement in &block.statements {
            self.compile_statement(statement)?;
        }
        if new_scope && let Some(scope) = self.scopes.pop() {
            for local in scope.into_values() {
                self.clear_root_slot(local)?;
            }
        }
        Ok(())
    }

    /// Compiles one source statement.
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
                self.compile_expression(value)?;
                let local = self.allocate_local();
                if let Some(annotation) = type_annotation {
                    let contract = self.types.resolve(Some(annotation), name.span)?;
                    let schema = self.types.wire_schema(&contract);
                    self.store_stack_value_in(local)?;
                    self.function.instruction(&Instruction::LocalGet(local));
                    self.compile_string(&schema, *span)?;
                    self.runtime_call("__exs_rt_decode_wire", *span)?;
                    self.return_if_error(*span)?;
                    self.store_stack_value_in(local)?;
                } else {
                    self.store_stack_value_in(local)?;
                }
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(name.name.clone(), local);
                }
            }
            Statement::Assign { target, value, .. } => match target {
                AssignmentTarget::Variable(name) => {
                    let local = self.lookup(&name.name).ok_or_else(|| {
                        diagnostics(CompileDiagnostic::new(
                            "E0205",
                            name.span,
                            format!("unknown binding `{}`", name.name),
                        ))
                    })?;
                    self.compile_expression(value)?;
                    self.store_stack_value_in(local)?;
                }
                AssignmentTarget::Index {
                    receiver,
                    index,
                    span,
                } => {
                    self.compile_expression(receiver)?;
                    let receiver = self.store_stack_value()?;
                    self.compile_expression(index)?;
                    let index = self.store_stack_value()?;
                    self.compile_expression(value)?;
                    let value = self.store_stack_value()?;
                    self.function.instruction(&Instruction::LocalGet(receiver));
                    self.function.instruction(&Instruction::LocalGet(index));
                    self.function.instruction(&Instruction::LocalGet(value));
                    self.runtime_call("__exs_rt_index_set", *span)?;
                    self.function.instruction(&Instruction::Drop);
                    self.clear_root_slot(receiver)?;
                    self.clear_root_slot(index)?;
                    self.clear_root_slot(value)?;
                }
                AssignmentTarget::Property {
                    receiver,
                    property,
                    span,
                } => {
                    self.compile_expression(receiver)?;
                    let receiver = self.store_stack_value()?;
                    self.compile_string(&property.name, property.span)?;
                    let property = self.store_stack_value()?;
                    self.compile_expression(value)?;
                    let value = self.store_stack_value()?;
                    self.function.instruction(&Instruction::LocalGet(receiver));
                    self.function.instruction(&Instruction::LocalGet(property));
                    self.function.instruction(&Instruction::LocalGet(value));
                    self.runtime_call("__exs_rt_index_set", *span)?;
                    self.function.instruction(&Instruction::Drop);
                    self.clear_root_slot(receiver)?;
                    self.clear_root_slot(property)?;
                    self.clear_root_slot(value)?;
                }
            },
            Statement::Return { value, span } => {
                if let Some(value) = value {
                    self.compile_expression(value)?;
                } else {
                    self.emit_none();
                }
                self.validate_return_type(*span)?;
                self.return_stack_value()?;
            }
            Statement::Expression { expression, .. } => {
                self.compile_expression(expression)?;
                self.function.instruction(&Instruction::Drop);
            }
            Statement::Block { block, .. } => self.compile_block(block, true)?,
            Statement::If {
                condition,
                then_block,
                else_branch,
                ..
            } => {
                self.compile_expression(condition)?;
                self.compile_condition(condition_span(condition))?;
                self.function
                    .instruction(&Instruction::If(BlockType::Empty));
                self.enter_control()?;
                self.compile_block(then_block, true)?;
                if let Some(else_branch) = else_branch {
                    self.function.instruction(&Instruction::Else);
                    match else_branch {
                        crate::ast::ElseBranch::Block(block) => self.compile_block(block, true)?,
                        crate::ast::ElseBranch::If(statement) => {
                            self.compile_statement(statement)?
                        }
                    }
                }
                self.function.instruction(&Instruction::End);
                self.exit_control()?;
            }
            Statement::While {
                condition,
                body,
                span,
            } => self.compile_while(condition, body, *span)?,
            Statement::For {
                binding,
                type_annotation,
                iterable,
                body,
                span,
            } => self.compile_for(binding, type_annotation.as_ref(), iterable, body, *span)?,
            Statement::Break { span } => self.compile_loop_branch(*span, true)?,
            Statement::Continue { span } => self.compile_loop_branch(*span, false)?,
        }
        Ok(())
    }

    /// Lowers a while loop to one break block and one condition-checking Wasm loop.
    pub(super) fn compile_while(
        &mut self,
        condition: &Expression<'a>,
        body: &Block<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.function
            .instruction(&Instruction::Block(BlockType::Empty));
        self.enter_control()?;
        let break_depth = self.control_depth;
        self.function
            .instruction(&Instruction::Loop(BlockType::Empty));
        self.enter_control()?;
        let continue_depth = self.control_depth;

        self.compile_expression(condition)?;
        self.compile_condition(condition_span(condition))?;
        self.function.instruction(&Instruction::I32Eqz);
        self.branch_if_to(break_depth, span)?;

        self.loops.push(LoopContext {
            break_depth,
            continue_depth,
            cleanup_scope_start: self.scopes.len(),
        });
        self.compile_block(body, true)?;
        let _loop = self.loops.pop();
        self.branch_to(continue_depth, span)?;

        self.function.instruction(&Instruction::End);
        self.exit_control()?;
        self.function.instruction(&Instruction::End);
        self.exit_control()
    }

    /// Lowers a for loop through a runtime iterable snapshot advanced natively per iteration.
    ///
    /// Synchronous lowering cannot dispatch a possibly suspending `Iterator::next`, so receivers
    /// the runtime cannot advance natively produce a NotIterable error.
    pub(super) fn compile_for(
        &mut self,
        binding: &crate::ast::Identifier<'a>,
        type_annotation: Option<&crate::ast::TypeAnnotation<'a>>,
        iterable: &Expression<'a>,
        body: &Block<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let unshared_range = is_unshared_range(iterable);
        let range_type_id = self
            .types
            .get("Range")
            .map_or(-1, |range| range.id.cast_signed());
        self.compile_expression(iterable)?;
        let iterable = self.store_stack_value()?;
        self.function.instruction(&Instruction::LocalGet(iterable));
        if unshared_range {
            self.function
                .instruction(&Instruction::I32Const(range_type_id));
            self.runtime_call("__exs_rt_iter_range_snapshot", span)?;
        } else {
            self.runtime_call("__exs_rt_iter_snapshot", span)?;
        }
        self.return_if_error(span)?;
        let snapshot = self.store_stack_value()?;
        self.clear_root_slot(iterable)?;

        let next = self.allocate_local();
        let item = self.allocate_local();
        let mut iteration_scope = HashMap::new();
        iteration_scope.insert(binding.name.clone(), item);
        self.scopes.push(iteration_scope);

        self.function
            .instruction(&Instruction::Block(BlockType::Empty));
        self.enter_control()?;
        let break_depth = self.control_depth;
        self.function
            .instruction(&Instruction::Loop(BlockType::Empty));
        self.enter_control()?;
        let loop_depth = self.control_depth;

        self.function.instruction(&Instruction::LocalGet(snapshot));
        self.function
            .instruction(&Instruction::I32Const(range_type_id));
        self.runtime_call("__exs_rt_iter_native_next", span)?;
        self.function.instruction(&Instruction::LocalTee(next));
        self.function.instruction(&Instruction::I64Const(-1));
        self.function.instruction(&Instruction::I64Eq);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.enter_control()?;
        self.function.instruction(&Instruction::LocalGet(snapshot));
        self.runtime_call("__exs_rt_iter_unsupported", span)?;
        self.return_if_error(span)?;
        self.function.instruction(&Instruction::Drop);
        self.function.instruction(&Instruction::End);
        self.exit_control()?;
        self.function.instruction(&Instruction::LocalGet(next));
        self.function.instruction(&Instruction::I64Eqz);
        self.branch_if_to(break_depth, span)?;

        self.function
            .instruction(&Instruction::Block(BlockType::Empty));
        self.enter_control()?;
        let continue_depth = self.control_depth;
        self.loops.push(LoopContext {
            break_depth,
            continue_depth,
            cleanup_scope_start: self.scopes.len(),
        });

        self.function.instruction(&Instruction::LocalGet(next));
        self.store_stack_value_in(item)?;
        if let Some(annotation) = type_annotation {
            let contract = self.types.resolve(Some(annotation), binding.span)?;
            let schema = self.types.wire_schema(&contract);
            self.function.instruction(&Instruction::LocalGet(item));
            self.compile_string(&schema, binding.span)?;
            self.runtime_call("__exs_rt_decode_wire", binding.span)?;
            self.return_if_error(binding.span)?;
            self.store_stack_value_in(item)?;
        }
        self.compile_block(body, true)?;
        let _loop = self.loops.pop();

        self.function.instruction(&Instruction::End);
        self.exit_control()?;
        self.branch_to(loop_depth, span)?;

        self.function.instruction(&Instruction::End);
        self.exit_control()?;
        self.function.instruction(&Instruction::End);
        self.exit_control()?;
        let Some(iteration_scope) = self.scopes.pop() else {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "missing for-loop iteration scope",
            )));
        };
        for local in iteration_scope.into_values() {
            self.clear_root_slot(local)?;
        }
        self.clear_root_slot(snapshot)
    }

    /// Emits break or continue after releasing roots local to the current loop body.
    pub(super) fn compile_loop_branch(
        &mut self,
        span: SourceSpan<'a>,
        is_break: bool,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let Some(context) = self.loops.last().copied() else {
            let keyword = if is_break { "break" } else { "continue" };
            return Err(diagnostics(CompileDiagnostic::new(
                "E0213",
                span,
                format!("{keyword} is only valid inside a loop"),
            )));
        };
        self.clear_roots_from_scope(context.cleanup_scope_start)?;
        if is_break {
            self.branch_to(context.break_depth, span)
        } else {
            self.branch_to(context.continue_depth, span)
        }
    }
}
