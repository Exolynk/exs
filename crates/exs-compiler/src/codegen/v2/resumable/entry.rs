//! The runner-facing exports of a module with resumable units: the run loop that dispatches
//! the active frame to its step function, the start of a resumable entry, and the resume and
//! cancel exports of a suspended task.

use core::mem::offset_of;

use exs_abi::{STATUS_CANCELLED, STATUS_COMPLETE, STATUS_PENDING, STATUS_READY};
use exs_layout::VmState;
use wasm_encoder::{BlockType, Function, Instruction, ValType};

use super::layouts::StepInfo;
use crate::codegen::v2::entry::EntryLayout;
use crate::codegen::v2::function::{
    FunctionInfo, emit_push_frame, root_stack_end_offset, slot_memory, vm_field,
};
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Byte offset of the first durable slot of a frame.
const FRAME_SLOTS: u64 = core::mem::size_of::<exs_layout::FrameObject>() as u64;

impl EntryLayout<'_> {
    /// Returns the `() -> i32` run loop: it runs the active frame's step function while the
    /// task is ready and returns `STATUS_PENDING` when it waits for the runner. On completion
    /// it encodes the result, releases the task and returns `STATUS_COMPLETE`.
    ///
    /// `steps` holds the Wasm index of every step function by dense step index.
    pub(in crate::codegen::v2) fn run<'a>(
        &self,
        steps: &[u32],
        span: SourceSpan<'a>,
    ) -> Result<Function, CompileDiagnostics<'a>> {
        // Locals: 0 the active frame, 1 the status of its step.
        let (frame, status) = (0, 1);
        let count = u32::try_from(steps.len()).unwrap_or(u32::MAX);
        let mut function = Function::new([(2, ValType::I32)]);
        function.instruction(&Instruction::Loop(BlockType::Empty));
        function.instruction(&Instruction::Block(BlockType::Empty));
        for _ in steps {
            function.instruction(&Instruction::Block(BlockType::Empty));
        }
        function.instruction(&Instruction::Block(BlockType::Empty));
        function.instruction(&Instruction::I32Const(0));
        function.instruction(&Instruction::I32Load(vm_field(
            self.vm,
            offset_of!(VmState, task.frame) as u32,
        )));
        function.instruction(&Instruction::LocalTee(frame));
        // The step index is the upper word of the frame's header.
        function.instruction(&Instruction::I32Load(wasm_encoder::MemArg {
            offset: 4,
            align: 2,
            memory_index: 0,
        }));
        let targets: Vec<u32> = (1..=count).collect();
        function.instruction(&Instruction::BrTable(targets.into(), 0));
        function.instruction(&Instruction::End);
        function.instruction(&Instruction::Unreachable);
        for (case, index) in (0_u32..).zip(steps) {
            function.instruction(&Instruction::End);
            function.instruction(&Instruction::LocalGet(frame));
            function.instruction(&Instruction::Call(*index));
            function.instruction(&Instruction::LocalSet(status));
            function.instruction(&Instruction::Br(count - 1 - case));
        }
        function.instruction(&Instruction::End);
        function.instruction(&Instruction::LocalGet(status));
        function.instruction(&Instruction::I32Const(STATUS_READY));
        function.instruction(&Instruction::I32Eq);
        function.instruction(&Instruction::BrIf(0));
        function.instruction(&Instruction::End);
        function.instruction(&Instruction::LocalGet(status));
        function.instruction(&Instruction::I32Const(STATUS_PENDING));
        function.instruction(&Instruction::I32Eq);
        function.instruction(&Instruction::If(BlockType::Empty));
        function.instruction(&Instruction::I32Const(STATUS_PENDING));
        function.instruction(&Instruction::Return);
        function.instruction(&Instruction::End);
        function.instruction(&Instruction::I32Const(self.vm.cast_signed()));
        function.instruction(&Instruction::I32Const(self.vm.cast_signed()));
        function.instruction(&Instruction::Call(
            self.core("__exs_core_take_completed", span)?,
        ));
        function.instruction(&Instruction::Call(
            self.core("__exs_core_encode_result", span)?,
        ));
        function.instruction(&Instruction::Call(
            self.core("__exs_core_task_release", span)?,
        ));
        function.instruction(&Instruction::I32Const(STATUS_COMPLETE));
        function.instruction(&Instruction::End);
        Ok(function)
    }

    /// Returns `__exs_start_<name>(input, length) -> status` of the resumable entry `target`:
    /// it decodes the arguments like a synchronous entry into the slots of the task's root
    /// frame and runs the task. Excess arguments of a fixed-arity entry complete it with the
    /// fatal ArityError at once.
    pub(in crate::codegen::v2) fn resumable_start<'a>(
        &self,
        target: &FunctionInfo,
        step: StepInfo,
        run: u32,
        span: SourceSpan<'a>,
    ) -> Result<Function, CompileDiagnostics<'a>> {
        // Locals: 0 and 1 are the input parameters, 2 the root-stack frame of the decoded
        // arguments, 3 the argument count, 4 the task's root frame.
        let (rooted, count, frame) = (2, 3, 4);
        let variadic = target.variadic.is_some();
        let arguments = target.arity + u32::from(variadic);
        let mut function = Function::new([(3, ValType::I32)]);
        function.instruction(&Instruction::Call(
            self.core("__exs_core_task_acquire", span)?,
        ));
        emit_push_frame(&mut function, self.vm, rooted, arguments);
        function.instruction(&Instruction::I32Const(self.vm.cast_signed()));
        function.instruction(&Instruction::LocalGet(rooted));
        function.instruction(&Instruction::I32Const(target.arity.cast_signed()));
        if variadic {
            function.instruction(&Instruction::Call(
                self.core("__exs_core_decode_variadic_inputs", span)?,
            ));
        } else {
            function.instruction(&Instruction::Call(
                self.core("__exs_core_decode_inputs", span)?,
            ));
            function.instruction(&Instruction::LocalSet(count));
            // Like v1, excess arguments make the result a fatal ArityError instead of a call.
            function.instruction(&Instruction::LocalGet(count));
            function.instruction(&Instruction::I32Const(target.arity.cast_signed()));
            function.instruction(&Instruction::I32GtU);
            function.instruction(&Instruction::If(BlockType::Empty));
            function.instruction(&Instruction::I32Const(self.vm.cast_signed()));
            function.instruction(&Instruction::I32Const(self.vm.cast_signed()));
            function.instruction(&Instruction::Call(
                self.core("__exs_core_arity_error", span)?,
            ));
            function.instruction(&Instruction::Call(
                self.core("__exs_core_encode_result", span)?,
            ));
            self.pop_root_frame(&mut function, rooted);
            function.instruction(&Instruction::Call(
                self.core("__exs_core_task_release", span)?,
            ));
            function.instruction(&Instruction::I32Const(STATUS_COMPLETE));
            function.instruction(&Instruction::Return);
            function.instruction(&Instruction::End);
        }
        function.instruction(&Instruction::I32Const(self.vm.cast_signed()));
        for value in [step.step, step.function_id, step.slot_count] {
            function.instruction(&Instruction::I32Const(value.cast_signed()));
        }
        function.instruction(&Instruction::Call(
            self.core("__exs_core_frame_start", span)?,
        ));
        function.instruction(&Instruction::LocalSet(frame));
        for slot in 0..arguments {
            function.instruction(&Instruction::LocalGet(frame));
            function.instruction(&Instruction::LocalGet(rooted));
            function.instruction(&Instruction::I64Load(slot_memory(slot)));
            function.instruction(&Instruction::I64Store(wasm_encoder::MemArg {
                offset: FRAME_SLOTS + u64::from(slot) * 8,
                align: 3,
                memory_index: 0,
            }));
        }
        self.pop_root_frame(&mut function, rooted);
        function.instruction(&Instruction::Call(run));
        function.instruction(&Instruction::End);
        Ok(function)
    }

    /// Pops the root-stack frame whose base is in the local `frame`.
    fn pop_root_frame(&self, function: &mut Function, frame: u32) {
        function.instruction(&Instruction::I32Const(0));
        function.instruction(&Instruction::LocalGet(frame));
        function.instruction(&Instruction::I32Store(vm_field(
            self.vm,
            root_stack_end_offset(),
        )));
    }

    /// Returns `__exs_resume_host(call_id, address, length) -> status`, which delivers the
    /// runner's response to the pending host call and runs the task on.
    pub(in crate::codegen::v2) fn resume_host<'a>(
        &self,
        run: u32,
        span: SourceSpan<'a>,
    ) -> Result<Function, CompileDiagnostics<'a>> {
        let mut function = Function::new([]);
        function.instruction(&Instruction::I32Const(self.vm.cast_signed()));
        function.instruction(&Instruction::LocalGet(0));
        function.instruction(&Instruction::LocalGet(1));
        function.instruction(&Instruction::LocalGet(2));
        function.instruction(&Instruction::Call(
            self.core("__exs_core_host_call_resume", span)?,
        ));
        function.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        function.instruction(&Instruction::I32Const(STATUS_CANCELLED));
        function.instruction(&Instruction::Else);
        function.instruction(&Instruction::Call(run));
        function.instruction(&Instruction::End);
        function.instruction(&Instruction::End);
        Ok(function)
    }

    /// Returns `__exs_cancel()`, which abandons the suspended task and releases it.
    pub(in crate::codegen::v2) fn cancel<'a>(
        &self,
        span: SourceSpan<'a>,
    ) -> Result<Function, CompileDiagnostics<'a>> {
        let mut function = Function::new([]);
        function.instruction(&Instruction::I32Const(self.vm.cast_signed()));
        function.instruction(&Instruction::Call(
            self.core("__exs_core_task_cancel", span)?,
        ));
        function.instruction(&Instruction::Call(
            self.core("__exs_core_task_release", span)?,
        ));
        function.instruction(&Instruction::End);
        Ok(function)
    }
}
