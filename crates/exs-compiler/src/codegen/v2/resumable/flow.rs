//! Control flow of resumable code: branches, jumps, `for` loop iteration and returns.

use core::mem::{offset_of, size_of};

use exs_abi::{BuiltinMethod, TYPE_LIST};
use exs_layout::{BufferObject, EnumObject, ListObject};
use exs_value::ValueRef;
use wasm_encoder::{BlockType, Instruction, ValType};

use super::step::StepEmitter;
use crate::codegen::continuation::Operation;
use crate::codegen::v2::collections::{u32_field, value_field};
use crate::codegen::v2::function::error;
use crate::codegen::{VALUE_TYPE, immediate};
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Byte offset of the element count of a List.
const LEN: u64 = offset_of!(ListObject, len) as u64;
/// Byte offset of the element Buffer of a List.
const ITEMS: u64 = offset_of!(ListObject, items) as u64;
/// Byte offset of the first slot of a Buffer.
const SLOTS: u64 = size_of::<BufferObject>() as u64;
/// Byte offset of the variant index of an enum.
const VARIANT: u64 = offset_of!(EnumObject, variant) as u64;
/// Byte offset of the payload field count of an enum.
const FIELD_COUNT: u64 = offset_of!(EnumObject, field_count) as u64;
/// Byte offset of the first payload field of an enum.
const FIELDS: u64 = size_of::<EnumObject>() as u64;

