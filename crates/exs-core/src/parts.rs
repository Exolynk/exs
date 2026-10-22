//! Structural equality: how far two values are equal without looking into their parts, and
//! the parts the prelude compares one by one.

use exs_layout::{
    EQUALITY_DIFFERENT, EQUALITY_SAME, EQUALITY_STRUCTURAL, HOST_ENUM_VARIANT,
    TYPE_ENTRY_CUSTOM_EQUALITY, ValueRef,
};

use crate::bytes::{bytes_object, octets};
use crate::enums::{enum_object, enum_type, field_address, names, payload_count, variant};
use crate::errors::{error_object, error_parts};
use crate::lists::{allocate_list, item_address, len, list_object, list_reference, set_len};
use crate::memory::Memory;
use crate::methods::objects::object_len;
use crate::numbers::{number, numbers_equal};
use crate::objects::{find_value, shape, untyped_object, value_address};
use crate::shapes::{key, key_count};
use crate::strings::{string_object, strings_equal};
use crate::typed::{slot_address, slot_count, type_entry, type_id, typed_object};

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

/// Deepest nesting the native comparison follows before it leaves the rest to the prelude,
/// which also handles cycles.
const NATIVE_DEPTH: u32 = 32;

/// Compares two values part by part where the core can: like [`equality`], but composites
/// are compared natively in the prelude's order. Returns [`EQUALITY_STRUCTURAL`] only when
/// the comparison reaches a value whose type implements `Eq` or `Compare`, or nests too
/// deeply; the prelude then compares the parts.
#[must_use]
pub fn deep_equality(memory: &impl Memory, vm: u32, left: u64, right: u64) -> u32 {
    deep(memory, vm, left, right, 0)
}

/// Compares `left` and `right` at nesting `depth`.
fn deep(memory: &impl Memory, vm: u32, left: u64, right: u64, depth: u32) -> u32 {
    let outcome = equality(memory, vm, left, right);
    if outcome != EQUALITY_STRUCTURAL {
        return outcome;
    }
    if depth >= NATIVE_DEPTH || custom_kind(memory, vm, left, TYPE_ENTRY_CUSTOM_EQUALITY) {
        return EQUALITY_STRUCTURAL;
    }
    // Compares the `count` part pairs `part(index)` returns, stopping at the first that is
    // not equal.
    let pairs = |count: u32, part: &dyn Fn(u32) -> (u64, u64)| {
        for index in 0..count {
            let (left, right) = part(index);
            let outcome = deep(memory, vm, left, right, depth + 1);
            if outcome != EQUALITY_SAME {
                return outcome;
            }
        }
        EQUALITY_SAME
    };
    if let (Some(left_list), Some(right_list)) = (list_object(left), list_object(right)) {
        return pairs(len(memory, left_list), &|index| {
            (
                memory.load_u64(item_address(memory, left_list, index)),
                memory.load_u64(item_address(memory, right_list, index)),
            )
        });
    }
    if let (Some(left_object), Some(right_object)) =
        (typed_object(memory, left), typed_object(memory, right))
    {
        return pairs(slot_count(memory, left_object), &|index| {
            (
                present(memory.load_u64(slot_address(left_object, index))),
                present(memory.load_u64(slot_address(right_object, index))),
            )
        });
    }
    if let (Some(left_enum), Some(right_enum)) =
        (enum_object(memory, left), enum_object(memory, right))
    {
        return pairs(payload_count(memory, left_enum), &|index| {
            (
                present(memory.load_u64(field_address(left_enum, index))),
                present(memory.load_u64(field_address(right_enum, index))),
            )
        });
    }
    if let (Some(left_error), Some(right_error)) = (error_object(left), error_object(right)) {
        let (left_parts, right_parts) = (
            error_parts(memory, left_error),
            error_parts(memory, right_error),
        );
        let left_values = [
            left_parts.kind,
            left_parts.message,
            left_parts.data,
            left_parts.cause,
        ];
        let right_values = [
            right_parts.kind,
            right_parts.message,
            right_parts.data,
            right_parts.cause,
        ];
        for (left, right) in left_values.into_iter().zip(right_values) {
            let outcome = deep(memory, vm, present(left), present(right), depth + 1);
            if outcome != EQUALITY_SAME {
                return outcome;
            }
        }
        return EQUALITY_SAME;
    }
    if let (Some(left_object), Some(right_object)) =
        (untyped_object(memory, left), untyped_object(memory, right))
    {
        let shape = shape(memory, left_object);
        for index in 0..key_count(memory, shape) {
            let Some(name) = string_object(key(memory, shape, index)) else {
                return EQUALITY_STRUCTURAL;
            };
            let Some(position) = find_value(memory, right_object, name) else {
                return EQUALITY_DIFFERENT;
            };
            let outcome = deep(
                memory,
                vm,
                memory.load_u64(value_address(memory, left_object, index)),
                memory.load_u64(value_address(memory, right_object, position)),
                depth + 1,
            );
            if outcome != EQUALITY_SAME {
                return outcome;
            }
        }
        return EQUALITY_SAME;
    }
    EQUALITY_STRUCTURAL
}

/// Reports whether `raw` is a nominal value or enum whose type entry has the `flag` bit.
pub(crate) fn custom_kind(memory: &impl Memory, vm: u32, raw: u64, flag: u32) -> bool {
    let type_id = typed_object(memory, raw)
        .map(|object| type_id(memory, object))
        .or_else(|| enum_object(memory, raw).map(|object| enum_type(memory, object)));
    type_id
        .and_then(|type_id| type_entry(memory, vm, type_id))
        .is_some_and(|entry| entry.kind & flag != 0)
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
pub(crate) fn present(value: u64) -> u64 {
    if value == 0 {
        ValueRef::NONE.raw()
    } else {
        value
    }
}
