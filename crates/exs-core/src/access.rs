//! Index and property access outside the inline fast paths of generated code.

use exs_value::{Immediate, ValueRef};

use crate::lists::{item_address, len, list_object};
use crate::memory::Memory;
use crate::numbers::{Number, number, reference};
use crate::typed::{field_address, typed_object};

/// Why an index or property access produced no value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessError {
    /// The receiver cannot be indexed or has no properties.
    NotAccessible,
    /// A List index is not a non-negative Int.
    InvalidIndex,
    /// A List index is outside the List's bounds.
    OutOfBounds,
    /// The receiver cannot store this key yet.
    UnsupportedKey,
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

/// Evaluates `receiver[index]` for Lists and typed Objects.
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

/// Evaluates `receiver[index] = value` for Lists and typed Objects.
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

/// Reads the property named by the String `name`; a field the type does not declare is None.
///
/// # Errors
///
/// Returns [`AccessError::NotAccessible`] when the receiver is no Object.
pub fn property_get(
    memory: &impl Memory,
    vm: u32,
    receiver: u64,
    name: u64,
) -> Result<u64, AccessError> {
    let object = typed_object(memory, receiver).ok_or(AccessError::NotAccessible)?;
    Ok(field_address(memory, vm, object, name)
        .map_or(ValueRef::NONE.raw(), |slot| memory.load_u64(slot)))
}

/// Writes the property named by the String `name`.
///
/// # Errors
///
/// Returns [`AccessError::NotAccessible`] when the receiver is no Object and
/// [`AccessError::UnsupportedKey`] for keys its type does not declare.
pub fn property_set(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    name: u64,
    value: u64,
) -> Result<(), AccessError> {
    let object = typed_object(memory, receiver).ok_or(AccessError::NotAccessible)?;
    let slot = field_address(memory, vm, object, name).ok_or(AccessError::UnsupportedKey)?;
    memory.store_u64(slot, value);
    Ok(())
}