impl<'a> StepEmitter<'a, '_> {
    /// Emits `operation` when it transfers control, ending in a state transition or a return.
    /// Returns whether it did.
    pub(super) fn flow_operation(
        &mut self,
        state: u32,
        operation: &Operation<'a, '_>,
    ) -> Result<bool, CompileDiagnostics<'a>> {
        let next = state + 1;
        match operation {
            Operation::Branch {
                condition,
                when_true,
                when_false,
                span,
                ..
            } => {
                self.get_slot(*condition);
                self.compiler.condition(*span)?;
                self.instruction(Instruction::If(BlockType::Empty));
                self.ready(*when_true, *span)?;
                self.instruction(Instruction::Else);
                self.ready(*when_false, *span)?;
                self.instruction(Instruction::End);
            }
            Operation::Goto { target, span, .. } => self.ready(*target, *span)?,
            Operation::FormatValue {
                builder,
                value,
                type_ids,
                skip,
                span,
            } => {
                // Values of a type with a `ToString` implementation continue with its call.
                if !type_ids.is_empty() {
                    let local = self.load(*value);
                    for (index, type_id) in type_ids.iter().enumerate() {
                        self.instruction(Instruction::LocalGet(local));
                        self.compiler.type_tag_test(*type_id, *span)?;
                        if index > 0 {
                            self.instruction(Instruction::I32Or);
                        }
                    }
                    self.instruction(Instruction::If(BlockType::Empty));
                    self.ready(next, *span)?;
                    self.instruction(Instruction::End);
                }
                self.append_slot(*builder, *value, *span)?;
                self.ready(*skip, *span)?;
            }
            Operation::IterSnapshot {
                iterable,
                destination,
                span,
                ..
            } => self.snapshot(next, *iterable, *destination, *span)?,
            Operation::NativeIterNext {
                snapshot,
                item,
                range_type_id,
                when_item,
                when_done,
                span,
            } => {
                let targets = Targets {
                    next,
                    item: *when_item,
                    done: *when_done,
                };
                self.native_next(*snapshot, *item, *range_type_id, targets, *span)?;
            }
            Operation::IteratorBranch {
                step,
                item,
                when_item,
                when_done,
                span,
                ..
            } => {
                let targets = Targets {
                    next,
                    item: *when_item,
                    done: *when_done,
                };
                self.iterator_branch(*step, *item, targets, *span)?;
            }
            Operation::Return { value, span } => {
                self.get_slot(*value);
                self.compiler.return_value(*span)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Stores the snapshot `for` iterates of the value in durable slot `iterable`, like
    /// synchronous code: a List snapshot is reversed, so the loop pops its items in order.
    fn snapshot(
        &mut self,
        next: u32,
        iterable: u32,
        destination: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let source = self.load(iterable);
        self.compiler.set_position(span)?;
        self.compiler.push_vm();
        self.instruction(Instruction::LocalGet(source));
        self.compiler.core_call("__exs_core_for_snapshot", span)?;
        let snapshot = self.compiler.store_new();
        self.compiler.return_if_error(snapshot, span)?;
        self.instruction(Instruction::LocalGet(snapshot));
        immediate::type_mask_test(&mut self.compiler.function, TYPE_LIST);
        self.instruction(Instruction::If(BlockType::Empty));
        self.compiler.push_vm();
        self.instruction(Instruction::I32Const(
            BuiltinMethod::Reverse.id().cast_signed(),
        ));
        self.instruction(Instruction::LocalGet(snapshot));
        immediate::push(&mut self.compiler.function, ValueRef::NONE);
        immediate::push(&mut self.compiler.function, ValueRef::NONE);
        self.compiler.core_call("__exs_core_call_builtin", span)?;
        self.instruction(Instruction::Drop);
        self.instruction(Instruction::End);
        self.instruction(Instruction::LocalGet(snapshot));
        self.set_slot(destination);
        self.ready(next, span)
    }

    /// Advances the snapshot in durable slot `snapshot` natively: a List pops its next item,
    /// a well-formed prelude Range steps, and any other value continues at the
    /// `Iterator::next` dispatch.
    fn native_next(
        &mut self,
        snapshot: u32,
        item: u32,
        range_type_id: i32,
        targets: Targets,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let list = self.load(snapshot);
        self.instruction(Instruction::LocalGet(list));
        immediate::type_mask_test(&mut self.compiler.function, TYPE_LIST);
        self.instruction(Instruction::If(BlockType::Empty));
        let len = self.compiler.scalar_local();
        self.compiler.push_address(list);
        self.instruction(Instruction::I32Load(u32_field(LEN)));
        self.instruction(Instruction::I64ExtendI32U);
        self.instruction(Instruction::LocalTee(len));
        self.instruction(Instruction::I64Eqz);
        self.instruction(Instruction::If(BlockType::Empty));
        self.ready(targets.done, span)?;
        self.instruction(Instruction::End);
        // The last item of the reversed snapshot is the next one.
        self.compiler.push_address(list);
        self.instruction(Instruction::LocalGet(len));
        self.instruction(Instruction::I64Const(1));
        self.instruction(Instruction::I64Sub);
        self.instruction(Instruction::LocalTee(len));
        self.instruction(Instruction::I32WrapI64);
        self.instruction(Instruction::I32Store(u32_field(LEN)));
        self.compiler.push_address(list);
        self.instruction(Instruction::I32Load(u32_field(ITEMS)));
        self.instruction(Instruction::LocalGet(len));
        self.instruction(Instruction::I32WrapI64);
        self.instruction(Instruction::I32Const(3));
        self.instruction(Instruction::I32Shl);
        self.instruction(Instruction::I32Add);
        self.instruction(Instruction::I64Load(value_field(SLOTS)));
        self.set_slot(item);
        self.ready(targets.item, span)?;
        self.instruction(Instruction::End);
        if range_type_id >= 0 {
            let native = self.compiler.scalar_local();
            self.instruction(Instruction::LocalGet(list));
            self.instruction(Instruction::I32Const(range_type_id));
            self.compiler.core_call("__exs_core_range_step", span)?;
            self.instruction(Instruction::LocalTee(native));
            self.instruction(Instruction::I64Eqz);
            self.instruction(Instruction::If(BlockType::Empty));
            self.ready(targets.done, span)?;
            self.instruction(Instruction::End);
            self.instruction(Instruction::LocalGet(native));
            self.instruction(Instruction::I64Const(-1));
            self.instruction(Instruction::I64Ne);
            self.instruction(Instruction::If(BlockType::Empty));
            self.instruction(Instruction::LocalGet(native));
            self.set_slot(item);
            self.ready(targets.item, span)?;
            self.instruction(Instruction::End);
        }
        self.ready(targets.next, span)
    }

    /// Continues with the item of the `IteratorStep` in durable slot `step`, or leaves the
    /// loop on `Done`, like synchronous code: an Error returns, and a value that is no
    /// `IteratorStep` returns its MatchError.
    fn iterator_branch(
        &mut self,
        step: u32,
        item: u32,
        targets: Targets,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let context = self.compiler.context;
        let step_type = |name: &str| {
            context
                .types
                .enum_variant(name)
                .map(|variant| (variant.type_id, variant.index))
                .ok_or_else(|| error("E0999", span, format!("missing prelude `{name}`")))
        };
        let (step_type_id, done_index) = step_type("IteratorStep::Done")?;
        let (_, item_index) = step_type("IteratorStep::Item")?;
        let value = self.load(step);
        self.compiler.return_if_error(value, span)?;
        for (index, done) in [(done_index, true), (item_index, false)] {
            self.instruction(Instruction::LocalGet(value));
            self.compiler.type_tag_test(step_type_id, span)?;
            self.instruction(Instruction::If(BlockType::Result(ValType::I32)));
            self.compiler.push_address(value);
            self.instruction(Instruction::I32Load(u32_field(VARIANT)));
            self.instruction(Instruction::I32Const(index.cast_signed()));
            self.instruction(Instruction::I32Eq);
            self.instruction(Instruction::Else);
            self.instruction(Instruction::I32Const(0));
            self.instruction(Instruction::End);
            self.instruction(Instruction::If(BlockType::Empty));
            if done {
                self.ready(targets.done, span)?;
            } else {
                self.compiler.push_address(value);
                self.instruction(Instruction::I32Load(u32_field(FIELD_COUNT)));
                self.instruction(Instruction::If(BlockType::Result(VALUE_TYPE)));
                self.compiler.push_address(value);
                self.instruction(Instruction::I64Load(value_field(FIELDS)));
                self.instruction(Instruction::Else);
                self.compiler.set_position(span)?;
                self.compiler.push_vm();
                self.instruction(Instruction::LocalGet(value));
                self.compiler
                    .core_call("__exs_core_enum_missing_field", span)?;
                self.instruction(Instruction::End);
                self.set_slot(item);
                self.ready(targets.item, span)?;
            }
            self.instruction(Instruction::End);
        }
        self.compiler.set_position(span)?;
        self.compiler.push_vm();
        self.instruction(Instruction::LocalGet(value));
        self.compiler.core_call("__exs_core_match_error", span)?;
        let mismatch = self.compiler.store_new();
        self.compiler.return_if_error(mismatch, span)?;
        self.ready(targets.done, span)
    }
}

/// The states a `for` loop step continues at.
#[derive(Clone, Copy)]
struct Targets {
    /// The state following the operation.
    next: u32,
    /// The state entering the loop body with the next item.
    item: u32,
    /// The state after the loop.
    done: u32,
}
