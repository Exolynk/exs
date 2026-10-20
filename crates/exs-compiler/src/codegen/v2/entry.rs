//! The runner-facing exports of a v2 module.

use std::collections::HashMap;

use core::mem::offset_of;

use exs_abi::{ABI_VERSION, STATUS_COMPLETE, TYPE_FLOAT, TYPE_INT};
use exs_layout::{FreeCell, ObjectKind, VmState};
use exs_value::layout::{EXTRA_SHIFT, INT_BOXED, KIND_SHIFT, type_kind};
use wasm_encoder::{BlockType, Function, Instruction, MemArg, ValType};

use crate::codegen::immediate;

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

    /// Returns the `(i64) -> i64` helper that boxes one Int value or Float bits.
    ///
    /// Ints inside the `i32` range stay immediates. Otherwise it takes a 16-byte cell straight
    /// from the size-class free list and only calls the core allocator when the list is empty,
    /// a collection is due or the heap debug flags are set.
    pub(super) fn boxed_number<'a>(
        &self,
        kind: ObjectKind,
        span: SourceSpan<'a>,
    ) -> Result<Function, CompileDiagnostics<'a>> {
        let (type_bit, extra) = match kind {
            ObjectKind::BigInt => (TYPE_INT, INT_BOXED),
            _ => (TYPE_FLOAT, 0),
        };
        let cell = 1;
        let mut function = Function::new([(1, ValType::I32)]);
        if kind == ObjectKind::BigInt {
            function.instruction(&Instruction::LocalGet(0));
            function.instruction(&Instruction::I32WrapI64);
            function.instruction(&Instruction::I64ExtendI32S);
            function.instruction(&Instruction::LocalGet(0));
            function.instruction(&Instruction::I64Eq);
            function.instruction(&Instruction::If(BlockType::Empty));
            immediate::push_small_int(&mut function, 0);
            function.instruction(&Instruction::Return);
            function.instruction(&Instruction::End);
        }
        let field = |offset: usize| vm_field(self.vm, offset as u32);
        let head = field(offset_of!(VmState, heap.free_heads));
        let allocated = field(offset_of!(VmState, heap.allocated_since_collection));
        function.instruction(&Instruction::I32Const(0));
        function.instruction(&Instruction::I32Load(head));
        function.instruction(&Instruction::LocalTee(cell));
        function.instruction(&Instruction::I32Eqz);
        function.instruction(&Instruction::I32Const(0));
        function.instruction(&Instruction::I32Load(allocated));
        function.instruction(&Instruction::I32Const(0));
        function.instruction(&Instruction::I32Load(field(offset_of!(
            VmState,
            heap.collection_threshold
        ))));
        function.instruction(&Instruction::I32GeU);
        function.instruction(&Instruction::I32Or);
        function.instruction(&Instruction::I32Const(0));
        function.instruction(&Instruction::I32Load(field(offset_of!(
            VmState, heap.flags
        ))));
        function.instruction(&Instruction::I32Or);
        function.instruction(&Instruction::If(BlockType::Empty));
        function.instruction(&Instruction::I32Const(self.vm.cast_signed()));
        function.instruction(&Instruction::I32Const(CELL_SIZE.cast_signed()));
        function.instruction(&Instruction::I32Const(i32::from(kind as u8)));
        function.instruction(&Instruction::Call(self.core("__exs_core_allocate", span)?));
        function.instruction(&Instruction::LocalSet(cell));
        function.instruction(&Instruction::Else);
        function.instruction(&Instruction::I32Const(0));
        function.instruction(&Instruction::LocalGet(cell));
        function.instruction(&Instruction::I32Load(MemArg {
            offset: u64::from(FREE_NEXT),
            align: 2,
            memory_index: 0,
        }));
        function.instruction(&Instruction::I32Store(head));
        function.instruction(&Instruction::I32Const(0));
        function.instruction(&Instruction::I32Const(0));
        function.instruction(&Instruction::I32Load(allocated));
        function.instruction(&Instruction::I32Const(CELL_SIZE.cast_signed()));
        function.instruction(&Instruction::I32Add);
        function.instruction(&Instruction::I32Store(allocated));
        // Header: the kind byte, clear GC bits, size class 0 and a zero header word.
        function.instruction(&Instruction::LocalGet(cell));
        function.instruction(&Instruction::I64Const(i64::from(kind as u8)));
        function.instruction(&Instruction::I64Store(slot_memory(0)));
        function.instruction(&Instruction::End);
        function.instruction(&Instruction::LocalGet(cell));
        function.instruction(&Instruction::LocalGet(0));
        function.instruction(&Instruction::I64Store(slot_memory(1)));
        let tag =
            (i64::from(type_kind(type_bit)) << KIND_SHIFT) | (i64::from(extra) << EXTRA_SHIFT);
        function.instruction(&Instruction::I64Const(tag));
        function.instruction(&Instruction::LocalGet(cell));
        function.instruction(&Instruction::I64ExtendI32U);
        function.instruction(&Instruction::I64Or);
        function.instruction(&Instruction::End);
        Ok(function)
    }
}

/// Byte size of the size-class cell that holds one boxed Int or Float.
const CELL_SIZE: u32 = 16;

/// Byte offset of the next-cell link of a free cell.
const FREE_NEXT: u32 = offset_of!(FreeCell, next) as u32;

// Boxed numbers are exactly one cell of the smallest size class.
const _: () = assert!(exs_layout::SIZE_CLASSES[0] == CELL_SIZE);
const _: () = assert!(core::mem::size_of::<exs_layout::FloatObject>() == CELL_SIZE as usize);
const _: () = assert!(core::mem::size_of::<exs_layout::BigIntObject>() == CELL_SIZE as usize);
