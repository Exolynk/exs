//! Statement lowering of the v2 backend.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Instruction};

use super::function::{FunctionCompiler, unsupported};
use crate::ast::{AssignmentTarget, Block, ElseBranch, Expression, Statement};
use crate::codegen::immediate;
use crate::codegen::standard::is_unshared_range;
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

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
    fn compile_statement(
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
                    self.check_contract(local, &contract, *span)?;
                }
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(name.name.clone(), local);
                }
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
                self.store(local);
            }
            Statement::Assign { span, .. } => {
                return Err(unsupported(*span, "index and property assignments"));
            }
            Statement::Return { value, span } => {
                match value {
                    Some(value) => self.compile_expression(value)?,
                    None => immediate::push(&mut self.function, exs_value::ValueRef::NONE),
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
                self.condition();
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
                self.condition();
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
                if type_annotation.is_some() {
                    return Err(unsupported(*span, "typed for-loop bindings"));
                }
                self.compile_range_for(&binding.name, iterable, body, *span)?;
            }
            Statement::Break { span } | Statement::Continue { span } => {
                let Some((break_depth, continue_depth)) = self.loops.last().copied() else {
                    return Err(unsupported(*span, "break and continue outside loops"));
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

    /// Compiles a `for` loop over a literal `a..b` or `a..=b` Range of immediate Ints.
    ///
    /// Mirrors the prelude `Range::next`: the cursor moves toward the end by one and an
    /// inclusive Range yields its end last. Bounds outside the `i32` range trap for now.
    fn compile_range_for(
        &mut self,
        binding: &str,
        iterable: &Expression<'a>,
        body: &Block<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let Expression::StaticMethodCall {
            method, arguments, ..
        } = iterable
        else {
            return Err(unsupported(
                span,
                "for loops over values other than Range literals",
            ));
        };
        if !is_unshared_range(iterable) || arguments.len() != 2 {
            return Err(unsupported(
                span,
                "for loops over values other than Range literals",
            ));
        }
        let inclusive = method.name == "through";
        self.compile_expression(&arguments[0])?;
        let start = self.store_new();
        self.compile_expression(&arguments[1])?;
        let end = self.store_new();
        immediate::both_small_ints_test(&mut self.function, start, end);
        self.instruction(Instruction::I32Eqz);
        self.instruction(Instruction::If(BlockType::Empty));
        self.instruction(Instruction::Unreachable);
        self.instruction(Instruction::End);
        let (cursor, last, step, done) = (
            self.scalar_local(),
            self.scalar_local(),
            self.scalar_local(),
            self.scalar_local(),
        );
        immediate::push_payload(&mut self.function, start);
        self.instruction(Instruction::LocalSet(cursor));
        immediate::push_payload(&mut self.function, end);
        self.instruction(Instruction::LocalSet(last));
        // A start after the end counts down.
        self.instruction(Instruction::I64Const(-1));
        self.instruction(Instruction::I64Const(1));
        self.instruction(Instruction::LocalGet(cursor));
        self.instruction(Instruction::LocalGet(last));
        self.instruction(Instruction::I64GtS);
        self.instruction(Instruction::Select);
        self.instruction(Instruction::LocalSet(step));
        self.instruction(Instruction::I64Const(0));
        self.instruction(Instruction::LocalSet(done));
        let item = self.value_local();
        self.scopes
            .push(HashMap::from([(binding.to_owned(), item)]));
        self.instruction(Instruction::Block(BlockType::Empty));
        let break_depth = self.depth;
        self.instruction(Instruction::Loop(BlockType::Empty));
        let loop_depth = self.depth;
        self.instruction(Instruction::LocalGet(done));
        self.instruction(Instruction::I32WrapI64);
        self.instruction(Instruction::BrIf(self.depth - break_depth));
        let (ascending_past, descending_past) = if inclusive {
            (Instruction::I64GtS, Instruction::I64LtS)
        } else {
            (Instruction::I64GeS, Instruction::I64LeS)
        };
        self.instruction(Instruction::LocalGet(step));
        self.instruction(Instruction::I64Const(0));
        self.instruction(Instruction::I64GtS);
        self.instruction(Instruction::If(BlockType::Result(
            wasm_encoder::ValType::I32,
        )));
        self.instruction(Instruction::LocalGet(cursor));
        self.instruction(Instruction::LocalGet(last));
        self.instruction(ascending_past);
        self.instruction(Instruction::Else);
        self.instruction(Instruction::LocalGet(cursor));
        self.instruction(Instruction::LocalGet(last));
        self.instruction(descending_past);
        self.instruction(Instruction::End);
        self.instruction(Instruction::BrIf(self.depth - break_depth));
        immediate::push_small_int(&mut self.function, cursor);
        self.store(item);
        self.instruction(Instruction::LocalGet(cursor));
        self.instruction(Instruction::LocalGet(last));
        self.instruction(Instruction::I64Eq);
        self.instruction(Instruction::If(BlockType::Empty));
        self.instruction(Instruction::I64Const(1));
        self.instruction(Instruction::LocalSet(done));
        self.instruction(Instruction::Else);
        self.instruction(Instruction::LocalGet(cursor));
        self.instruction(Instruction::LocalGet(step));
        self.instruction(Instruction::I64Add);
        self.instruction(Instruction::LocalSet(cursor));
        self.instruction(Instruction::End);
        // The body is one block, so `continue` leaves it and returns to the loop head.
        self.instruction(Instruction::Block(BlockType::Empty));
        let continue_depth = self.depth;
        self.loops.push((break_depth, continue_depth));
        for statement in &body.statements {
            self.compile_statement(statement)?;
        }
        self.loops.pop();
        self.instruction(Instruction::End);
        self.branch_to(loop_depth);
        self.instruction(Instruction::End);
        self.instruction(Instruction::End);
        self.scopes.pop();
        Ok(())
    }
}
