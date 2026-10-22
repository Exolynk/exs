//! Objects of nominal types: declared fields in slots, named by the program's type table.

use core::mem::{offset_of, size_of};

use exs_layout::TYPE_OBJECT;
use exs_layout::value::layout::{KIND_SHIFT, type_kind};
use exs_layout::{ObjectKind, TypeEntry, TypedObject};

use crate::memory::Memory;
use crate::strings::{string_object, strings_equal};
use crate::vm::Vm;

/// Byte offset of the type id in the header word.
const TYPE_ID: u32 = 4;
/// Byte offset of [`TypedObject::slot_count`].
const SLOT_COUNT: u32 = offset_of!(TypedObject, slot_count) as u32;
/// Byte offset of [`TypedObject::extra`].
const EXTRA: u32 = offset_of!(TypedObject, extra) as u32;
/// Byte offset of [`TypedObject::order`].
const ORDER: u32 = offset_of!(TypedObject, order) as u32;
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

/// Returns the field-order array of the typed Object at `object`, or 0 for declaration order.
pub(crate) fn order(memory: &impl Memory, object: u32) -> u32 {
    memory.load_u32(object + ORDER)
}

/// Sets the field-order array of the typed Object at `object`.
pub(crate) fn set_order(memory: &mut impl Memory, object: u32, order: u32) {
    memory.store_u32(object + ORDER, order);
}

/// Returns the slot index of the declared field at entry position `position` of the typed
/// Object at `object`, which lists its fields in the order of the literal that built it.
pub(crate) fn ordered_slot(memory: &impl Memory, object: u32, position: u32) -> u32 {
    match order(memory, object) {
        0 => position,
        order => memory.load_u32(order + position * 4),
    }
}

/// Returns the address of field slot `index` of the typed Object at `object`.
pub(crate) const fn slot_address(object: u32, index: u32) -> u32 {
    object + SLOTS + index * 8
}

/// Returns the program's metadata of nominal type `type_id`, or `None` when the program has
/// no entry for it. Entries are sorted by type id.
pub(crate) fn type_entry(memory: &impl Memory, vm: u32, type_id: u32) -> Option<TypeEntry> {
    let (mut low, mut high) = (0, Vm(vm).program_type_count(memory));
    while low < high {
        let middle = low + (high - low) / 2;
        let entry = type_entry_at(memory, vm, middle)?;
        match entry.type_id.cmp(&type_id) {
            core::cmp::Ordering::Equal => return Some(entry),
            core::cmp::Ordering::Less => low = middle + 1,
            core::cmp::Ordering::Greater => high = middle,
        }
    }
    None
}

/// Returns entry `index` of the program's type table, or `None` past its end.
pub(crate) fn type_entry_at(memory: &impl Memory, vm: u32, index: u32) -> Option<TypeEntry> {
    let vm = Vm(vm);
    if index >= vm.program_type_count(memory) {
        return None;
    }
    let entry = vm.program_types(memory) + index * size_of::<TypeEntry>() as u32;
    let field = |offset: usize| entry + offset as u32;
    Some(TypeEntry {
        field_count: memory.load_u32(field(offset_of!(TypeEntry, field_count))),
        field_names: memory.load_u32(field(offset_of!(TypeEntry, field_names))),
        identity: memory.load_u64(field(offset_of!(TypeEntry, identity))),
        type_id: memory.load_u32(field(offset_of!(TypeEntry, type_id))),
        kind: memory.load_u32(field(offset_of!(TypeEntry, kind))),
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

/// Allocates a typed Object of nominal type `type_id` with `slots` empty field slots and
/// returns its reference.
pub(crate) fn allocate_typed(
    memory: &mut impl Memory,
    vm: u32,
    type_id: u32,
    slots: u32,
) -> Option<u64> {
    let size = slots.checked_mul(8)?.checked_add(SLOTS)?;
    let object = crate::allocator::allocate(memory, vm, size, ObjectKind::TypedObject)?;
    memory.store_u32(object + TYPE_ID, type_id);
    memory.store_u32(object + SLOT_COUNT, slots);
    let extra = exs_layout::value::layout::nominal_extra(type_id);
    exs_layout::ValueRef::from_parts(type_kind(TYPE_OBJECT), extra, object)
        .map(exs_layout::ValueRef::raw)
}
