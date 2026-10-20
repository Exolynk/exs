//! Typed Object lowering of the v2 backend: literals and inline property access.
//!
//! A typed Object keeps its declared fields in slots in declaration order and carries its type
//! id in its reference's extra bits. Property access tests the receiver's tag against every
//! type declaring the property and reads that type's slot inline; any other receiver goes
//! through the core library.

use core::mem::size_of;

use exs_layout::{ObjectKind, TypedObject};
use exs_value::ValueRef;
use exs_value::layout::object_tag;
use wasm_encoder::{BlockType, Instruction};

use super::collections::{u32_field, value_field};
use super::function::{FunctionCompiler, error};
use crate::ast::{Expression, Identifier, ObjectProperty};
use crate::codegen::types::NominalKind;
use crate::codegen::{VALUE_TYPE, immediate};
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Byte offset of the type id in the header word.
const TYPE_ID: u64 = 4;
/// Byte offset of [`TypedObject::slot_count`].
const SLOT_COUNT: u64 = core::mem::offset_of!(TypedObject, slot_count) as u64;
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
                format!("unknown type `{}`", type_name.name),
            )
        })?;
        if nominal.kind != NominalKind::Object {
            return Err(error(
                "E0216",
                type_name.span,
                format!("enum `{}` requires a variant constructor", type_name.name),
            ));
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
                // v1 reports a TypeError for undeclared and repeated properties; v2 traps.
                _ => {
                    self.instruction(Instruction::Unreachable);
                    return Ok(());
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
        let count = u32::try_from(slots.len())
            .map_err(|_| error("E0212", span, "too many fields for one nominal type".into()))?;
        let tag = object_tag(nominal.id)
            .ok_or_else(|| super::function::unsupported(span, "more than 65534 nominal types"))?;
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
        for (offset, value) in [(TYPE_ID, nominal.id), (SLOT_COUNT, count)] {
            self.push_address(object);
            self.instruction(Instruction::I32Const(value.cast_signed()));
            self.instruction(Instruction::I32Store(u32_field(offset)));
        }
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

    /// Returns the inline tag and slot offset of every nominal Object type declaring
    /// `property`.
    fn property_slots(&self, property: &str) -> Vec<(u32, u64)> {
        let mut slots: Vec<(u32, u64)> = self
            .context
            .types
            .nominal_types()
            .filter(|nominal| nominal.kind == NominalKind::Object)
            .filter_map(|nominal| {
                let index = nominal
                    .fields
                    .iter()
                    .position(|field| field.name == property)?;
                Some((object_tag(nominal.id)?, SLOTS + index as u64 * 8))
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
        self.instruction(Instruction::Block(BlockType::Result(VALUE_TYPE)));
        self.instruction(Instruction::Block(BlockType::Empty));
        self.property_dispatch(object, property, 1, |compiler, offset| {
            compiler.push_address(object);
            compiler.instruction(Instruction::I64Load(value_field(offset)));
        });
        self.instruction(Instruction::End);
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
        self.instruction(Instruction::Block(BlockType::Empty));
        self.instruction(Instruction::Block(BlockType::Empty));
        self.property_dispatch(object, property, 1, |compiler, offset| {
            compiler.push_address(object);
            compiler.instruction(Instruction::LocalGet(value));
            compiler.instruction(Instruction::I64Store(value_field(offset)));
        });
        self.instruction(Instruction::End);
        self.push_vm();
        self.instruction(Instruction::LocalGet(object));
        self.instruction(Instruction::I64Const(name.cast_signed()));
        self.instruction(Instruction::LocalGet(value));
        self.core_call("__exs_core_property_set", span)?;
        self.instruction(Instruction::End);
        Ok(())
    }
}
