//! Statement and loop lowering for one ExS function.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Instruction};

use crate::ast::{AssignmentTarget, Block, Expression, Statement};
use crate::codegen::diagnostics;
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
            Statement::Let { name, value, .. } => {
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
                self.store_stack_value_in(local)?;
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
                    self.runtime_value_call("__exs_rt_index_set", 3, *span)?;
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
                    self.runtime_value_call("__exs_rt_index_set", 3, *span)?;
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
                    self.runtime_call("__exs_rt_none_new", *span)?;
                }
                self.validate_return_type(*span)?;
                self.return_stack_value()?;
            }
            Statement::Expression { expression, .. } => {
                self.compile_expression(expression)?;
                self.function.instruction(&Instruction::Drop);
            }
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
                iterable,
                body,
                span,
            } => self.compile_for(binding, iterable, body, *span)?,
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

    /// Lowers a for loop through a runtime iterable snapshot and indexed Wasm loop.
    pub(super) fn compile_for(
        &mut self,
        binding: &crate::ast::Identifier<'a>,
        iterable: &Expression<'a>,
        body: &Block<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(iterable)?;
        let iterable = self.store_stack_value()?;
        self.function.instruction(&Instruction::LocalGet(iterable));
        self.runtime_value_call("__exs_rt_iter_snapshot", 1, span)?;
        self.return_if_error(span)?;
        let snapshot = self.store_stack_value()?;
        self.clear_root_slot(iterable)?;

        self.function.instruction(&Instruction::I64Const(0));
        self.runtime_call("__exs_rt_int_new", span)?;
        let index = self.store_stack_value()?;
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
        self.runtime_value_call("__exs_rt_length", 1, span)?;
        let length = self.store_stack_value()?;
        self.function.instruction(&Instruction::LocalGet(index));
        self.function.instruction(&Instruction::LocalGet(length));
        self.runtime_value_call("__exs_rt_lt", 2, span)?;
        let condition = self.allocate_local();
        self.function.instruction(&Instruction::LocalSet(condition));
        self.clear_root_slot(length)?;
        self.function.instruction(&Instruction::LocalGet(condition));
        self.compile_condition(span)?;
        self.function.instruction(&Instruction::I32Eqz);
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

        self.function.instruction(&Instruction::LocalGet(snapshot));
        self.function.instruction(&Instruction::LocalGet(index));
        self.runtime_value_call("__exs_rt_index_get", 2, span)?;
        self.store_stack_value_in(item)?;
        self.compile_block(body, true)?;
        let _loop = self.loops.pop();

        self.function.instruction(&Instruction::End);
        self.exit_control()?;
        self.function.instruction(&Instruction::LocalGet(index));
        self.function.instruction(&Instruction::I64Const(1));
        self.runtime_call("__exs_rt_int_new", span)?;
        self.runtime_value_call("__exs_rt_add", 2, span)?;
        self.store_stack_value_in(index)?;
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
        self.clear_root_slot(snapshot)?;
        self.clear_root_slot(index)
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
