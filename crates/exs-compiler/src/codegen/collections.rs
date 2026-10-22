//! List lowering: literals, inline element access and `for` over Lists.

use core::mem::{offset_of, size_of};

use exs_layout::TYPE_LIST;
use exs_layout::value::layout::{EXTRA_SHIFT, SMALL_INT_TAG, tag, type_kind};
use exs_layout::{BufferObject, ListObject};
use wasm_encoder::{BlockType, Instruction, MemArg};

use super::function::{FunctionCompiler, error};
use crate::ast::Expression;
use crate::codegen::VALUE_TYPE;
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Byte offset of [`ListObject::len`].
const LEN: u64 = offset_of!(ListObject, len) as u64;
/// Byte offset of [`ListObject::items`].
const ITEMS: u64 = offset_of!(ListObject, items) as u64;
/// Byte offset of a Buffer's first slot.
const SLOTS: u64 = size_of::<BufferObject>() as u64;

/// Returns the memory operand of one `u32` field at `offset`.
pub(super) const fn u32_field(offset: u64) -> MemArg {
    MemArg {
        offset,
        align: 2,
        memory_index: 0,
    }
}

/// Returns the memory operand of one `ValueRef` slot at `offset`.
pub(super) const fn value_field(offset: u64) -> MemArg {
    MemArg {
        offset,
        align: 3,
        memory_index: 0,
    }
}

impl<'a> FunctionCompiler<'a, '_> {
    /// Pushes 1 when the `ValueRef` in `local` has the upper 32 bits `expected`, else 0.
    pub(super) fn tag_test(&mut self, local: u32, expected: u32) {
        self.instruction(Instruction::LocalGet(local));
        self.instruction(Instruction::I64Const(i64::from(EXTRA_SHIFT)));
        self.instruction(Instruction::I64ShrU);
        self.instruction(Instruction::I64Const(i64::from(expected)));
        self.instruction(Instruction::I64Eq);
    }

    /// Pushes the heap address of the `ValueRef` in `local`.
    pub(super) fn push_address(&mut self, local: u32) {
        self.instruction(Instruction::LocalGet(local));
        self.instruction(Instruction::I32WrapI64);
    }

    /// Compiles a List literal, evaluating its elements left to right.
    pub(super) fn compile_list(
        &mut self,
        elements: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let count = u32::try_from(elements.len())
            .map_err(|_| error("E0212", span, "too many List elements".to_owned()))?;
        let block = self.consecutive_locals(count.max(1));
        for (offset, element) in (0..count).zip(elements) {
            self.compile_expression(element)?;
            self.store(block + offset);
        }
        self.push_vm();
        self.push_slot_address(block);
        self.instruction(Instruction::I32Const(count.cast_signed()));
        self.core_call("__exs_core_list_new", span)
    }

    /// Pushes the address of element `index` of the List in `list` when `list` is a List and
    /// `index` an immediate Int inside its bounds; otherwise branches to the enclosing block
    /// `slow` levels out.
    fn list_slot_or_branch(&mut self, list: u32, index: u32, slow: u32) {
        self.tag_test(list, tag(type_kind(TYPE_LIST), 0));
        self.tag_test(index, SMALL_INT_TAG);
        self.instruction(Instruction::I32And);
        self.instruction(Instruction::I32Eqz);
        self.instruction(Instruction::BrIf(slow));
        // An unsigned comparison also rejects negative indices.
        self.push_address(index);
        self.push_address(list);
        self.instruction(Instruction::I32Load(u32_field(LEN)));
        self.instruction(Instruction::I32GeU);
        self.instruction(Instruction::BrIf(slow));
        self.push_address(list);
        self.instruction(Instruction::I32Load(u32_field(ITEMS)));
        self.push_address(index);
        self.instruction(Instruction::I32Const(3));
        self.instruction(Instruction::I32Shl);
        self.instruction(Instruction::I32Add);
    }

    /// Compiles `receiver[index]` with an inline path for Lists.
    pub(super) fn compile_index_get(
        &mut self,
        receiver: &Expression<'a>,
        index: &Expression<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(receiver)?;
        let receiver = self.store_new();
        self.compile_expression(index)?;
        let index = self.store_new();
        self.instruction(Instruction::Block(BlockType::Result(VALUE_TYPE)));
        self.instruction(Instruction::Block(BlockType::Empty));
        self.list_slot_or_branch(receiver, index, 0);
        self.instruction(Instruction::I64Load(value_field(SLOTS)));
        self.instruction(Instruction::Br(1));
        self.instruction(Instruction::End);
        self.set_position(span)?;
        self.push_vm();
        self.instruction(Instruction::LocalGet(receiver));
        self.instruction(Instruction::LocalGet(index));
        self.core_call("__exs_core_index_get", span)?;
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Compiles `receiver[index] = value` with an inline path for Lists.
    pub(super) fn compile_index_set(
        &mut self,
        receiver: &Expression<'a>,
        index: &Expression<'a>,
        value: &Expression<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(receiver)?;
        let receiver = self.store_new();
        self.compile_expression(index)?;
        let index = self.store_new();
        self.compile_expression(value)?;
        let value = self.store_new();
        self.instruction(Instruction::Block(BlockType::Empty));
        self.instruction(Instruction::Block(BlockType::Empty));
        self.list_slot_or_branch(receiver, index, 0);
        self.instruction(Instruction::LocalGet(value));
        self.instruction(Instruction::I64Store(value_field(SLOTS)));
        self.instruction(Instruction::Br(1));
        self.instruction(Instruction::End);
        // Like v1, the assignment's result, also an Error, is discarded.
        self.set_position(span)?;
        self.push_vm();
        self.instruction(Instruction::LocalGet(receiver));
        self.instruction(Instruction::LocalGet(index));
        self.instruction(Instruction::LocalGet(value));
        self.core_call("__exs_core_index_set", span)?;
        self.instruction(Instruction::Drop);
        self.instruction(Instruction::End);
        Ok(())
    }
}
