//! Structural equality: how far two values are equal without looking into their parts, and
//! the parts the prelude compares one by one.

use exs_layout::{
    EQUALITY_DIFFERENT, EQUALITY_SAME, EQUALITY_STRUCTURAL, HOST_ENUM_VARIANT, ValueRef,
};

use crate::bytes::{bytes_object, octets};
use crate::enums::{enum_object, enum_type, field_address, names, payload_count, variant};
use crate::errors::{error_object, error_parts};
use crate::lists::{allocate_list, item_address, len, list_object, list_reference, set_len};
use crate::memory::Memory;
use crate::methods::objects::object_len;
use crate::numbers::{number, numbers_equal};
use crate::strings::{string_object, strings_equal};
use crate::typed::{slot_address, slot_count, type_id, typed_object};

/// Compares two values as far as possible without looking into their parts.
///
/// Numbers compare by value with Float promotion, Strings by text, Bytes by octets, and
/// identical references are equal. Closures, cells and other resources compare by identity.
#[must_use]
pub fn equality(memory: &impl Memory, vm: u32, left: u64, right: u64) -> u32 {
    let same = |equal: bool| {
        if equal {
            EQUALITY_SAME
        } else {
            EQUALITY_DIFFERENT
        }
    };
    if left == right {
        return EQUALITY_SAME;
    }
    if let (Some(left), Some(right)) = (number(memory, left), number(memory, right)) {
        return same(numbers_equal(left, right));
    }
    if let (Some(left), Some(right)) = (string_object(left), string_object(right)) {
        return same(strings_equal(memory, left, right));
    }
    if let (Some(left), Some(right)) = (bytes_object(left), bytes_object(right)) {
        return same(octets(memory, left) == octets(memory, right));
    }
    if let (Some(left), Some(right)) = (list_object(left), list_object(right)) {
        return structural(len(memory, left) == len(memory, right));
    }
    if error_object(left).is_some() && error_object(right).is_some() {
        return EQUALITY_STRUCTURAL;
    }
    if let (Some(left), Some(right)) = (enum_object(memory, left), enum_object(memory, right)) {
        return enum_equality(memory, vm, left, right);
    }
    if let (Some(left), Some(right)) = (typed_object(memory, left), typed_object(memory, right)) {
        return structural(type_id(memory, left) == type_id(memory, right));
    }
    if typed_object(memory, left).is_some() || typed_object(memory, right).is_some() {
        return EQUALITY_DIFFERENT;
    }
    match (object_len(memory, vm, left), object_len(memory, vm, right)) {
        (Some(left), Some(right)) => structural(left == right),
        _ => EQUALITY_DIFFERENT,
    }
}

/// Returns [`EQUALITY_STRUCTURAL`] when `candidate` holds, else [`EQUALITY_DIFFERENT`].
const fn structural(candidate: bool) -> u32 {
    if candidate {
        EQUALITY_STRUCTURAL
    } else {
        EQUALITY_DIFFERENT
    }
}

/// Compares the enums at `left` and `right`: equal variants of one type are equal without a
/// payload and structural with one.
fn enum_equality(memory: &impl Memory, vm: u32, left: u32, right: u32) -> u32 {
    let host =
        variant(memory, left) == HOST_ENUM_VARIANT || variant(memory, right) == HOST_ENUM_VARIANT;
    let same_variant = if host {
        match (names(memory, vm, left), names(memory, vm, right)) {
            (Some((left_type, left_name)), Some((right_type, right_name))) => {
                strings_equal(memory, left_type, right_type)
                    && strings_equal(memory, left_name, right_name)
            }
            _ => false,
        }
    } else {
        enum_type(memory, left) == enum_type(memory, right)
            && variant(memory, left) == variant(memory, right)
    };
    let count = payload_count(memory, left);
    if !same_variant || count != payload_count(memory, right) {
        EQUALITY_DIFFERENT
    } else if count == 0 {
        EQUALITY_SAME
    } else {
        EQUALITY_STRUCTURAL
    }
}

/// Returns a new List of the parts of a composite value in a stable order: the elements of a
/// List, the field values of a nominal value in declaration order, the payload of an enum, and
/// the kind, message, data and cause of an Error. Absent parts are None.
///
/// Returns None for untyped Objects, whose parts are named, and for every other value; `None`
/// when memory is exhausted. `raw` must be rooted.
pub fn parts(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<u64> {
    let (source, count) = if let Some(list) = list_object(raw) {
        let count = len(memory, list);
        (item_address(memory, list, 0), count)
    } else if let Some(object) = typed_object(memory, raw) {
        (slot_address(object, 0), slot_count(memory, object))
    } else if let Some(object) = enum_object(memory, raw) {
        (field_address(object, 0), payload_count(memory, object))
    } else if let Some(object) = error_object(raw) {
        let parts = error_parts(memory, object);
        let values = [parts.kind, parts.message, parts.data, parts.cause];
        let list = allocate_list(memory, vm, 4)?;
        for (index, value) in (0..).zip(values) {
            memory.store_u64(item_address(memory, list, index), present(value));
        }
        set_len(memory, list, 4);
        return list_reference(list);
    } else {
        return Some(ValueRef::NONE.raw());
    };
    // The heap does not move and `raw` is rooted, so `source` stays valid across the
    // allocation; a List's item buffer only moves when the List grows.
    let list = allocate_list(memory, vm, count)?;
    for index in 0..count {
        let value = memory.load_u64(source + index * 8);
        memory.store_u64(item_address(memory, list, index), present(value));
    }
    set_len(memory, list, count);
    list_reference(list)
}

/// Returns `value`, or None for an empty slot.
fn present(value: u64) -> u64 {
    if value == 0 {
        ValueRef::NONE.raw()
    } else {
        value
    }
}
