//! Statement and loop lowering for one ExS function.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Instruction, ValType};

use crate::ast::{AssignmentTarget, Block, Expression, Statement};
use crate::codegen::standard::is_unshared_range;
use crate::codegen::{VALUE_TYPE, diagnostics, immediate};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

use super::analysis::condition_span;
use super::{FunctionCompiler, LoopContext};

/// Wasm locals of a `for` loop counting over a literal Range of immediate Ints.
pub(super) struct InlineRange {
    /// Next candidate item as a plain `i64`.
    cursor: u32,
    /// Range end as a plain `i64`.
    last: u32,
    /// Signed distance between items, 1 or -1.
    step: u32,
    /// 1 once the end itself has been yielded, else 0.
    done: u32,
    /// Whether the end itself is yielded, as for `a..=b`.
    inclusive: bool,
}

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
                    self.runtime_call("__exs_rt_property_set", *span)?;
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
        let range_type_id = self
            .types
            .get("Range")
            .map_or(-1, |range| range.id.cast_signed());
        let snapshot = self.allocate_local();
        let inline_range = match iterable {
            Expression::StaticMethodCall {
                method,
                arguments,
                span: call_span,
                ..
            } if is_unshared_range(iterable) && arguments.len() == 2 => {
                Some(self.begin_literal_range(method, arguments, *call_span, snapshot, span)?)
            }
            _ => {
                let unshared_range = is_unshared_range(iterable);
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
                self.store_stack_value_in(snapshot)?;
                self.clear_root_slot(iterable)?;
                None
            }
        };

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

        // An inline range leaves the snapshot empty and counts in locals; otherwise the
        // runtime advances the snapshot.
        if let Some(range) = &inline_range {
            self.function.instruction(&Instruction::LocalGet(snapshot));
            self.function.instruction(&Instruction::I64Eqz);
            self.function
                .instruction(&Instruction::If(BlockType::Result(VALUE_TYPE)));
            self.enter_control()?;
            self.emit_inline_range_next(range);
            self.function.instruction(&Instruction::Else);
        }
        self.function.instruction(&Instruction::LocalGet(snapshot));
        self.function
            .instruction(&Instruction::I32Const(range_type_id));
        self.runtime_call("__exs_rt_iter_native_next", span)?;
        if inline_range.is_some() {
            self.function.instruction(&Instruction::End);
            self.exit_control()?;
        }
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

    /// Evaluates the bounds of a literal `a..b` or `a..=b` Range for a `for` loop.
    ///
    /// When both bounds are immediate Ints, the loop counts in Wasm locals and leaves
    /// `snapshot` empty. Otherwise it calls `Range::until` or `Range::through`, exactly like
    /// the desugared literal, and stores the runtime Range snapshot in `snapshot`.
    fn begin_literal_range(
        &mut self,
        method: &crate::ast::Identifier<'a>,
        arguments: &[Expression<'a>],
        call_span: SourceSpan<'a>,
        snapshot: u32,
        span: SourceSpan<'a>,
    ) -> Result<InlineRange, CompileDiagnostics<'a>> {
        let range_type_id = self
            .types
            .get("Range")
            .map_or(-1, |range| range.id.cast_signed());
        let key = format!("Range::{}", method.name);
        let signature = self.signatures.get(&key).cloned().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0999",
                call_span,
                format!("missing prelude `{key}` signature"),
            ))
        })?;
        let mut bounds = Vec::with_capacity(2);
        for argument in arguments {
            self.compile_expression(argument)?;
            bounds.push(self.store_stack_value()?);
        }
        let range = InlineRange {
            cursor: self.allocate_local(),
            last: self.allocate_local(),
            step: self.allocate_local(),
            done: self.allocate_local(),
            inclusive: method.name == "through",
        };
        immediate::both_small_ints_test(&mut self.function, bounds[0], bounds[1]);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.enter_control()?;
        immediate::push_payload(&mut self.function, bounds[0]);
        self.function
            .instruction(&Instruction::LocalSet(range.cursor));
        immediate::push_payload(&mut self.function, bounds[1]);
        self.function
            .instruction(&Instruction::LocalSet(range.last));
        // Like `Range::until` and `Range::through`, a start after the end counts down.
        self.function.instruction(&Instruction::I64Const(-1));
        self.function.instruction(&Instruction::I64Const(1));
        self.function
            .instruction(&Instruction::LocalGet(range.cursor));
        self.function
            .instruction(&Instruction::LocalGet(range.last));
        self.function.instruction(&Instruction::I64GtS);
        self.function.instruction(&Instruction::Select);
        self.function
            .instruction(&Instruction::LocalSet(range.step));
        self.function.instruction(&Instruction::I64Const(0));
        self.function
            .instruction(&Instruction::LocalSet(range.done));
        self.function.instruction(&Instruction::Else);
        let packed_tail = self.emit_call_arguments(&bounds, &signature, call_span)?;
        self.set_runtime_call_site(call_span)?;
        self.function
            .instruction(&Instruction::Call(signature.index));
        self.return_if_fatal_error(call_span)?;
        if let Some(list) = packed_tail {
            self.clear_root_slot(list)?;
        }
        self.function
            .instruction(&Instruction::I32Const(range_type_id));
        self.runtime_call("__exs_rt_iter_range_snapshot", span)?;
        self.return_if_error(span)?;
        self.store_stack_value_in(snapshot)?;
        self.function.instruction(&Instruction::End);
        self.exit_control()?;
        for bound in bounds {
            self.clear_root_slot(bound)?;
        }
        Ok(range)
    }

    /// Pushes the next item of an inline literal Range, or 0 once it is exhausted.
    ///
    /// Mirrors the prelude `Range::next`: the cursor never moves past the end, so it stays an
    /// immediate Int.
    fn emit_inline_range_next(&mut self, range: &InlineRange) {
        let (past_end_ascending, past_end_descending) = if range.inclusive {
            (Instruction::I64GtS, Instruction::I64LtS)
        } else {
            (Instruction::I64GeS, Instruction::I64LeS)
        };
        self.function
            .instruction(&Instruction::LocalGet(range.done));
        self.function.instruction(&Instruction::I64Eqz);
        self.function
            .instruction(&Instruction::If(BlockType::Result(VALUE_TYPE)));
        self.function
            .instruction(&Instruction::LocalGet(range.step));
        self.function.instruction(&Instruction::I64Const(0));
        self.function.instruction(&Instruction::I64GtS);
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        self.function
            .instruction(&Instruction::LocalGet(range.cursor));
        self.function
            .instruction(&Instruction::LocalGet(range.last));
        self.function.instruction(&past_end_ascending);
        self.function.instruction(&Instruction::Else);
        self.function
            .instruction(&Instruction::LocalGet(range.cursor));
        self.function
            .instruction(&Instruction::LocalGet(range.last));
        self.function.instruction(&past_end_descending);
        self.function.instruction(&Instruction::End);
        self.function
            .instruction(&Instruction::If(BlockType::Result(VALUE_TYPE)));
        self.function.instruction(&Instruction::I64Const(0));
        self.function.instruction(&Instruction::Else);
        immediate::push_small_int(&mut self.function, range.cursor);
        self.function
            .instruction(&Instruction::LocalGet(range.cursor));
        self.function
            .instruction(&Instruction::LocalGet(range.last));
        self.function.instruction(&Instruction::I64Eq);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.function.instruction(&Instruction::I64Const(1));
        self.function
            .instruction(&Instruction::LocalSet(range.done));
        self.function.instruction(&Instruction::Else);
        self.function
            .instruction(&Instruction::LocalGet(range.cursor));
        self.function
            .instruction(&Instruction::LocalGet(range.step));
        self.function.instruction(&Instruction::I64Add);
        self.function
            .instruction(&Instruction::LocalSet(range.cursor));
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::Else);
        self.function.instruction(&Instruction::I64Const(0));
        self.function.instruction(&Instruction::End);
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
