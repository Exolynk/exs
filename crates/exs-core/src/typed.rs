//! Objects of nominal types: declared fields in slots, named by the program's type table.

use core::mem::{offset_of, size_of};

use exs_abi::TYPE_OBJECT;
use exs_layout::{ObjectKind, TypeEntry, TypedObject};
use exs_value::layout::{KIND_SHIFT, type_kind};

use crate::memory::Memory;
use crate::strings::{string_object, strings_equal};
use crate::vm::Vm;

/// Byte offset of the type id in the header word.
const TYPE_ID: u32 = 4;
/// Byte offset of [`TypedObject::slot_count`].
const SLOT_COUNT: u32 = offset_of!(TypedObject, slot_count) as u32;
/// Byte offset of [`TypedObject::extra`].
const EXTRA: u32 = offset_of!(TypedObject, extra) as u32;
/// Byte offset of the first field slot.
const SLOTS: u32 = size_of::<TypedObject>() as u32;

/// Returns the address of the typed Object behind one raw reference, or `None` for other
/// values.
pub(crate) fn typed_object(memory: &impl Memory, raw: u64) -> Option<u32> {
    let object = raw as u32;
    (raw >> KIND_SHIFT == u64::from(type_kind(TYPE_OBJECT))
        && object != 0
        && memory.load_u8(object) == ObjectKind::TypedObject as u8)
        .then_some(object)
}

/// Returns the nominal type id of the typed Object at `object`.
pub(crate) fn type_id(memory: &impl Memory, object: u32) -> u32 {
    memory.load_u32(object + TYPE_ID)
}

/// Returns the number of field slots of the typed Object at `object`.
pub(crate) fn slot_count(memory: &impl Memory, object: u32) -> u32 {
    memory.load_u32(object + SLOT_COUNT)
}

/// Returns the untyped Object holding the undeclared keys of the typed Object at `object`, or
/// 0 when it has none.
pub(crate) fn extra(memory: &impl Memory, object: u32) -> u32 {
    memory.load_u32(object + EXTRA)
}

/// Sets the untyped Object holding the undeclared keys of the typed Object at `object`.
pub(crate) fn set_extra(memory: &mut impl Memory, object: u32, extra: u32) {
    memory.store_u32(object + EXTRA, extra);
}

/// Returns the address of field slot `index` of the typed Object at `object`.
pub(crate) const fn slot_address(object: u32, index: u32) -> u32 {
    object + SLOTS + index * 8
}

/// Returns the program's metadata of nominal type `type_id`, or `None` when the program has
/// no entry for it.
pub(crate) fn type_entry(memory: &impl Memory, vm: u32, type_id: u32) -> Option<TypeEntry> {
    let vm = Vm(vm);
    if type_id >= vm.program_type_count(memory) {
        return None;
    }
    let entry = vm.program_types(memory) + type_id * size_of::<TypeEntry>() as u32;
    Some(TypeEntry {
        field_count: memory.load_u32(entry + offset_of!(TypeEntry, field_count) as u32),
        field_names: memory.load_u32(entry + offset_of!(TypeEntry, field_names) as u32),
    })
}

/// Returns the slot address of the field named by the String `name` in the typed Object at
/// `object`, or `None` when its type declares no such field.
pub(crate) fn field_address(memory: &impl Memory, vm: u32, object: u32, name: u64) -> Option<u32> {
    let name = string_object(name)?;
    let entry = type_entry(memory, vm, type_id(memory, object))?;
    let count = entry.field_count.min(slot_count(memory, object));
    (0..count)
        .find(|index| {
            string_object(memory.load_u64(entry.field_names + index * 8))
                .is_some_and(|field| strings_equal(memory, field, name))
        })
        .map(|index| slot_address(object, index))
}
