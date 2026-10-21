//! Decoding of `let` and `for` bindings against the compiler's static wire schemas, like v1.
//!
//! An Error passes unchanged. A List under a List element contract becomes a new List of
//! decoded elements. An enum of a schema's enum type passes unchanged; an Object tagged with
//! `$variant` becomes that enum, its payload taken by field name or from `$values`. An Object
//! of the first nominal Object alternative passes unchanged; any other Object becomes one with
//! its declared fields decoded and its other keys dropped. Every other value must carry one of
//! the contract's built-in type flags, or decoding returns v1's WireDecodeError.

use core::mem::offset_of;

use exs_abi::TYPE_ERROR;
use exs_layout::{WireContract, WireEnum, WireField, WireObject, WireVariant};

use crate::enums::{allocate_enum, enum_object, enum_type, field_address as enum_field};
use crate::errors::{recoverable, wire_mismatch};
use crate::lists::{allocate_list, item_address, len, list_object, list_reference, push_unchecked};
use crate::memory::Memory;
use crate::methods::{field_value, is_object};
use crate::numbers::reference;
use crate::strings::{string_object, strings_equal};
use crate::typed::{allocate_typed, slot_address, type_id, typed_object};
use crate::vm::Vm;

/// Deepest schema nesting decoded, a bound against recursive types.
const MAX_DEPTH: u32 = 128;

/// Reads the `u32` field at `offset` of the record at `record`.
fn field(memory: &impl Memory, record: u32, offset: usize) -> u32 {
    memory.load_u32(record + offset as u32)
}

/// Decodes `raw` against the contract at `contract`, returning the decoded value or the Error
/// that rejects it.
///
/// `raw` must be rooted. Returns `None` when memory is exhausted or schemas nest too deeply.
pub fn wire_decode(memory: &mut impl Memory, vm: u32, raw: u64, contract: u32) -> Option<u64> {
    decode(memory, vm, raw, contract, 0)
}

/// Decodes `raw` against the contract at `contract` at nesting `depth`.
fn decode(memory: &mut impl Memory, vm: u32, raw: u64, contract: u32, depth: u32) -> Option<u64> {
    if depth >= MAX_DEPTH {
        return None;
    }
    let value = reference(raw)?;
    if value.has_type(TYPE_ERROR) {
        return Some(raw);
    }
    let list = field(memory, contract, offset_of!(WireContract, list));
    if list != 0
        && let Some(source) = list_object(raw)
    {
        return decode_list(memory, vm, source, list, depth);
    }
    if is_object(memory, raw) {
        if let Some(decoded) = decode_enum(memory, vm, raw, contract, depth)? {
            return Some(decoded);
        }
        let object = field(memory, contract, offset_of!(WireContract, object));
        if object != 0 {
            return decode_object(memory, vm, raw, object, depth);
        }
    }
    if value.has_type(field(memory, contract, offset_of!(WireContract, mask))) {
        Some(raw)
    } else {
        wire_mismatch(memory, vm, raw)
    }
}

/// Decodes every element of the List at `source` into a new List, returning the first
/// element's Error instead when one is rejected.
fn decode_list(
    memory: &mut impl Memory,
    vm: u32,
    source: u32,
    item: u32,
    depth: u32,
) -> Option<u64> {
    let count = len(memory, source);
    let list = allocate_list(memory, vm, count)?;
    let state = Vm(vm);
    let root = state.push_root(memory, list_reference(list)?)?;
    let mut result = list_reference(list);
    for index in 0..count {
        // The source List is rooted by the caller and may not shrink while elements decode.
        let element = memory.load_u64(item_address(memory, source, index));
        let decoded = decode(memory, vm, element, item, depth + 1);
        match decoded {
            Some(decoded) if is_error(decoded) => {
                result = Some(decoded);
                break;
            }
            Some(decoded) => push_unchecked(memory, list, decoded),
            None => {
                result = None;
                break;
            }
        }
    }
    state.pop_roots(memory, root);
    result
}

/// Returns whether `raw` is an Error.
fn is_error(raw: u64) -> bool {
    reference(raw).is_some_and(|value| value.has_type(TYPE_ERROR))
}

/// Returns the address of the enum alternative of `contract` with type id `type_id`.
fn enum_schema(memory: &impl Memory, contract: u32, type_id: u32) -> Option<u32> {
    let count = field(memory, contract, offset_of!(WireContract, enum_count));
    let enums = field(memory, contract, offset_of!(WireContract, enums));
    (0..count)
        .map(|index| enums + index * size_of::<WireEnum>() as u32)
        .find(|schema| field(memory, *schema, offset_of!(WireEnum, type_id)) == type_id)
}

