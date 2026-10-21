//! Shapes: the shared, ordered key lists of untyped Objects.
//!
//! Shapes form a tree rooted at the VM's empty root shape; adding a key to an Object moves it
//! to the child shape extending its current shape by that key. Equal key sequences therefore
//! share one shape, which generated code compares by address in its inline caches. Shapes
//! are reachable from the root shape and are never collected.

use core::mem::{offset_of, size_of};

use exs_layout::{BufferObject, ObjectKind, ShapeObject};

use crate::allocator::allocate;
use crate::memory::Memory;
use crate::strings::{string_object, strings_equal};
use crate::vm::Vm;

/// Byte offset of the key count in the header word.
const KEY_COUNT: u32 = 4;
/// Byte offset of [`ShapeObject::parent`].
const PARENT: u32 = offset_of!(ShapeObject, parent) as u32;
/// Byte offset of [`ShapeObject::keys`].
const KEYS: u32 = offset_of!(ShapeObject, keys) as u32;
/// Byte offset of [`ShapeObject::first_child`].
const FIRST_CHILD: u32 = offset_of!(ShapeObject, first_child) as u32;
/// Byte offset of [`ShapeObject::next_sibling`].
const NEXT_SIBLING: u32 = offset_of!(ShapeObject, next_sibling) as u32;
/// Byte offset of a Buffer's slot count in its header word.
const CAPACITY: u32 = 4;
/// Byte offset of a Buffer's first slot.
const SLOTS: u32 = size_of::<BufferObject>() as u32;

/// Returns the VM's empty root shape, creating it on first use.
pub(crate) fn root_shape(memory: &mut impl Memory, vm: u32) -> Option<u32> {
    let state = Vm(vm);
    let existing = state.root_shape(memory);
    if existing != 0 {
        return Some(existing);
    }
    let shape = allocate(
        memory,
        vm,
        size_of::<ShapeObject>() as u32,
        ObjectKind::Shape,
    )?;
    state.set_root_shape(memory, shape);
    Some(shape)
}

/// Returns the number of keys of the shape at `shape`.
pub(crate) fn key_count(memory: &impl Memory, shape: u32) -> u32 {
    memory.load_u32(shape + KEY_COUNT)
}

/// Returns the raw String reference of key `index` of the shape at `shape`.
pub(crate) fn key(memory: &impl Memory, shape: u32, index: u32) -> u64 {
    memory.load_u64(memory.load_u32(shape + KEYS) + SLOTS + index * 8)
}

/// Returns the position of the key equal to the String at `name` in the shape at `shape`.
pub(crate) fn find_key(memory: &impl Memory, shape: u32, name: u32) -> Option<u32> {
    (0..key_count(memory, shape)).find(|index| {
        string_object(key(memory, shape, *index))
            .is_some_and(|candidate| strings_equal(memory, candidate, name))
    })
}

/// Returns the shape extending the shape at `shape` by the String `name`, creating it when
/// no Object had this key sequence before.
///
/// `name` must be rooted; the new shape keeps it alive from then on.
pub(crate) fn transition(memory: &mut impl Memory, vm: u32, shape: u32, name: u64) -> Option<u32> {
    let text = string_object(name)?;
    let count = key_count(memory, shape);
    let mut child = memory.load_u32(shape + FIRST_CHILD);
    while child != 0 {
        // A child whose key Buffer could not be allocated has no keys and never matches.
        if key_count(memory, child) == count + 1
            && string_object(key(memory, child, count))
                .is_some_and(|last| strings_equal(memory, last, text))
        {
            return Some(child);
        }
        child = memory.load_u32(child + NEXT_SIBLING);
    }
    // Linking the shape first keeps it reachable from the root while its keys are allocated.
    let child = allocate(
        memory,
        vm,
        size_of::<ShapeObject>() as u32,
        ObjectKind::Shape,
    )?;
    memory.store_u32(child + PARENT, shape);
    memory.store_u32(child + NEXT_SIBLING, memory.load_u32(shape + FIRST_CHILD));
    memory.store_u32(shape + FIRST_CHILD, child);
    let keys_size = (count + 1).checked_mul(8)?.checked_add(SLOTS)?;
    let keys = allocate(memory, vm, keys_size, ObjectKind::Buffer)?;
    memory.store_u32(keys + CAPACITY, count + 1);
    if count > 0 {
        let parent_keys = memory.load_u32(shape + KEYS);
        memory.copy(keys + SLOTS, parent_keys + SLOTS, count * 8);
    }
    memory.store_u64(keys + SLOTS + count * 8, name);
    memory.store_u32(child + KEYS, keys);
    memory.store_u32(child + KEY_COUNT, count + 1);
    Some(child)
}

/// Returns the shape of the `count` distinct key Strings at `keys`, which must be rooted or
/// static.
pub fn shape_for(memory: &mut impl Memory, vm: u32, keys: u32, count: u32) -> Option<u32> {
    let mut shape = root_shape(memory, vm)?;
    for index in 0..count {
        shape = transition(memory, vm, shape, memory.load_u64(keys + index * 8))?;
    }
    Some(shape)
}
