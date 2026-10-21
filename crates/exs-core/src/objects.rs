//! Untyped Objects: a shape naming the keys and a Buffer holding one value per key.

use core::mem::{offset_of, size_of};

use exs_abi::TYPE_OBJECT;
use exs_layout::{BufferObject, ObjectKind, ObjectObject};
use exs_value::ValueRef;
use exs_value::layout::{EXTRA_SHIFT, tag, type_kind};

use crate::allocator::allocate;
use crate::memory::Memory;
use crate::shapes::{find_key, key_count, root_shape, transition};
use crate::strings::string_object;
use crate::vm::Vm;

/// Byte offset of [`ObjectObject::shape`].
const SHAPE: u32 = offset_of!(ObjectObject, shape) as u32;
/// Byte offset of [`ObjectObject::values`].
const VALUES: u32 = offset_of!(ObjectObject, values) as u32;
/// Byte offset of a Buffer's slot count in its header word.
const CAPACITY: u32 = 4;
/// Byte offset of a Buffer's first slot.
const SLOTS: u32 = size_of::<BufferObject>() as u32;
/// Smallest value capacity of a new Object.
const MIN_CAPACITY: u32 = 4;

/// Returns the address of the untyped Object behind one raw reference, or `None` for other
/// values.
pub(crate) fn untyped_object(memory: &impl Memory, raw: u64) -> Option<u32> {
    let object = raw as u32;
    ((raw >> EXTRA_SHIFT) as u32 == tag(type_kind(TYPE_OBJECT), 0)
        && object != 0
        && memory.load_u8(object) == ObjectKind::Object as u8)
        .then_some(object)
}

/// Returns the raw reference of the untyped Object at `object`.
pub(crate) fn object_reference(object: u32) -> Option<u64> {
    ValueRef::from_parts(type_kind(TYPE_OBJECT), 0, object).map(ValueRef::raw)
}

/// Returns the shape of the untyped Object at `object`.
pub(crate) fn shape(memory: &impl Memory, object: u32) -> u32 {
    memory.load_u32(object + SHAPE)
}

/// Returns the address of value slot `index` of the untyped Object at `object`.
pub(crate) fn value_address(memory: &impl Memory, object: u32, index: u32) -> u32 {
    memory.load_u32(object + VALUES) + SLOTS + index * 8
}

/// Allocates an untyped Object of `shape` with room for `capacity` values; all of its
/// `shape`'s values are None.
///
/// The Object is not rooted when this returns.
pub(crate) fn allocate_object(
    memory: &mut impl Memory,
    vm: u32,
    shape: u32,
    capacity: u32,
) -> Option<u32> {
    let capacity = capacity.max(key_count(memory, shape)).max(MIN_CAPACITY);
    let values_size = capacity.checked_mul(8)?.checked_add(SLOTS)?;
    let object = allocate(
        memory,
        vm,
        size_of::<ObjectObject>() as u32,
        ObjectKind::Object,
    )?;
    memory.store_u32(object + SHAPE, shape);
    let root = Vm(vm).push_root(memory, object_reference(object)?)?;
    let values = allocate(memory, vm, values_size, ObjectKind::Buffer);
    Vm(vm).pop_roots(memory, root);
    let values = values?;
    memory.store_u32(values + CAPACITY, capacity);
    for index in 0..key_count(memory, shape) {
        memory.store_u64(values + SLOTS + index * 8, ValueRef::NONE.raw());
    }
    memory.store_u32(object + VALUES, values);
    Some(object)
}

/// Creates an untyped Object of `shape` holding the `count` rooted values at `slots`, one per
/// key of the shape.
pub fn object_with_shape(
    memory: &mut impl Memory,
    vm: u32,
    shape: u32,
    slots: u32,
    count: u32,
) -> Option<u64> {
    if count != key_count(memory, shape) {
        return None;
    }
    let object = allocate_object(memory, vm, shape, count)?;
    memory.copy(value_address(memory, object, 0), slots, count * 8);
    object_reference(object)
}

/// Creates an empty untyped Object with room for `capacity` values.
pub(crate) fn empty_object(memory: &mut impl Memory, vm: u32, capacity: u32) -> Option<u32> {
    let shape = root_shape(memory, vm)?;
    allocate_object(memory, vm, shape, capacity)
}

/// Returns the slot position of the key named by the String `name` in the untyped Object at
/// `object`, or `None` when it has no such key.
pub(crate) fn find_value(memory: &impl Memory, object: u32, name: u32) -> Option<u32> {
    find_key(memory, shape(memory, object), name)
}

/// Returns the value of the key named by the String `name`, or None when the Object lacks it.
pub(crate) fn object_get(memory: &impl Memory, object: u32, name: u32) -> u64 {
    find_value(memory, object, name).map_or(ValueRef::NONE.raw(), |index| {
        memory.load_u64(value_address(memory, object, index))
    })
}

/// Stores `value` under the String `name`, appending the key when the Object lacks it.
///
/// The Object, `name` and `value` must be reachable from the roots, because adding a key may
/// allocate.
pub(crate) fn object_set(
    memory: &mut impl Memory,
    vm: u32,
    object: u32,
    name: u64,
    value: u64,
) -> Option<()> {
    let text = string_object(name)?;
    if let Some(index) = find_value(memory, object, text) {
        memory.store_u64(value_address(memory, object, index), value);
        return Some(());
    }
    let current = shape(memory, object);
    let next = transition(memory, vm, current, name)?;
    let count = key_count(memory, current);
    let values = memory.load_u32(object + VALUES);
    let capacity = memory.load_u32(values + CAPACITY);
    if count == capacity {
        let grown = capacity.checked_mul(2)?.max(MIN_CAPACITY);
        let size = grown.checked_mul(8)?.checked_add(SLOTS)?;
        let replacement = allocate(memory, vm, size, ObjectKind::Buffer)?;
        memory.store_u32(replacement + CAPACITY, grown);
        // The old Buffer stays reachable through the Object until it is replaced here.
        memory.copy(replacement + SLOTS, values + SLOTS, count * 8);
        memory.store_u32(object + VALUES, replacement);
    }
    memory.store_u64(value_address(memory, object, count), value);
    memory.store_u32(object + SHAPE, next);
    Some(())
}

/// Returns the shape of the untyped Object `raw` and the slot position of the key named by the
/// String `name`, packed as `shape << 32 | position`, or `None` when either is missing.
///
/// Generated code caches the result per access site.
#[must_use]
pub fn object_lookup(memory: &impl Memory, raw: u64, name: u64) -> Option<u64> {
    let object = untyped_object(memory, raw)?;
    let position = find_value(memory, object, string_object(name)?)?;
    Some(u64::from(shape(memory, object)) << 32 | u64::from(position))
}
