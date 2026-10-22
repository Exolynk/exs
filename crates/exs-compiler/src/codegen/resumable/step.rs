//! The step function of one resumable unit: a state machine over v1's continuation graph.
//!
//! The body is `loop { block_{n-1} .. block_0 { br_table } state_0 end state_1 .. }` like
//! v1's: a later state is reached by branching to the end of its block, an earlier one through
//! the state local and the loop. Durable values live in the frame's slots; every operation
//! loads what it needs into temporaries of the step function's root frame, which are released
//! after the operation. The step function returns to the dispatcher only at suspension points,
//! child calls and completion.

use std::collections::{HashMap, HashSet};

use core::mem::{offset_of, size_of};

use exs_abi::{STATUS_READY, TYPE_ERROR};
use exs_layout::FrameObject;
use wasm_encoder::{BlockType, Function, Instruction};

use super::constants::{constant, constant_slots};
use super::layouts::{Resumable, StepInfo};
use crate::ast::FunctionDeclaration;
use crate::codegen::collections::{u32_field, value_field};
use crate::codegen::continuation::{ContinuationGraph, Operation, operation_span};
use crate::codegen::function::{Context, FunctionCompiler, error};
use crate::codegen::immediate;
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Byte offset of [`FrameObject::state`].
const STATE: u64 = offset_of!(FrameObject, state) as u64;
/// Byte offset of the first durable slot of a frame.
pub(super) const SLOTS: u64 = size_of::<FrameObject>() as u64;

/// The emitter of one step function.
pub(in crate::codegen) struct StepEmitter<'a, 'm> {
    /// The function compiler in step mode, whose helpers implement the operations.
    pub(super) compiler: FunctionCompiler<'a, 'm>,
    /// The module's resumable units.
    pub(super) resumable: &'m Resumable<'m, 'a>,
    /// The `i64` local holding the state the next loop iteration runs.
    state: u32,
    /// Control depth directly inside the state loop.
    loop_depth: u32,
    /// Number of states.
    state_count: u32,
    /// The state being emitted.
    current: u32,
    /// The `Iterator::next` dispatch states of `for` loops, which follow their native step.
    pub(super) iterator_next_states: HashSet<u32>,
    /// The raw value of every slot that only holds one constant.
    constants: HashMap<u32, u64>,
    /// Whether the operation ended by falling through to the following state, whose branch
    /// was left out.
    falls_through: bool,
}

