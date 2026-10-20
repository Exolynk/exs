//! The runner-facing exports of a v2 module.

use std::collections::HashMap;

use exs_abi::{ABI_VERSION, STATUS_COMPLETE};
use wasm_encoder::{BlockType, Function, Instruction, ValType};

use super::function::{
    FunctionInfo, emit_push_frame, error, root_stack_end_offset, slot_memory, vm_field,
};
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Pages of root-stack storage created when the module first runs.
pub(super) const ROOT_STACK_PAGES: u32 = 16;

/// Addresses the generated exports work with.
pub(super) struct EntryLayout<'m> {
    /// Core template export indices.
    pub(super) core: &'m HashMap<String, u32>,
    /// Address of the VM state block.
    pub(super) vm: u32,
    /// First address available to the heap.
    pub(super) heap_start: u32,
}

impl EntryLayout<'_> {
    /// Returns the index of one core export.
    fn core<'a>(&self, name: &str, span: SourceSpan<'a>) -> Result<u32, CompileDiagnostics<'a>> {
        self.core.get(name).copied().ok_or_else(|| {
            error(
                "E1001",
                span,
                format!("the v2 core template does not export `{name}`"),
            )
        })
    }

    /// Returns `__exs_start_<name>(input, length) -> status`, which decodes the arguments,
    /// calls the function and encodes its result.
    ///
    /// The input was already decoded into the VM buffer by `__exs_input_alloc`; excess
    /// arguments trap for now.
    pub(super) fn start<'a>(
        &self,
        target: &FunctionInfo,
        span: SourceSpan<'a>,
    ) -> Result<Function, CompileDiagnostics<'a>> {
        // Locals: 0 and 1 are the input parameters, 2 the frame base, 3 the argument count.
        let (frame, count) = (2, 3);
        let mut function = Function::new([(2, ValType::I32)]);
        function.instruction(&Instruction::Call(
            self.core("__exs_core_task_acquire", span)?,
        ));
        emit_push_frame(&mut function, self.vm, frame, target.arity);
        function.instruction(&Instruction::I32Const(self.vm.cast_signed()));
        function.instruction(&Instruction::LocalGet(frame));
        function.instruction(&Instruction::I32Const(target.arity.cast_signed()));
        function.instruction(&Instruction::Call(
            self.core("__exs_core_decode_inputs", span)?,
        ));
        function.instruction(&Instruction::LocalTee(count));
        function.instruction(&Instruction::I32Const(target.arity.cast_signed()));
        function.instruction(&Instruction::I32GtU);
        function.instruction(&Instruction::If(BlockType::Empty));
        function.instruction(&Instruction::Unreachable);
        function.instruction(&Instruction::End);
        function.instruction(&Instruction::I32Const(self.vm.cast_signed()));
        for slot in 0..target.arity {
            function.instruction(&Instruction::LocalGet(frame));
            function.instruction(&Instruction::I64Load(slot_memory(slot)));
        }
        function.instruction(&Instruction::Call(target.index));
        function.instruction(&Instruction::Call(
            self.core("__exs_core_encode_result", span)?,
        ));
        function.instruction(&Instruction::I32Const(0));
        function.instruction(&Instruction::LocalGet(frame));
        function.instruction(&Instruction::I32Store(vm_field(
            self.vm,
            root_stack_end_offset(),
        )));
        function.instruction(&Instruction::Call(
            self.core("__exs_core_task_release", span)?,
        ));
        function.instruction(&Instruction::I32Const(STATUS_COMPLETE));
        function.instruction(&Instruction::End);
        Ok(function)
    }

    /// Returns `__exs_input_alloc(length) -> address`, which initializes the VM on first use.
    pub(super) fn input_alloc<'a>(
        &self,
        span: SourceSpan<'a>,
    ) -> Result<Function, CompileDiagnostics<'a>> {
        let mut function = Function::new([]);
        function.instruction(&Instruction::I32Const(self.vm.cast_signed()));
        function.instruction(&Instruction::I32Const(self.heap_start.cast_signed()));
        function.instruction(&Instruction::I32Const(ROOT_STACK_PAGES.cast_signed()));
        function.instruction(&Instruction::Call(self.core("__exs_core_init", span)?));
        function.instruction(&Instruction::I32Const(self.vm.cast_signed()));
        function.instruction(&Instruction::LocalGet(0));
        function.instruction(&Instruction::Call(
            self.core("__exs_core_reserve_input", span)?,
        ));
        function.instruction(&Instruction::End);
        Ok(function)
    }

    /// Returns a `() -> i32` export that forwards to the core function `core` with the VM.
    pub(super) fn vm_accessor<'a>(
        &self,
        core: &str,
        span: SourceSpan<'a>,
    ) -> Result<Function, CompileDiagnostics<'a>> {
        let mut function = Function::new([]);
        function.instruction(&Instruction::I32Const(self.vm.cast_signed()));
        function.instruction(&Instruction::Call(self.core(core, span)?));
        function.instruction(&Instruction::End);
        Ok(function)
    }

    /// Returns `__exs_abi_version() -> i32`.
    pub(super) fn abi_version() -> Function {
        let mut function = Function::new([]);
        function.instruction(&Instruction::I32Const(ABI_VERSION.cast_signed()));
        function.instruction(&Instruction::End);
        function
    }

    /// Returns the `(i64) -> i64` helper that turns one plain Int into its `ValueRef`.
    pub(super) fn int_new<'a>(
        &self,
        span: SourceSpan<'a>,
    ) -> Result<Function, CompileDiagnostics<'a>> {
        let mut function = Function::new([]);
        function.instruction(&Instruction::I32Const(self.vm.cast_signed()));
        function.instruction(&Instruction::LocalGet(0));
        function.instruction(&Instruction::Call(self.core("__exs_core_int", span)?));
        function.instruction(&Instruction::End);
        Ok(function)
    }
}
