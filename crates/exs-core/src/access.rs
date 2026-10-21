//! Index and property access outside the inline fast paths of generated code.

use exs_value::{Immediate, ValueRef};

use crate::lists::{item_address, len, list_object};
use crate::memory::Memory;
use crate::numbers::{Number, number, reference};
use crate::objects::{empty_object, object_get, object_set, untyped_object};
use crate::strings::string_object;
use crate::typed::{extra, field_address, set_extra, typed_object};

/// Why an index or property access produced no value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessError {
    /// The receiver cannot be indexed or has no properties.
    NotAccessible,
    /// A List index is not a non-negative Int.
    InvalidIndex,
    /// A List index is outside the List's bounds.
    OutOfBounds,
    /// An Object key is not a String.
    InvalidKey,
    /// The heap could not hold a new key.
    OutOfMemory,
}

/// Returns the slot address of List element `index`, checking the index like the language.
fn list_slot(memory: &impl Memory, list: u32, index: u64) -> Result<u32, AccessError> {
    let is_bool = matches!(
        reference(index).and_then(ValueRef::immediate),
        Some(Immediate::Bool(_))
    );
    let index = match number(memory, index) {
        Some(Number::Int(index)) if !is_bool && index >= 0 => index,
        _ => return Err(AccessError::InvalidIndex),
    };
    match u32::try_from(index) {
        Ok(index) if index < len(memory, list) => Ok(item_address(memory, list, index)),
        _ => Err(AccessError::OutOfBounds),
    }
}

/// Evaluates `receiver[index]` for Lists and Objects.
///
/// # Errors
///
/// Returns why the access produced no value.
pub fn index_get(
    memory: &impl Memory,
    vm: u32,
    receiver: u64,
    index: u64,
) -> Result<u64, AccessError> {
    if let Some(list) = list_object(receiver) {
        return Ok(memory.load_u64(list_slot(memory, list, index)?));
    }
    property_get(memory, vm, receiver, index)
}

/// Evaluates `receiver[index] = value` for Lists and Objects.
///
/// # Errors
///
/// Returns why the value could not be stored.
pub fn index_set(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    index: u64,
    value: u64,
) -> Result<(), AccessError> {
    if let Some(list) = list_object(receiver) {
        let slot = list_slot(memory, list, index)?;
        memory.store_u64(slot, value);
        return Ok(());
    }
    property_set(memory, vm, receiver, index, value)
}

/// Reads the property named by the String `name`; a key the Object lacks reads as None.
///
/// Typed Objects look up their declared fields first and then their extra keys.
///
/// # Errors
///
/// Returns [`AccessError::NotAccessible`] when the receiver is no Object and
/// [`AccessError::InvalidKey`] when `name` is no String.
pub fn property_get(
    memory: &impl Memory,
    vm: u32,
    receiver: u64,
    name: u64,
) -> Result<u64, AccessError> {
    let text = string_object(name);
    if let Some(object) = typed_object(memory, receiver) {
        let text = text.ok_or(AccessError::InvalidKey)?;
        if let Some(slot) = field_address(memory, vm, object, name) {
            return Ok(memory.load_u64(slot));
        }
        return Ok(match extra(memory, object) {
            0 => ValueRef::NONE.raw(),
            extra => object_get(memory, extra, text),
        });
    }
    let object = untyped_object(memory, receiver).ok_or(AccessError::NotAccessible)?;
    Ok(object_get(
        memory,
        object,
        text.ok_or(AccessError::InvalidKey)?,
    ))
}

/// Writes the property named by the String `name`, adding the key when the Object lacks it.
///
/// Typed Objects keep keys their type does not declare in an untyped extra Object. The
/// receiver, `name` and `value` must be rooted, because adding a key may allocate.
///
/// # Errors
///
/// Returns [`AccessError::NotAccessible`] when the receiver is no Object,
/// [`AccessError::InvalidKey`] when `name` is no String and [`AccessError::OutOfMemory`] when
/// a new key does not fit.
pub fn property_set(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    name: u64,
    value: u64,
) -> Result<(), AccessError> {
    if string_object(name).is_none() {
        return Err(AccessError::InvalidKey);
    }
    let object = if let Some(object) = typed_object(memory, receiver) {
        if let Some(slot) = field_address(memory, vm, object, name) {
            memory.store_u64(slot, value);
            return Ok(());
        }
        match extra(memory, object) {
            0 => {
                let created = empty_object(memory, vm, 0).ok_or(AccessError::OutOfMemory)?;
                set_extra(memory, object, created);
                created
            }
            extra => extra,
        }
    } else {
        untyped_object(memory, receiver).ok_or(AccessError::NotAccessible)?
    };
    object_set(memory, vm, object, name, value).ok_or(AccessError::OutOfMemory)
}
