//! Lists: a List object over one Buffer whose slot count is the List's capacity.
//!
//! Generated code reads and writes elements inline; this module creates Lists and implements
//! the operations that allocate.

use core::mem::{offset_of, size_of};

use exs_abi::TYPE_LIST;
use exs_layout::{BufferObject, ListObject, ObjectKind};
use exs_value::ValueRef;
use exs_value::layout::{EXTRA_SHIFT, tag, type_kind};

use crate::allocator::allocate;
use crate::memory::Memory;
use crate::vm::Vm;

/// Byte offset of [`ListObject::len`].
const LEN: u32 = offset_of!(ListObject, len) as u32;
/// Byte offset of [`ListObject::items`].
const ITEMS: u32 = offset_of!(ListObject, items) as u32;
/// Byte offset of a Buffer's slot count in its header word.
const CAPACITY: u32 = 4;
/// Byte offset of a Buffer's first slot.
const SLOTS: u32 = size_of::<BufferObject>() as u32;
/// Smallest capacity of a new List, so short Lists can grow without reallocating at once.
const MIN_CAPACITY: u32 = 4;

/// Returns the address of the List behind one raw reference, or `None` for other values.
#[must_use]
pub fn list_object(raw: u64) -> Option<u32> {
    ((raw >> EXTRA_SHIFT) as u32 == tag(type_kind(TYPE_LIST), 0)).then_some(raw as u32)
}

/// Returns the raw reference of the List at `object`.
pub(crate) fn list_reference(object: u32) -> Option<u64> {
    ValueRef::from_parts(type_kind(TYPE_LIST), 0, object).map(ValueRef::raw)
}

/// Returns the number of elements of the List at `list`.
pub(crate) fn len(memory: &impl Memory, list: u32) -> u32 {
    memory.load_u32(list + LEN)
}

/// Returns the address of element `index` of the List at `list`, which must be in bounds.
pub(crate) fn item_address(memory: &impl Memory, list: u32, index: u32) -> u32 {
    memory.load_u32(list + ITEMS) + SLOTS + index * 8
}

/// Allocates an empty List with room for `capacity` elements and returns its address.
///
/// The List is not rooted when this returns.
pub(crate) fn allocate_list(memory: &mut impl Memory, vm: u32, capacity: u32) -> Option<u32> {
    let capacity = capacity.max(MIN_CAPACITY);
    let buffer_size = capacity.checked_mul(8)?.checked_add(SLOTS)?;
    let list = allocate(memory, vm, size_of::<ListObject>() as u32, ObjectKind::List)?;
    let root = Vm(vm).push_root(memory, list_reference(list)?)?;
    let items = allocate(memory, vm, buffer_size, ObjectKind::Buffer);
    Vm(vm).pop_roots(memory, root);
    let items = items?;
    memory.store_u32(items + CAPACITY, capacity);
    memory.store_u32(list + ITEMS, items);
    Some(list)
}

/// Appends `count` elements at `source` to the List at `list`, which must have room for them.
fn extend(memory: &mut impl Memory, list: u32, source: u32, count: u32) {
    let used = len(memory, list);
    memory.copy(item_address(memory, list, used), source, count * 8);
    memory.store_u32(list + LEN, used + count);
}

/// Creates a List of the `count` values in the rooted slots at `slots`.
pub fn list_new(memory: &mut impl Memory, vm: u32, slots: u32, count: u32) -> Option<u64> {
    let list = allocate_list(memory, vm, count)?;
    extend(memory, list, slots, count);
    list_reference(list)
}

/// Evaluates `left + right` for a List `left`: a new List of `left`'s elements followed by
/// `right`'s elements when it is a List, or by `right` itself otherwise.
///
/// Both values must be rooted. Returns `None` when `left` is no List or memory is exhausted.
pub fn list_add(memory: &mut impl Memory, vm: u32, left: u64, right: u64) -> Option<u64> {
    let left = list_object(left)?;
    let right_list = list_object(right);
    let left_len = len(memory, left);
    let right_len = right_list.map_or(1, |list| len(memory, list));
    let list = allocate_list(memory, vm, left_len.checked_add(right_len)?)?;
    extend(memory, list, item_address(memory, left, 0), left_len);
    match right_list {
        Some(right) => extend(memory, list, item_address(memory, right, 0), right_len),
        None => {
            memory.store_u64(item_address(memory, list, left_len), right);
            memory.store_u32(list + LEN, left_len + 1);
        }
    }
    list_reference(list)
}

/// Returns a shallow copy of the rooted List `raw`, as `for` loops iterate over.
///
/// Returns `None` when `raw` is no List or memory is exhausted.
pub fn list_copy(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<u64> {
    let source = list_object(raw)?;
    let count = len(memory, source);
    let list = allocate_list(memory, vm, count)?;
    extend(memory, list, item_address(memory, source, 0), count);
    list_reference(list)
}

/// Appends one value to the List at `list`, which must have room for it.
pub(crate) fn push_unchecked(memory: &mut impl Memory, list: u32, value: u64) {
    let used = len(memory, list);
    memory.store_u64(item_address(memory, list, used), value);
    memory.store_u32(list + LEN, used + 1);
}
