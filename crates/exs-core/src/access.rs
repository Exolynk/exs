//! Index and property access outside the inline fast paths of generated code.
//!
//! Failures are the language's IndexError and TypeError values with the messages; every
//! function returns `None` only when memory is exhausted.

use exs_layout::{Immediate, ValueRef};

use crate::errors::recoverable;
use crate::lists::{item_address, len, list_object};
use crate::memory::Memory;
use crate::numbers::{Number, number, reference};
use crate::objects::{object_get, object_set, untyped_object};
use crate::strings::string_object;
use crate::typed::{field_address, typed_object};

/// Message of the TypeError for an Object key that is no String.
const KEY_NOT_STRING: &str = "Object keys and method names require a String value";

/// Where one checked List index points.
enum ListSlot {
    /// The address of the element slot.
    Slot(u32),
    /// The IndexError the index produced.
    Error(u64),
}

/// Reads a List index: a non-negative Int in the supported range, else the IndexError
/// value (`Err(None)` when memory is exhausted).
pub(crate) fn list_index(
    memory: &mut impl Memory,
    vm: u32,
    index: u64,
) -> Result<u32, Option<u64>> {
    let is_bool = matches!(
        reference(index).and_then(ValueRef::immediate),
        Some(Immediate::Bool(_))
    );
    let message = match number(memory, index) {
        Some(Number::Int(position)) if !is_bool && position >= 0 => match u32::try_from(position) {
            Ok(position) => return Ok(position),
            Err(_) => "List index is outside the supported range",
        },
        _ => "List index requires a non-negative Int value",
    };
    Err(recoverable(memory, vm, "IndexError", message, index))
}

/// Checks `index` against the List at `list`: first that it is a non-negative Int,
/// then that it lies inside the List.
fn list_slot(memory: &mut impl Memory, vm: u32, list: u32, index: u64) -> Option<ListSlot> {
    match list_index(memory, vm, index) {
        Ok(position) if position < len(memory, list) => {
            Some(ListSlot::Slot(item_address(memory, list, position)))
        }
        Ok(_) => recoverable(
            memory,
            vm,
            "IndexError",
            "List index is outside the List bounds",
            index,
        )
        .map(ListSlot::Error),
        Err(error) => error.map(ListSlot::Error),
    }
}

/// Where the key `name` of an Object receiver lives.
enum KeyOwner {
    /// A field slot declared by the receiver's type.
    Field(u32),
    /// The keys of an untyped Object.
    Untyped(u32),
    /// A key the type of a typed Object does not declare.
    Undeclared,
    /// An enum, an Object without keys.
    Enum,
}

/// Classifies an Object receiver for the key `name`, or returns `None` for other receivers.
fn key_owner(memory: &impl Memory, vm: u32, receiver: u64, name: u64) -> Option<KeyOwner> {
    if let Some(object) = typed_object(memory, receiver) {
        return Some(match field_address(memory, vm, object, name) {
            Some(slot) => KeyOwner::Field(slot),
            None => KeyOwner::Undeclared,
        });
    }
    if crate::enums::enum_object(memory, receiver).is_some() {
        return Some(KeyOwner::Enum);
    }
    untyped_object(memory, receiver).map(KeyOwner::Untyped)
}

/// Evaluates `receiver[index]` for Lists and Objects.
pub fn index_get(memory: &mut impl Memory, vm: u32, receiver: u64, index: u64) -> Option<u64> {
    if let Some(list) = list_object(receiver) {
        return match list_slot(memory, vm, list, index)? {
            ListSlot::Slot(slot) => Some(memory.load_u64(slot)),
            ListSlot::Error(error) => Some(error),
        };
    }
    if crate::bytes::bytes_object(receiver).is_some() {
        return crate::methods::bytes_get(memory, vm, receiver, index);
    }
    let Some(owner) = key_owner(memory, vm, receiver, index) else {
        return recoverable(
            memory,
            vm,
            "TypeError",
            "index access requires a Bytes, List, or Object receiver",
            receiver,
        );
    };
    let Some(name) = string_object(index) else {
        return recoverable(memory, vm, "TypeError", KEY_NOT_STRING, index);
    };
    Some(match owner {
        // An empty declared slot reads as None.
        KeyOwner::Field(slot) => match memory.load_u64(slot) {
            0 => ValueRef::NONE.raw(),
            value => value,
        },
        KeyOwner::Untyped(object) => object_get(memory, object, name),
        KeyOwner::Enum => return recoverable(memory, vm, "TypeError", NO_ENUM_FIELDS, index),
        KeyOwner::Undeclared => return recoverable(memory, vm, "TypeError", NO_SUCH_FIELD, index),
    })
}

/// Message of the TypeError for a field the type of a nominal value does not declare.
const NO_SUCH_FIELD: &str = "the nominal type declares no such field";

/// Message of the TypeError for a property of an enum, whose payload `match` binds.
const NO_ENUM_FIELDS: &str = "enum values have no properties; bind their payload with match";

/// Evaluates `receiver[index] = value` for Lists and Objects and returns `value`, or the Error
/// the assignment produced.
///
/// Nominal values only have their declared fields. All values must be rooted, because adding a
/// key to an untyped Object may allocate.
pub fn index_set(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    index: u64,
    value: u64,
) -> Option<u64> {
    if let Some(list) = list_object(receiver) {
        return match list_slot(memory, vm, list, index)? {
            ListSlot::Slot(slot) => {
                memory.store_u64(slot, value);
                Some(value)
            }
            ListSlot::Error(error) => Some(error),
        };
    }
    let Some(owner) = key_owner(memory, vm, receiver, index) else {
        return recoverable(
            memory,
            vm,
            "TypeError",
            "index assignment requires a List or Object receiver",
            receiver,
        );
    };
    if string_object(index).is_none() {
        return recoverable(memory, vm, "TypeError", KEY_NOT_STRING, index);
    }
    let object = match owner {
        KeyOwner::Field(slot) => {
            memory.store_u64(slot, value);
            return Some(value);
        }
        KeyOwner::Untyped(object) => object,
        KeyOwner::Enum => return recoverable(memory, vm, "TypeError", NO_ENUM_FIELDS, index),
        KeyOwner::Undeclared => return recoverable(memory, vm, "TypeError", NO_SUCH_FIELD, index),
    };
    object_set(memory, vm, object, index, value)?;
    Some(value)
}

/// Reads the property named by the static String `name`, like `receiver[name]`.
pub fn property_get(memory: &mut impl Memory, vm: u32, receiver: u64, name: u64) -> Option<u64> {
    index_get(memory, vm, receiver, name)
}

/// Writes the property named by the static String `name`, like `receiver[name] = value`.
pub fn property_set(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    name: u64,
    value: u64,
) -> Option<u64> {
    index_set(memory, vm, receiver, name, value)
}
