//! Closure calls in resumable code: a closure that may suspend runs as a child frame, any
//! other value goes through the synchronous closure call.

use exs_layout::{STATUS_READY, TYPE_FN};
use wasm_encoder::{BlockType, Instruction};

use super::layouts::ClosureStep;
use super::step::StepEmitter;
use crate::ast::{Expression, Identifier};
use crate::codegen::closures::{CAPTURES, CLOSURE_INDEX, accepts};
use crate::codegen::collections::{u32_field, value_field};
use crate::codegen::immediate;
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

impl<'a> StepEmitter<'a, '_> {
    /// Calls the closure in durable slot `closure` with the durable `arguments`, storing the
    /// result in `destination` before continuing at `next`.
    ///
    /// The closure's dense index selects a closure that may suspend, which starts a child
    /// frame; every other value is called like synchronous code, which also creates v1's
    /// Errors for values that are no closures.
    pub(super) fn closure_call(
        &mut self,
        next: u32,
        closure: u32,
        arguments: &[u32],
        destination: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let mut cases: Vec<(u32, ClosureStep)> = self
            .resumable
            .closures
            .iter()
            .map(|(index, step)| (*index, *step))
            .collect();
        if !cases.is_empty() {
            cases.sort_by_key(|(index, _)| *index);
            let callee = self.load(closure);
            self.instruction(Instruction::LocalGet(callee));
            immediate::type_mask_test(&mut self.compiler.function, TYPE_FN);
            self.instruction(Instruction::If(BlockType::Empty));
            // The outermost block continues synchronously; label `k` selects case `k`.
            self.instruction(Instruction::Block(BlockType::Empty));
            for _ in &cases {
                self.instruction(Instruction::Block(BlockType::Empty));
            }
            self.instruction(Instruction::LocalGet(callee));
            self.instruction(Instruction::I32WrapI64);
            self.instruction(Instruction::I32Load(u32_field(CLOSURE_INDEX)));
            let fallback = cases.len() as u32;
            let limit = cases.last().map_or(0, |(index, _)| index + 1);
            let targets: Vec<u32> = (0..limit)
                .map(|index| {
                    cases
                        .iter()
                        .position(|(case, _)| *case == index)
                        .map_or(fallback, |case| case as u32)
                })
                .collect();
            self.instruction(Instruction::BrTable(targets.into(), fallback));
            for (_, case) in &cases {
                self.instruction(Instruction::End);
                self.suspending_closure(next, callee, *case, arguments, destination, span)?;
            }
            self.instruction(Instruction::End);
            self.instruction(Instruction::End);
        }
        let slots: Vec<u32> = core::iter::once(closure)
            .chain(arguments.iter().copied())
            .collect();
        self.synthesize(&slots, span, |mut operands| {
            let callee = match operands.remove(0) {
                Expression::Variable(identifier) => identifier,
                _ => Identifier {
                    name: "$0".to_owned(),
                    span,
                },
            };
            Expression::Call {
                callee,
                arguments: operands,
                span,
            }
        })?;
        self.set_slot(destination);
        self.ready(next, span)
    }

    /// Starts a child frame of the closure `case`, whose value is in the temporary `callee`,
    /// with its captured Cells and the durable `arguments`, or stores v1's ArityError for a
    /// wrong argument count.
    fn suspending_closure(
        &mut self,
        next: u32,
        callee: u32,
        case: ClosureStep,
        arguments: &[u32],
        destination: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if !accepts(case.arity, case.variadic, arguments.len()) {
            self.compiler.set_position(span)?;
            self.compiler.push_vm();
            self.compiler
                .core_call("__exs_core_closure_arity_error", span)?;
            self.set_slot(destination);
            return self.ready(next, span);
        }
        let fixed = case.arity as usize;
        let rest = self.rest_list(
            &arguments[fixed.min(arguments.len())..],
            case.variadic,
            span,
        )?;
        let child = self.link_child(next, case.step, destination, span)?;
        for capture in 0..case.captures {
            self.child_slot(child, capture, |emitter| {
                emitter.instruction(Instruction::LocalGet(callee));
                emitter.instruction(Instruction::I32WrapI64);
                emitter.instruction(Instruction::I64Load(value_field(
                    CAPTURES + u64::from(capture) * 8,
                )));
            });
        }
        for (slot, argument) in (case.captures..).zip(&arguments[..fixed]) {
            self.child_slot(child, slot, |emitter| emitter.get_slot(*argument));
        }
        if let Some(rest) = rest {
            self.child_slot(child, case.captures + case.arity, |emitter| {
                emitter.instruction(Instruction::LocalGet(rest));
            });
        }
        self.leave(STATUS_READY);
        Ok(())
    }

    /// Returns a rooted temporary holding a new List of the durable `rest` arguments when the
    /// callee is `variadic`.
    pub(super) fn rest_list(
        &mut self,
        rest: &[u32],
        variadic: bool,
        span: SourceSpan<'a>,
    ) -> Result<Option<u32>, CompileDiagnostics<'a>> {
        if !variadic {
            return Ok(None);
        }
        let count = rest.len() as u32;
        let block = self.compiler.consecutive_locals(count.max(1));
        for (offset, argument) in (0..count).zip(rest) {
            self.get_slot(*argument);
            self.compiler.store(block + offset);
        }
        self.compiler.push_vm();
        self.compiler.push_slot_address(block);
        self.instruction(Instruction::I32Const(count.cast_signed()));
        self.compiler.core_call("__exs_core_list_new", span)?;
        Ok(Some(self.compiler.store_new()))
    }
}