/// Decodes the Object `raw` as an enum of `contract`, returning `Some(None)` when it is none
/// of its enums and carries no known `$variant`.
fn decode_enum(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
    contract: u32,
    depth: u32,
) -> Option<Option<u64>> {
    if let Some(object) = enum_object(memory, raw)
        && enum_schema(memory, contract, enum_type(memory, object)).is_some()
    {
        return Some(Some(raw));
    }
    let Some(variant) = field_value(memory, vm, raw, b"$variant") else {
        return Some(None);
    };
    let Some(name) = string_object(variant) else {
        let error = recoverable(
            memory,
            vm,
            "WireDecodeError",
            "enum `$variant` must be a String",
            raw,
        )?;
        return Some(Some(error));
    };
    let Some((schema, variant)) = find_variant(memory, contract, name) else {
        return Some(None);
    };
    decode_variant(memory, vm, raw, schema, variant, depth).map(Some)
}

/// Finds the enum alternative of `contract` and its variant named by the String at `name`.
fn find_variant(memory: &impl Memory, contract: u32, name: u32) -> Option<(u32, u32)> {
    let count = field(memory, contract, offset_of!(WireContract, enum_count));
    let enums = field(memory, contract, offset_of!(WireContract, enums));
    (0..count)
        .map(|index| enums + index * size_of::<WireEnum>() as u32)
        .find_map(|schema| {
            let variants = field(memory, schema, offset_of!(WireEnum, variants));
            let count = field(memory, schema, offset_of!(WireEnum, variant_count));
            (0..count)
                .map(|index| variants + index * size_of::<WireVariant>() as u32)
                .find(|variant| {
                    let candidate =
                        memory.load_u64(*variant + offset_of!(WireVariant, name) as u32);
                    string_object(candidate)
                        .is_some_and(|candidate| strings_equal(memory, candidate, name))
                })
                .map(|variant| (schema, variant))
        })
}

/// Decodes the payload of the tagged Object `raw` into a new enum of variant `variant`.
fn decode_variant(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
    schema: u32,
    variant: u32,
    depth: u32,
) -> Option<u64> {
    let type_id = field(memory, schema, offset_of!(WireEnum, type_id));
    let index = field(memory, variant, offset_of!(WireVariant, index));
    let count = field(memory, variant, offset_of!(WireVariant, field_count));
    let fields = field(memory, variant, offset_of!(WireVariant, fields));
    let values = field_value(memory, vm, raw, b"$values").and_then(list_object);
    let result = allocate_enum(memory, vm, type_id, index, count)?;
    let first = enum_field(result as u32, 0);
    decode_fields(
        memory,
        vm,
        raw,
        (fields, count),
        (result, first),
        values,
        depth,
    )
}

/// Decodes the `raw` Object as nominal Object `schema`, or passes an Object of that type.
fn decode_object(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
    schema: u32,
    depth: u32,
) -> Option<u64> {
    let target = field(memory, schema, offset_of!(WireObject, type_id));
    if typed_object(memory, raw).is_some_and(|object| type_id(memory, object) == target) {
        return Some(raw);
    }
    let count = field(memory, schema, offset_of!(WireObject, field_count));
    let fields = field(memory, schema, offset_of!(WireObject, fields));
    let result = allocate_typed(memory, vm, target, count)?;
    let first = slot_address(result as u32, 0);
    decode_fields(
        memory,
        vm,
        raw,
        (fields, count),
        (result, first),
        None,
        depth,
    )
}

/// Decodes the `count` fields at `fields` of the Object `raw` into the consecutive slots at
/// `first` of the new object `result`, each taken by name, else from the List `positional`,
/// else None. Returns `result`, or the first field's Error that rejects its value.
fn decode_fields(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
    (fields, count): (u32, u32),
    (result, first): (u64, u32),
    positional: Option<u32>,
    depth: u32,
) -> Option<u64> {
    let state = Vm(vm);
    let root = state.push_root(memory, result)?;
    let mut outcome = Some(result);
    for position in 0..count {
        let schema = fields + position * size_of::<WireField>() as u32;
        let name = memory.load_u64(schema + offset_of!(WireField, name) as u32);
        let contract = field(memory, schema, offset_of!(WireField, contract));
        let by_name = string_object(name).and_then(|name| {
            let key = crate::strings::text(memory, name).as_bytes();
            field_value(memory, vm, raw, key)
        });
        let value = by_name
            .or_else(|| {
                positional
                    .filter(|values| position < len(memory, *values))
                    .map(|values| memory.load_u64(item_address(memory, values, position)))
            })
            .unwrap_or(exs_value::ValueRef::NONE.raw());
        match decode(memory, vm, value, contract, depth + 1) {
            Some(decoded) if is_error(decoded) => {
                outcome = Some(decoded);
                break;
            }
            Some(decoded) => memory.store_u64(first + position * 8, decoded),
            None => {
                outcome = None;
                break;
            }
        }
    }
    state.pop_roots(memory, root);
    outcome
}
