//! Typed Object lowering: literals and inline property access.
//!
//! A typed Object keeps its declared fields in slots in declaration order and carries its type
//! id in its reference's extra bits. Property access tests the receiver's tag against every
//! type declaring the property and reads that type's slot inline, then tries the site's inline
//! cache for untyped Objects; any other receiver goes through the core library.
//!
//! A literal lists its properties in source order, then the fields it omits in
//! declaration order. A literal whose order differs from the declaration points its Object at
//! a static array of slot indices in that order.

use core::mem::size_of;

use exs_layout::TYPE_OBJECT;
use exs_layout::ValueRef;
use exs_layout::value::layout::{nominal_extra, nominal_tag, tag, type_kind};
use exs_layout::{ObjectKind, TypedObject};
use wasm_encoder::{BlockType, Instruction};

use super::collections::{u32_field, value_field};
use super::function::{FunctionCompiler, error};
use super::untyped::VALUE_SLOTS;
use crate::ast::{Expression, Identifier, ObjectProperty};
use crate::codegen::types::{NominalField, NominalKind, NominalType};
use crate::codegen::{VALUE_TYPE, immediate};
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Byte offset of the type id in the header word.
const TYPE_ID: u64 = 4;
/// Byte offset of [`TypedObject::slot_count`].
const SLOT_COUNT: u64 = core::mem::offset_of!(TypedObject, slot_count) as u64;
/// Byte offset of [`TypedObject::order`].
const ORDER: u64 = core::mem::offset_of!(TypedObject, order) as u64;
/// Byte offset of the first field slot.
const SLOTS: u64 = size_of::<TypedObject>() as u64;

