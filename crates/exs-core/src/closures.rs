//! Closures and the Cells of the bindings they capture.
//!
//! A closure is a `ClosureObject` whose header word is the program's dense closure index and
//! whose capture slots hold Cells. A Cell is the shared storage of one captured binding; the
//! compiler reads and writes its value inline.

use core::mem::{offset_of, size_of};

use exs_abi::TYPE_FN;
use exs_layout::{CellObject, ClosureObject, ObjectKind};
use exs_value::ValueRef;
use exs_value::layout::{KIND_CELL, KIND_SHIFT, type_kind};

use crate::allocator::allocate;
use crate::errors::recoverable;
use crate::memory::Memory;

/// Byte offset of the closure index in the header word.
const INDEX: u32 = 4;
/// Byte offset of [`ClosureObject::capture_count`].
const CAPTURE_COUNT: u32 = offset_of!(ClosureObject, capture_count) as u32;
/// Byte offset of the first capture slot.
const CAPTURES: u32 = size_of::<ClosureObject>() as u32;
/// Byte offset of [`CellObject::value`].
const CELL_VALUE: u32 = offset_of!(CellObject, value) as u32;

/// Returns the address of the closure behind one raw reference, or `None` for other values.
pub(crate) fn closure_object(raw: u64) -> Option<u32> {
    (raw >> KIND_SHIFT == u64::from(type_kind(TYPE_FN))).then_some(raw as u32)
}

/// Returns the address of the Cell behind one raw reference, or `None` for other values.
pub(crate) fn cell_object(raw: u64) -> Option<u32> {
    (raw >> KIND_SHIFT == u64::from(KIND_CELL)).then_some(raw as u32)
}

/// Returns the number of captures of the closure at `object`.
pub(crate) fn capture_count(memory: &impl Memory, object: u32) -> u32 {
    memory.load_u32(object + CAPTURE_COUNT)
}

/// Returns the program's closure index of the closure at `object`.
pub(crate) fn closure_index(memory: &impl Memory, object: u32) -> u32 {
    memory.load_u32(object + INDEX)
}

/// Returns the address of capture slot `index` of the closure at `object`.
pub(crate) const fn capture_address(object: u32, index: u32) -> u32 {
    object + CAPTURES + index * 8
}

/// Returns the address of the value slot of the Cell at `object`.
pub(crate) const fn cell_value_address(object: u32) -> u32 {
    object + CELL_VALUE
}

/// Allocates a closure of index `index` with `count` empty capture slots and returns its
/// reference.
pub(crate) fn allocate_closure(
    memory: &mut impl Memory,
    vm: u32,
    index: u32,
    count: u32,
) -> Option<u64> {
    let size = count.checked_mul(8)?.checked_add(CAPTURES)?;
    let object = allocate(memory, vm, size, ObjectKind::Closure)?;
    memory.store_u32(object + INDEX, index);
    memory.store_u32(object + CAPTURE_COUNT, count);
    ValueRef::from_parts(type_kind(TYPE_FN), 0, object).map(ValueRef::raw)
}

/// Creates the closure of index `index` capturing the `count` rooted Cells at `slots`.
pub fn closure_new(
    memory: &mut impl Memory,
    vm: u32,
    index: u32,
    slots: u32,
    count: u32,
) -> Option<u64> {
    let raw = allocate_closure(memory, vm, index, count)?;
    memory.copy(capture_address(raw as u32, 0), slots, count * 8);
    Some(raw)
}

/// Creates a Cell holding `value`, which must be rooted, and returns its reference.
pub fn cell_new(memory: &mut impl Memory, vm: u32, value: u64) -> Option<u64> {
    let object = allocate(memory, vm, size_of::<CellObject>() as u32, ObjectKind::Cell)?;
    memory.store_u64(object + CELL_VALUE, value);
    ValueRef::from_parts(KIND_CELL, 0, object).map(ValueRef::raw)
}

/// Creates the ArityError of a closure called with the wrong number of arguments, like v1.
pub fn closure_arity_error(memory: &mut impl Memory, vm: u32) -> Option<u64> {
    recoverable(
        memory,
        vm,
        "ArityError",
        "closure received an incorrect number of arguments",
        ValueRef::NONE.raw(),
    )
}

/// Creates the TypeError of a call of a binding that holds no closure, like v1.
pub fn not_callable(memory: &mut impl Memory, vm: u32, value: u64) -> Option<u64> {
    recoverable(memory, vm, "TypeError", "value is not callable", value)
}