impl<'a, 'm> StepEmitter<'a, 'm> {
    /// Compiles the step function of the resumable unit `key` declared by `declaration`.
    pub(in crate::codegen) fn compile(
        context: &'m Context<'m>,
        resumable: &'m Resumable<'m, 'a>,
        key: &'m str,
        declaration: &'m FunctionDeclaration<'a>,
    ) -> Result<Function, CompileDiagnostics<'a>> {
        let signature = resumable.signatures.get(key).ok_or_else(|| {
            error(
                "E0999",
                declaration.span,
                format!("missing resumable signature `{key}`"),
            )
        })?;
        let graph = ContinuationGraph::build(
            declaration,
            signature,
            &resumable.signatures,
            &resumable.frame_layouts,
            resumable.lifted,
            &resumable.methods,
            context.types,
        )?;
        let mut compiler = FunctionCompiler::step(context, key, declaration);
        let state = compiler.scalar_local();
        let mut emitter = Self {
            compiler,
            resumable,
            state,
            loop_depth: 0,
            state_count: 0,
            current: 0,
            iterator_next_states: (1_u32..)
                .zip(&graph.operations)
                .filter(|(_, operation)| matches!(operation, Operation::NativeIterNext { .. }))
                .map(|(next, _)| next)
                .collect(),
            constants: constant_slots(&graph.operations, context.strings),
            falls_through: false,
        };
        emitter.dispatch(&graph.operations, declaration.span)?;
        emitter.compiler.instruction(Instruction::Unreachable);
        emitter.compiler.instruction(Instruction::End);
        Ok(emitter.compiler.finish_step())
    }

    /// Emits the state loop over every operation.
    fn dispatch(
        &mut self,
        operations: &[Operation<'a, '_>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let count = u32::try_from(operations.len())
            .map_err(|_| error("E0212", span, "too many continuation states".to_owned()))?;
        self.state_count = count;
        self.instruction(Instruction::LocalGet(0));
        self.instruction(Instruction::I32Load(u32_field(STATE)));
        self.instruction(Instruction::I64ExtendI32U);
        self.instruction(Instruction::LocalSet(self.state));
        self.instruction(Instruction::Loop(BlockType::Empty));
        self.loop_depth = self.compiler.depth;
        for _ in 0..count {
            self.instruction(Instruction::Block(BlockType::Empty));
        }
        // The innermost block catches invalid states; label `index + 1` selects state `index`.
        self.instruction(Instruction::Block(BlockType::Empty));
        self.instruction(Instruction::LocalGet(self.state));
        self.instruction(Instruction::I32WrapI64);
        let targets: Vec<u32> = (1..=count).collect();
        self.instruction(Instruction::BrTable(targets.into(), 0));
        self.instruction(Instruction::End);
        self.instruction(Instruction::Unreachable);
        for (state, operation) in (0..count).zip(operations) {
            self.instruction(Instruction::End);
            self.current = state;
            let mark = self.compiler.temporaries();
            self.operation(state, operation)?;
            self.compiler.release_temporaries(mark);
            if !core::mem::take(&mut self.falls_through) {
                self.instruction(Instruction::Unreachable);
            }
        }
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Returns the state being emitted.
    pub(super) const fn current_state(&self) -> u32 {
        self.current
    }

    /// Emits one instruction.
    pub(super) fn instruction(&mut self, instruction: Instruction<'_>) {
        // A branch to the following state may only be left out when nothing follows it.
        if core::mem::take(&mut self.falls_through) {
            self.compiler.instruction(Instruction::Br(0));
        }
        self.compiler.instruction(instruction);
    }

    /// Continues with state `next` of this frame without returning to the dispatcher.
    pub(super) fn ready(
        &mut self,
        next: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let depth = self.compiler.depth;
        let label = if next > self.current {
            (self.loop_depth + self.state_count)
                .checked_sub(next)
                .and_then(|target| depth.checked_sub(target))
        } else {
            self.instruction(Instruction::I64Const(i64::from(next)));
            self.instruction(Instruction::LocalSet(self.state));
            depth.checked_sub(self.loop_depth)
        };
        let label = label.ok_or_else(|| {
            error(
                "E0999",
                span,
                "invalid continuation state transition".to_owned(),
            )
        })?;
        if label == 0 && next == self.current + 1 {
            // The operation's block ends right before the following state.
            self.falls_through = true;
        } else {
            self.instruction(Instruction::Br(label));
        }
        Ok(())
    }

    /// Stores the state the dispatcher runs when it next runs this frame.
    pub(super) fn store_state(&mut self, state: u32) {
        self.instruction(Instruction::LocalGet(0));
        self.instruction(Instruction::I32Const(state.cast_signed()));
        self.instruction(Instruction::I32Store(u32_field(STATE)));
    }

    /// Pops the step function's root frame and returns `status` to the dispatcher.
    pub(super) fn leave(&mut self, status: i32) {
        self.compiler.pop_frame();
        self.instruction(Instruction::I32Const(status));
        self.instruction(Instruction::Return);
    }

    /// Pushes the value of durable slot `slot`, or its constant.
    pub(super) fn get_slot(&mut self, slot: u32) {
        if let Some(raw) = self.constants.get(&slot).copied() {
            self.instruction(Instruction::I64Const(raw.cast_signed()));
            return;
        }
        self.instruction(Instruction::LocalGet(0));
        self.instruction(Instruction::I64Load(value_field(
            SLOTS + u64::from(slot) * 8,
        )));
    }

    /// Pops the stack's value into durable slot `slot`.
    pub(super) fn set_slot(&mut self, slot: u32) {
        let scratch = self.compiler.scratch;
        self.instruction(Instruction::LocalSet(scratch));
        self.instruction(Instruction::LocalGet(0));
        self.instruction(Instruction::LocalGet(scratch));
        self.instruction(Instruction::I64Store(value_field(
            SLOTS + u64::from(slot) * 8,
        )));
    }

    /// Loads durable slot `slot` into a new temporary and returns it.
    ///
    /// The temporary is not rooted: the frame's slot keeps its value alive, as no operation
    /// overwrites an operand's slot before its last use. Code that stores another value into
    /// the temporary needs [`Self::load_rooted`].
    pub(super) fn load(&mut self, slot: u32) -> u32 {
        self.get_slot(slot);
        let local = self.compiler.scalar_local();
        self.instruction(Instruction::LocalSet(local));
        local
    }

    /// Loads durable slot `slot` into a new rooted temporary and returns it.
    pub(super) fn load_rooted(&mut self, slot: u32) -> u32 {
        self.get_slot(slot);
        self.compiler.store_new()
    }

    /// Returns from the function with the Error in durable slot `slot`, like synchronous
    /// code's `return_if_error`.
    pub(super) fn return_if_error(
        &mut self,
        slot: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let local = self.load(slot);
        self.compiler.return_if_error(local, span)
    }

    /// Returns from the function with the fatal Error in durable slot `slot`.
    pub(super) fn return_if_fatal(&mut self, slot: u32) {
        let local = self.load(slot);
        self.compiler.return_if_fatal(local);
    }

    /// Pushes 1 when durable slot `slot` holds an Error.
    pub(super) fn is_error(&mut self, slot: u32) {
        self.get_slot(slot);
        immediate::type_mask_test(&mut self.compiler.function, TYPE_ERROR);
    }

    /// Calls the resumable unit `callee` from this frame with the values of the durable
    /// `arguments` as its first slots: the child becomes the active frame, whose result lands
    /// in `destination` before this frame continues at `next`.
    pub(super) fn child_call(
        &mut self,
        next: u32,
        callee: StepInfo,
        arguments: &[u32],
        destination: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let child = self.link_child(next, callee, destination, span)?;
        for (slot, argument) in (0_u32..).zip(arguments) {
            self.child_slot(child, slot, |emitter| emitter.get_slot(*argument));
        }
        self.leave(STATUS_READY);
        Ok(())
    }

    /// Links a frame of the resumable unit `callee` as the child of this frame, which
    /// continues at `next` with the child's result in `destination`, and returns the `i64`
    /// local holding the child's address. The caller fills its slots and leaves with
    /// `STATUS_READY`.
    pub(super) fn link_child(
        &mut self,
        next: u32,
        callee: StepInfo,
        destination: u32,
        span: SourceSpan<'a>,
    ) -> Result<u32, CompileDiagnostics<'a>> {
        self.store_state(next);
        self.compiler.set_call_site(span)?;
        self.compiler.push_vm();
        for value in [
            callee.step,
            callee.function_id,
            callee.slot_count,
            destination,
        ] {
            self.instruction(Instruction::I32Const(value.cast_signed()));
        }
        self.compiler.core_call("__exs_core_frame_call", span)?;
        let child = self.compiler.scalar_local();
        self.instruction(Instruction::I64ExtendI32U);
        self.instruction(Instruction::LocalSet(child));
        Ok(child)
    }

    /// Stores the value `value` pushes into slot `slot` of the frame whose address is in the
    /// `i64` local `child`.
    pub(super) fn child_slot(&mut self, child: u32, slot: u32, value: impl FnOnce(&mut Self)) {
        self.instruction(Instruction::LocalGet(child));
        self.instruction(Instruction::I32WrapI64);
        value(self);
        self.instruction(Instruction::I64Store(value_field(
            SLOTS + u64::from(slot) * 8,
        )));
    }

    /// Returns the step of the unit with the trace id `function_id`, if it is resumable.
    pub(super) fn step_of(&self, function_id: u32) -> Option<StepInfo> {
        self.resumable.step_of(function_id)
    }

    /// Returns the key of the unit with the trace id `function_id`.
    pub(super) fn key_of(
        &self,
        function_id: u32,
        span: SourceSpan<'a>,
    ) -> Result<&'m str, CompileDiagnostics<'a>> {
        self.resumable
            .keys
            .get(&function_id)
            .map(String::as_str)
            .ok_or_else(|| error("E0999", span, format!("missing unit {function_id}")))
    }

    /// Emits one operation, which always ends in a state transition or a return.
    fn operation(
        &mut self,
        state: u32,
        operation: &Operation<'a, '_>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let next = state + 1;
        let span = operation_span(operation);
        // A constant's readers use it directly, so storing it is unnecessary.
        if constant(operation, self.compiler.context.strings)
            .is_some_and(|(slot, _)| self.constants.contains_key(&slot))
        {
            return self.ready(next, span);
        }
        if self.value_operation(operation)? {
            return self.ready(next, span);
        }
        if self.call_operation(next, operation)? {
            return Ok(());
        }
        if self.flow_operation(state, operation)? {
            return Ok(());
        }
        self.host_operation(state, operation)
    }
}