impl<'a> FunctionCompiler<'a, '_> {
    /// Compiles one nominal Object construction, checking every declared field's contract.
    ///
    /// Properties are evaluated in source order; undeclared or repeated properties trap, and
    /// fields without a property hold None.
    pub(super) fn compile_typed_object(
        &mut self,
        type_name: &Identifier<'a>,
        properties: &[ObjectProperty<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let types = self.context.types;
        let nominal = types.get(&type_name.name).ok_or_else(|| {
            error(
                "E0216",
                type_name.span,
                crate::diagnostic::unknown_name("type", &type_name.name),
            )
        })?;
        if nominal.kind != NominalKind::Object {
            return Err(error(
                "E0216",
                type_name.span,
                format!("enum `{}` requires a variant constructor", type_name.name),
            ));
        }
        // The first undeclared or repeated property returns a TypeError before any
        // property is evaluated.
        let invalid = properties.iter().find(|property| {
            !nominal
                .fields
                .iter()
                .any(|field| field.name == property.key)
                || properties
                    .iter()
                    .filter(|other| other.key == property.key)
                    .count()
                    > 1
        });
        if let Some(property) = invalid {
            immediate::push(&mut self.function, ValueRef::NONE);
            self.instruction(Instruction::LocalSet(self.scratch));
            return self.return_type_mismatch(self.scratch, property.key_span);
        }
        let mut slots = vec![None; nominal.fields.len()];
        for property in properties {
            let field = nominal
                .fields
                .iter()
                .position(|field| field.name == property.key);
            match field.and_then(|field| slots.get_mut(field).map(|slot| (field, slot))) {
                Some((field, slot)) if slot.is_none() => {
                    self.compile_expression(&property.value)?;
                    let value = self.store_new();
                    let contract = &nominal.fields[field].contract;
                    self.check_contract(value, contract, property.span)?;
                    *slot = Some(value);
                }
                _ => {
                    return Err(error(
                        "E0999",
                        property.key_span,
                        "unchecked typed Object property".to_owned(),
                    ));
                }
            }
        }
        for (field, slot) in nominal.fields.iter().zip(&slots) {
            if slot.is_none() {
                immediate::push(&mut self.function, ValueRef::NONE);
                self.instruction(Instruction::LocalSet(self.scratch));
                self.check_contract(self.scratch, &field.contract, span)?;
            }
        }
        let keys: Vec<&str> = properties
            .iter()
            .map(|property| property.key.as_str())
            .collect();
        let object = self.allocate_typed_object(nominal, &keys, span)?;
        for (index, slot) in (0_u64..).zip(&slots) {
            self.push_address(object);
            match slot {
                Some(value) => self.instruction(Instruction::LocalGet(*value)),
                None => immediate::push(&mut self.function, ValueRef::NONE),
            }
            self.instruction(Instruction::I64Store(value_field(SLOTS + index * 8)));
        }
        self.instruction(Instruction::LocalGet(object));
        Ok(())
    }

    /// Allocates a typed Object of the nominal type `nominal`, built by a literal naming `keys`
    /// in source order, and returns the rooted local holding it. Its slots are empty.
    pub(super) fn allocate_typed_object(
        &mut self,
        nominal: &NominalType,
        keys: &[&str],
        span: SourceSpan<'a>,
    ) -> Result<u32, CompileDiagnostics<'a>> {
        let count = u32::try_from(nominal.fields.len())
            .map_err(|_| error("E0212", span, "too many fields for one nominal type".into()))?;
        let tag = tag(type_kind(TYPE_OBJECT), nominal_extra(nominal.id));
        self.push_vm();
        self.instruction(Instruction::I32Const(
            (SLOTS as u32 + count * 8).cast_signed(),
        ));
        self.instruction(Instruction::I32Const(i32::from(
            ObjectKind::TypedObject as u8,
        )));
        self.core_call("__exs_core_allocate", span)?;
        self.instruction(Instruction::I64ExtendI32U);
        self.instruction(Instruction::I64Const(i64::from(tag) << 32));
        self.instruction(Instruction::I64Or);
        let object = self.store_new();
        let order = self.field_order(keys, &nominal.fields);
        let header = [(TYPE_ID, nominal.id), (SLOT_COUNT, count)]
            .into_iter()
            .chain(order.map(|order| (ORDER, order)));
        for (offset, value) in header {
            self.push_address(object);
            self.instruction(Instruction::I32Const(value.cast_signed()));
            self.instruction(Instruction::I32Store(u32_field(offset)));
        }
        Ok(object)
    }

    /// Returns the address of the static field-order array of a literal naming `keys` of a
    /// type declaring `fields`, or `None` when the literal keeps the declaration order.
    fn field_order(&self, keys: &[&str], fields: &[NominalField]) -> Option<u32> {
        let position = |name: &str| fields.iter().position(|field| field.name == name);
        let mut order: Vec<u32> = keys
            .iter()
            .filter_map(|key| position(key))
            .map(|index| index as u32)
            .collect();
        for (index, field) in (0_u32..).zip(fields) {
            if !keys.contains(&field.name.as_str()) {
                order.push(index);
            }
        }
        if order.iter().copied().eq(0..order.len() as u32) {
            return None;
        }
        Some(self.static_array(order))
    }

    /// Returns the inline tag and slot offset of every nominal Object type the program uses
    /// that declares `property`.
    fn property_slots(&self, property: &str) -> Vec<(u32, u64)> {
        let mut slots: Vec<(u32, u64)> = self
            .context
            .types
            .nominal_types()
            // Types no value of the program can have need no test.
            .filter(|nominal| {
                nominal.kind == NominalKind::Object && self.context.used_types.contains(&nominal.id)
            })
            .filter_map(|nominal| {
                let index = nominal
                    .fields
                    .iter()
                    .position(|field| field.name == property)?;
                Some((nominal_tag(nominal.id)?, SLOTS + index as u64 * 8))
            })
            .collect();
        // A stable order keeps the generated code deterministic.
        slots.sort_unstable();
        slots
    }

    /// Emits one inline test per type declaring `property`: when the receiver in `object` has
    /// that type, `access` runs with the slot offset and control leaves the enclosing block
    /// `exit` levels out of the test.
    fn property_dispatch(
        &mut self,
        object: u32,
        property: &str,
        exit: u32,
        mut access: impl FnMut(&mut Self, u64),
    ) {
        for (tag, offset) in self.property_slots(property) {
            self.tag_test(object, tag);
            self.instruction(Instruction::If(BlockType::Empty));
            access(self, offset);
            self.instruction(Instruction::Br(exit + 1));
            self.instruction(Instruction::End);
        }
    }

    /// Compiles `receiver.property`.
    pub(super) fn compile_property_get(
        &mut self,
        receiver: &Expression<'a>,
        property: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(receiver)?;
        let object = self.store_new();
        let name = self.static_string_value(property, span)?;
        let cache = self.new_inline_cache();
        self.instruction(Instruction::Block(BlockType::Result(VALUE_TYPE)));
        self.instruction(Instruction::Block(BlockType::Empty));
        self.property_dispatch(object, property, 1, |compiler, offset| {
            // A declared field removed by `delete` or `clear` holds an empty slot: None.
            let scratch = compiler.scratch;
            compiler.push_address(object);
            compiler.instruction(Instruction::I64Load(value_field(offset)));
            compiler.instruction(Instruction::LocalSet(scratch));
            immediate::push(&mut compiler.function, ValueRef::NONE);
            compiler.instruction(Instruction::LocalGet(scratch));
            compiler.instruction(Instruction::LocalGet(scratch));
            compiler.instruction(Instruction::I64Eqz);
            compiler.instruction(Instruction::Select);
        });
        self.cached_slot_or_branch(object, cache, 0);
        self.instruction(Instruction::I64Load(value_field(VALUE_SLOTS)));
        self.instruction(Instruction::Br(1));
        self.instruction(Instruction::End);
        self.fill_inline_cache(object, name, cache, span)?;
        self.set_position(span)?;
        self.push_vm();
        self.instruction(Instruction::LocalGet(object));
        self.instruction(Instruction::I64Const(name.cast_signed()));
        self.core_call("__exs_core_property_get", span)?;
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Compiles `receiver.property = value`.
    pub(super) fn compile_property_set(
        &mut self,
        receiver: &Expression<'a>,
        property: &str,
        value: &Expression<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(receiver)?;
        let object = self.store_new();
        let name = self.static_string_value(property, span)?;
        self.compile_expression(value)?;
        let value = self.store_new();
        let cache = self.new_inline_cache();
        self.instruction(Instruction::Block(BlockType::Empty));
        self.instruction(Instruction::Block(BlockType::Empty));
        self.property_dispatch(object, property, 1, |compiler, offset| {
            compiler.push_address(object);
            compiler.instruction(Instruction::LocalGet(value));
            compiler.instruction(Instruction::I64Store(value_field(offset)));
        });
        self.cached_slot_or_branch(object, cache, 0);
        self.instruction(Instruction::LocalGet(value));
        self.instruction(Instruction::I64Store(value_field(VALUE_SLOTS)));
        self.instruction(Instruction::Br(1));
        self.instruction(Instruction::End);
        self.fill_inline_cache(object, name, cache, span)?;
        self.set_position(span)?;
        self.push_vm();
        self.instruction(Instruction::LocalGet(object));
        self.instruction(Instruction::I64Const(name.cast_signed()));
        self.instruction(Instruction::LocalGet(value));
        self.core_call("__exs_core_property_set", span)?;
        self.return_if_assignment_failed(value, span)?;
        self.instruction(Instruction::End);
        Ok(())
    }
}
