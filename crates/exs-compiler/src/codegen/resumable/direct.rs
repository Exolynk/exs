//! Direct statements in a step function: synchronous code over locals loaded from the frame.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Instruction};

use super::step::StepEmitter;
use crate::ast::Statement;
use crate::codegen::continuation::DirectBinding;
use crate::codegen::expressions::expression_span;
use crate::codegen::function::error;
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

impl<'a> StepEmitter<'a, '_> {
    /// Runs the direct statement `statement` of state `state` over its `bindings`, then
    /// continues with the following state.
    ///
    /// With `checkpoint`, the statement is a `while` loop that checks the scheduler before
    /// every iteration; when the task yields, the assigned bindings are stored and the loop
    /// restarts at its condition when the task runs again.
    pub(super) fn direct(
        &mut self,
        state: u32,
        statement: &Statement<'a>,
        bindings: &[DirectBinding],
        checkpoint: bool,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let mut scope = HashMap::with_capacity(bindings.len());
        let mut assigned = Vec::new();
        for binding in bindings {
            let local = self.load_rooted(binding.slot);
            scope.insert(binding.name.clone(), local);
            if binding.assigned {
                assigned.push((binding.slot, local));
            }
        }
        self.compiler.scopes.push(scope);
        let compiled = match statement {
            Statement::While {
                condition, body, ..
            } if checkpoint => {
                self.instruction(Instruction::Block(BlockType::Empty));
                let break_depth = self.compiler.depth;
                self.instruction(Instruction::Loop(BlockType::Empty));
                let continue_depth = self.compiler.depth;
                self.yield_checkpoint(state, span, |emitter| emitter.store_back(&assigned))?;
                self.compiler
                    .compile_condition(condition, expression_span(condition))?;
                self.instruction(Instruction::I32Eqz);
                self.instruction(Instruction::BrIf(self.compiler.depth - break_depth));
                self.compiler.loops.push((break_depth, continue_depth));
                self.compiler.compile_block(body)?;
                self.compiler.loops.pop();
                self.compiler.branch_to(continue_depth);
                self.instruction(Instruction::End);
                self.instruction(Instruction::End);
                Ok(())
            }
            Statement::While { .. } | Statement::For { .. } | Statement::If { .. } => {
                self.compiler.compile_statement(statement)
            }
            _ => Err(error(
                "E0999",
                span,
                "only while, for and if statements run directly".to_owned(),
            )),
        };
        self.compiler.scopes.pop();
        compiled?;
        self.store_back(&assigned);
        self.ready(state + 1, span)
    }

    /// Stores the locals of the `assigned` bindings back into their slots.
    fn store_back(&mut self, assigned: &[(u32, u32)]) {
        for (slot, local) in assigned {
            self.instruction(Instruction::LocalGet(*local));
            self.set_slot(*slot);
        }
    }
}
