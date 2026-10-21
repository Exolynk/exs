//! Untyped Object lowering of the v2 backend: literals and per-site inline caches.
//!
//! Every access site owns an inline cache of two mutable `i32` globals: the shape it last saw
//! and that shape's value position of the accessed key. A receiver of the cached shape reads or
//! writes its value slot directly; any other receiver asks the core for the position, refills
//! the cache and takes the general path.

use core::mem::{offset_of, size_of};

use exs_abi::TYPE_OBJECT;
use exs_layout::{BufferObject, ObjectObject};
use exs_value::layout::{tag, type_kind};
use wasm_encoder::{BlockType, Instruction};

use super::collections::u32_field;
use super::function::{FunctionCompiler, error};
use super::reachability::literal_keys;
use crate::ast::ObjectProperty;
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Byte offset of [`ObjectObject::shape`].
const SHAPE: u64 = offset_of!(ObjectObject, shape) as u64;
/// Byte offset of [`ObjectObject::values`].
const VALUES: u64 = offset_of!(ObjectObject, values) as u64;
/// Byte offset of a Buffer's first slot.
pub(super) const VALUE_SLOTS: u64 = size_of::<BufferObject>() as u64;

/// The two globals of one access site's inline cache.
#[derive(Clone, Copy)]
pub(super) struct InlineCache {
    /// Global holding the cached shape address, 0 while empty.
    shape: u32,
    /// Global holding the cached value position.
    position: u32,
}

impl<'a> FunctionCompiler<'a, '_> {
    /// Allocates the globals of one access site's inline cache.
    pub(super) fn new_inline_cache(&self) -> InlineCache {
        InlineCache {
            shape: self.new_global(),
            position: self.new_global(),
        }
    }

    /// Pushes the address of the value slot `cache` names when the receiver in `object` is an
    /// untyped Object of the cached shape; otherwise branches to the enclosing block `slow`
    /// levels out.
    ///
    /// Typed Objects never carry extra bits 0 in v2, because larger type ids are rejected at
    /// compile time, so the tag alone identifies untyped Objects.
    pub(super) fn cached_slot_or_branch(&mut self, object: u32, cache: InlineCache, slow: u32) {
        self.tag_test(object, tag(type_kind(TYPE_OBJECT), 0));
        self.instruction(Instruction::I32Eqz);
        self.instruction(Instruction::BrIf(slow));
        self.push_address(object);
        self.instruction(Instruction::I32Load(u32_field(SHAPE)));
        self.instruction(Instruction::GlobalGet(cache.shape));
        self.instruction(Instruction::I32Ne);
        self.instruction(Instruction::BrIf(slow));
        self.push_address(object);
        self.instruction(Instruction::I32Load(u32_field(VALUES)));
        self.instruction(Instruction::GlobalGet(cache.position));
        self.instruction(Instruction::I32Const(3));
        self.instruction(Instruction::I32Shl);
        self.instruction(Instruction::I32Add);
    }

    /// Refills `cache` with the receiver's shape and the position of the key `name` when the
    /// receiver in `object` is an untyped Object holding that key.
    pub(super) fn fill_inline_cache(
        &mut self,
        object: u32,
        name: u64,
        cache: InlineCache,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.instruction(Instruction::LocalGet(object));
        self.instruction(Instruction::I64Const(name.cast_signed()));
        self.core_call("__exs_core_object_lookup", span)?;
        self.instruction(Instruction::LocalTee(self.scratch));
        self.instruction(Instruction::I64Const(-1));
        self.instruction(Instruction::I64Ne);
        self.instruction(Instruction::If(BlockType::Empty));
        self.instruction(Instruction::LocalGet(self.scratch));
        self.instruction(Instruction::I64Const(32));
        self.instruction(Instruction::I64ShrU);
        self.instruction(Instruction::I32WrapI64);
        self.instruction(Instruction::GlobalSet(cache.shape));
        self.instruction(Instruction::LocalGet(self.scratch));
        self.instruction(Instruction::I32WrapI64);
        self.instruction(Instruction::GlobalSet(cache.position));
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Compiles an untyped Object literal, evaluating its values left to right.
    ///
    /// A repeated key keeps the position of its first occurrence and the value of its last,
    /// like v1. The literal's shape is computed on first use and cached in a global.
    pub(super) fn compile_object(
        &mut self,
        properties: &[ObjectProperty<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let keys = literal_keys(properties);
        let count = u32::try_from(keys.len())
            .map_err(|_| error("E0212", span, "too many Object properties".to_owned()))?;
        let keys_address = if keys.is_empty() {
            0
        } else {
            self.context
                .object_keys
                .get(&keys)
                .copied()
                .ok_or_else(|| error("E0999", span, "missing v2 Object literal keys".to_owned()))?
        };
        let block = self.consecutive_locals(count.max(1));
        for property in properties {
            let position = keys
                .iter()
                .position(|key| *key == property.key)
                .unwrap_or_default() as u32;
            self.compile_expression(&property.value)?;
            self.store(block + position);
        }
        let shape = self.new_global();
        self.instruction(Instruction::GlobalGet(shape));
        self.instruction(Instruction::I32Eqz);
        self.instruction(Instruction::If(BlockType::Empty));
        self.push_vm();
        self.instruction(Instruction::I32Const(keys_address.cast_signed()));
        self.instruction(Instruction::I32Const(count.cast_signed()));
        self.core_call("__exs_core_shape_for", span)?;
        self.instruction(Instruction::GlobalSet(shape));
        self.instruction(Instruction::End);
        self.push_vm();
        self.instruction(Instruction::GlobalGet(shape));
        self.push_slot_address(block);
        self.instruction(Instruction::I32Const(count.cast_signed()));
        self.core_call("__exs_core_object_with_shape", span)
    }
}
