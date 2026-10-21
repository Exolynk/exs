//! CBOR encoding of runner inputs and results.
//!
//! The runner passes the entry arguments as one CBOR array and expects one CBOR value back.
//! This version handles None, Bool, Int, Float, String, Bytes, List and untyped Object values both
//! ways, enums of the program's types both ways as the codec's tagged arrays, encodes typed
//! Objects as maps of their declared fields followed by their extra keys and Errors as the
//! codec's tagged seven-field map; other values are reported as unsupported.

use exs_value::{Immediate, ValueRef};

use crate::bytes::{bytes_len, bytes_object, new_bytes};
use crate::enums::{allocate_enum, enum_object, field_address as enum_field, field_count, names};
use crate::errors::{error_object, error_parts, new_error};
use crate::execution::{input, reserve_result};
use crate::lists::{allocate_list, item_address, len, list_object, list_reference, push_unchecked};
use crate::memory::Memory;
use crate::numbers::{Number, float_value, int_value, number, reference};
use crate::objects::{
    empty_object, find_value, object_reference, object_set, shape, untyped_object, value_address,
};
use crate::shapes::{key, key_count};
use crate::strings::{DATA, byte_len, copy_string, string_object};
use crate::typed::{
    extra, ordered_slot, slot_address, slot_count, type_entry, type_id, typed_object,
};
use crate::vm::Vm;

/// Deepest nesting of Lists and Objects the codec converts; deeper values, including cyclic
/// ones, are rejected.
const MAX_DEPTH: u32 = 128;

/// Why an input or result could not be converted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CborError {
    /// The bytes are not well-formed CBOR of the expected shape.
    Malformed,
    /// The value uses a type this codec does not handle yet.
    Unsupported,
    /// The heap could not hold a decoded value or the result.
    OutOfMemory,
}

/// CBOR major type of unsigned integers.
const MAJOR_UNSIGNED: u8 = 0;
/// CBOR major type of negative integers.
const MAJOR_NEGATIVE: u8 = 1;
/// CBOR major type of byte strings.
const MAJOR_BYTES: u8 = 2;
/// CBOR major type of text strings.
const MAJOR_TEXT: u8 = 3;
/// CBOR major type of arrays.
const MAJOR_ARRAY: u8 = 4;
/// CBOR major type of maps.
const MAJOR_MAP: u8 = 5;
/// CBOR major type of tags.
const MAJOR_TAG: u8 = 6;
/// CBOR tag of an ExS Error, as the runner's codec defines it.
const ERROR_TAG: u64 = 60_001;
/// CBOR tag of an ExS enum, as the runner's codec defines it.
const ENUM_TAG: u64 = 60_005;
/// CBOR major type of simple values and floats.
const MAJOR_SIMPLE: u8 = 7;
/// Encoded CBOR `false`.
const FALSE: u8 = 0xf4;
/// Encoded CBOR `true`.
const TRUE: u8 = 0xf5;
/// Encoded CBOR `null`.
const NULL: u8 = 0xf6;
/// Initial byte of a binary64 float.
const FLOAT64: u8 = 0xfb;

/// A read position inside one byte range of linear memory.
struct Reader {
    /// Address of the next byte.
    position: u32,
    /// Address just past the input.
    end: u32,
}

impl Reader {
    /// Returns the next byte without consuming it.
    fn peek(&self, memory: &impl Memory) -> Result<u8, CborError> {
        if self.position >= self.end {
            return Err(CborError::Malformed);
        }
        Ok(memory.load_u8(self.position))
    }

    /// Reads one byte.
    fn byte(&mut self, memory: &impl Memory) -> Result<u8, CborError> {
        if self.position >= self.end {
            return Err(CborError::Malformed);
        }
        let byte = memory.load_u8(self.position);
        self.position += 1;
        Ok(byte)
    }

    /// Reads a big-endian unsigned integer of `count` bytes.
    fn big_endian(&mut self, memory: &impl Memory, count: u32) -> Result<u64, CborError> {
        let mut value = 0;
        for _ in 0..count {
            value = (value << 8) | u64::from(self.byte(memory)?);
        }
        Ok(value)
    }

    /// Reads the argument following an initial byte with additional information `info`.
    fn argument(&mut self, memory: &impl Memory, info: u8) -> Result<u64, CborError> {
        match info {
            0..=23 => Ok(u64::from(info)),
            24 => self.big_endian(memory, 1),
            25 => self.big_endian(memory, 2),
            26 => self.big_endian(memory, 4),
            27 => self.big_endian(memory, 8),
            _ => Err(CborError::Malformed),
        }
    }

    /// Reads one argument as a signed Int magnitude.
    fn int_argument(&mut self, memory: &impl Memory, info: u8) -> Result<i64, CborError> {
        i64::try_from(self.argument(memory, info)?).map_err(|_| CborError::Malformed)
    }

    /// Reads one length argument that must fit the remaining input, as every counted item
    /// takes at least one byte.
    fn count(&mut self, memory: &impl Memory, info: u8) -> Result<u32, CborError> {
        u32::try_from(self.argument(memory, info)?)
            .ok()
            .filter(|count| *count <= self.end - self.position)
            .ok_or(CborError::Malformed)
    }

    /// Decodes one scalar item with initial byte `initial` without allocating.
    fn scalar(&mut self, memory: &impl Memory, initial: u8) -> Result<Scalar, CborError> {
        let (major, info) = (initial >> 5, initial & 0x1f);
        Ok(match (major, info) {
            (MAJOR_UNSIGNED, _) => Scalar::Number(Number::Int(self.int_argument(memory, info)?)),
            (MAJOR_NEGATIVE, _) => {
                Scalar::Number(Number::Int(-1 - self.int_argument(memory, info)?))
            }
            (MAJOR_BYTES, _) => {
                let len = self.count(memory, info)?;
                let start = self.position;
                self.position = start + len;
                Scalar::Bytes { start, len }
            }
            (MAJOR_TEXT, _) => {
                let len = self.count(memory, info)?;
                let start = self.position;
                let end = start + len;
                let chars = utf8_chars(memory, start, len).ok_or(CborError::Malformed)?;
                self.position = end;
                Scalar::Text { start, len, chars }
            }
            (MAJOR_SIMPLE, 20) => Scalar::Immediate(ValueRef::FALSE),
            (MAJOR_SIMPLE, 21) => Scalar::Immediate(ValueRef::TRUE),
            (MAJOR_SIMPLE, 22) => Scalar::Immediate(ValueRef::NONE),
            (MAJOR_SIMPLE, 25) => {
                let bits = self.big_endian(memory, 2)? as u16;
                Scalar::Number(Number::Float(half_to_f64(bits)))
            }
            (MAJOR_SIMPLE, 26) => {
                let bits = self.big_endian(memory, 4)? as u32;
                Scalar::Number(Number::Float(f64::from(f32::from_bits(bits))))
            }
            (MAJOR_SIMPLE, 27) => {
                Scalar::Number(Number::Float(f64::from_bits(self.big_endian(memory, 8)?)))
            }
            _ => return Err(CborError::Unsupported),
        })
    }
}

/// One decoded scalar before any allocation.
enum Scalar {
    /// A value encoded entirely in its reference.
    Immediate(ValueRef),
    /// A number that may need a heap object.
    Number(Number),
    /// Validated UTF-8 text of `len` bytes and `chars` characters at `start`.
    Text {
        /// Address of the first byte.
        start: u32,
        /// Byte length.
        len: u32,
        /// Character count.
        chars: u32,
    },
    /// Octets of `len` bytes at `start`.
    Bytes {
        /// Address of the first octet.
        start: u32,
        /// Number of octets.
        len: u32,
    },
}

/// Decodes the runner's argument array into the root slots at `slots`.
///
/// Stores at most `capacity` arguments, filling the remaining slots with None, and returns the
/// number of arguments the input holds. The slots must be rooted, because decoding allocates.
///
/// # Errors
///
/// Returns why the input could not be decoded.
pub fn decode_inputs(
    memory: &mut impl Memory,
    vm: u32,
    slots: u32,
    capacity: u32,
) -> Result<u32, CborError> {
    let (address, length) = input(memory, vm);
    let mut reader = Reader {
        position: address,
        end: address.checked_add(length).ok_or(CborError::Malformed)?,
    };
    let initial = reader.byte(memory)?;
    if initial >> 5 != MAJOR_ARRAY {
        return Err(CborError::Malformed);
    }
    let count = reader.count(memory, initial & 0x1f)?;
    for index in 0..count {
        let raw = decode_value(memory, vm, &mut reader, 1)?;
        if index < capacity {
            memory.store_u64(slots + index * 8, raw);
        }
    }
    for index in count..capacity {
        memory.store_u64(slots + index * 8, ValueRef::NONE.raw());
    }
    Ok(count)
}

/// Decodes the runner's argument array for an entry with `fixed` parameters and a trailing
/// variadic one: the first `fixed` arguments into the root slots at `slots`, None for missing
/// ones, and a List of the rest into the slot after them, like v1.
///
/// # Errors
///
/// Returns why the input could not be decoded.
pub fn decode_variadic_inputs(
    memory: &mut impl Memory,
    vm: u32,
    slots: u32,
    fixed: u32,
) -> Result<(), CborError> {
    let (address, length) = input(memory, vm);
    let mut reader = Reader {
        position: address,
        end: address.checked_add(length).ok_or(CborError::Malformed)?,
    };
    let initial = reader.byte(memory)?;
    if initial >> 5 != MAJOR_ARRAY {
        return Err(CborError::Malformed);
    }
    let count = reader.count(memory, initial & 0x1f)?;
    for index in 0..fixed {
        memory.store_u64(slots + index * 8, ValueRef::NONE.raw());
    }
    let rest =
        allocate_list(memory, vm, count.saturating_sub(fixed)).ok_or(CborError::OutOfMemory)?;
    let rest_slot = slots + fixed * 8;
    memory.store_u64(
        rest_slot,
        list_reference(rest).ok_or(CborError::OutOfMemory)?,
    );
    for index in 0..count {
        let raw = decode_value(memory, vm, &mut reader, 1)?;
        if index < fixed {
            memory.store_u64(slots + index * 8, raw);
        } else {
            push_unchecked(memory, rest, raw);
        }
    }
    Ok(())
}

/// Creates the fatal ArityError of an entry that received more arguments than it declares,
/// with all arguments as a List in its data, like v1.
///
/// # Errors
///
/// Returns why the input could not be decoded.
pub fn arity_error(memory: &mut impl Memory, vm: u32) -> Result<u64, CborError> {
    let (address, length) = input(memory, vm);
    let mut reader = Reader {
        position: address,
        end: address.checked_add(length).ok_or(CborError::Malformed)?,
    };
    let arguments = decode_value(memory, vm, &mut reader, 0)?;
    new_error(
        memory,
        vm,
        true,
        "ArityError",
        "main received more input values than declared parameters",
        arguments,
    )
    .ok_or(CborError::OutOfMemory)
}

/// Decodes one item at nesting depth `depth` into a value.
///
/// Every value decoded so far must be rooted, because decoding allocates.
fn decode_value(
    memory: &mut impl Memory,
    vm: u32,
    reader: &mut Reader,
    depth: u32,
) -> Result<u64, CborError> {
    let initial = reader.byte(memory)?;
    match initial >> 5 {
        MAJOR_ARRAY => return decode_list(memory, vm, reader, initial & 0x1f, depth),
        MAJOR_MAP => return decode_object(memory, vm, reader, initial & 0x1f, depth),
        MAJOR_TAG => {
            return match reader.argument(memory, initial & 0x1f)? {
                ENUM_TAG => decode_enum(memory, vm, reader, depth),
                ERROR_TAG => decode_error(memory, vm, reader, depth),
                _ => Err(CborError::Unsupported),
            };
        }
        _ => {}
    }
    Ok(match reader.scalar(memory, initial)? {
        Scalar::Immediate(value) => value.raw(),
        Scalar::Number(Number::Int(value)) => {
            int_value(memory, vm, value).ok_or(CborError::OutOfMemory)?
        }
        Scalar::Number(Number::Float(value)) => {
            float_value(memory, vm, value).ok_or(CborError::OutOfMemory)?
        }
        Scalar::Text { start, len, chars } => {
            copy_string(memory, vm, start, len, chars).ok_or(CborError::OutOfMemory)?
        }
        Scalar::Bytes { start, len } => new_bytes(memory, vm, len, |memory, position| {
            memory.copy(position, start, len);
        })
        .ok_or(CborError::OutOfMemory)?,
    })
}

/// Decodes the elements of one array with additional information `info` into a new List.
fn decode_list(
    memory: &mut impl Memory,
    vm: u32,
    reader: &mut Reader,
    info: u8,
    depth: u32,
) -> Result<u64, CborError> {
    if depth >= MAX_DEPTH {
        return Err(CborError::Unsupported);
    }
    let count = reader.count(memory, info)?;
    let list = allocate_list(memory, vm, count).ok_or(CborError::OutOfMemory)?;
    let raw = list_reference(list).ok_or(CborError::OutOfMemory)?;
    let root = Vm(vm)
        .push_root(memory, raw)
        .ok_or(CborError::OutOfMemory)?;
    let mut decoded = Ok(raw);
    for _ in 0..count {
        match decode_value(memory, vm, reader, depth + 1) {
            Ok(item) => push_unchecked(memory, list, item),
            Err(error) => {
                decoded = Err(error);
                break;
            }
        }
    }
    Vm(vm).pop_roots(memory, root);
    decoded
}

/// Decodes the one value encoded in the `length` bytes at `address`.
///
/// # Errors
///
/// Returns why the bytes could not be decoded.
pub fn decode_at(
    memory: &mut impl Memory,
    vm: u32,
    address: u32,
    length: u32,
) -> Result<u64, CborError> {
    let mut reader = Reader {
        position: address,
        end: address.checked_add(length).ok_or(CborError::Malformed)?,
    };
    let value = decode_value(memory, vm, &mut reader, 0)?;
    if reader.position == reader.end {
        Ok(value)
    } else {
        Err(CborError::Malformed)
    }
}

/// Reads one unsigned integer item, or `None` for a null item.
fn optional_unsigned(memory: &impl Memory, reader: &mut Reader) -> Result<Option<u64>, CborError> {
    let initial = reader.byte(memory)?;
    if initial == NULL {
        return Ok(None);
    }
    if initial >> 5 != MAJOR_UNSIGNED {
        return Err(CborError::Malformed);
    }
    reader.argument(memory, initial & 0x1f).map(Some)
}

/// Reads one map key, which must be the unsigned integer `expected`.
fn error_key(memory: &impl Memory, reader: &mut Reader, expected: u64) -> Result<(), CborError> {
    match optional_unsigned(memory, reader)? {
        Some(key) if key == expected => Ok(()),
        _ => Err(CborError::Malformed),
    }
}

/// Decodes the seven-field map of a tagged Error: severity, kind, message, data, origin,
/// trace entries and cause, like the runner's codec.
fn decode_error(
    memory: &mut impl Memory,
    vm: u32,
    reader: &mut Reader,
    depth: u32,
) -> Result<u64, CborError> {
    if depth >= MAX_DEPTH {
        return Err(CborError::Unsupported);
    }
    let initial = reader.byte(memory)?;
    if initial >> 5 != MAJOR_MAP || reader.argument(memory, initial & 0x1f)? != 7 {
        return Err(CborError::Malformed);
    }
    error_key(memory, reader, 0)?;
    let fatal = match optional_unsigned(memory, reader)? {
        Some(0) => false,
        Some(1) => true,
        _ => return Err(CborError::Malformed),
    };
    let state = Vm(vm);
    let root = state.root_stack_end(memory);
    let decoded = (|| {
        let mut texts = [0_u64; 2];
        for (key, text) in (1..).zip(&mut texts) {
            error_key(memory, reader, key)?;
            *text = decode_value(memory, vm, reader, depth + 1)?;
            if string_object(*text).is_none() {
                return Err(CborError::Malformed);
            }
            state
                .push_root(memory, *text)
                .ok_or(CborError::OutOfMemory)?;
        }
        error_key(memory, reader, 3)?;
        let data = decode_value(memory, vm, reader, depth + 1)?;
        state
            .push_root(memory, data)
            .ok_or(CborError::OutOfMemory)?;
        error_key(memory, reader, 4)?;
        let origin = optional_unsigned(memory, reader)?
            .map(|origin| u32::try_from(origin).map_err(|_| CborError::Malformed))
            .transpose()?
            .unwrap_or(0);
        error_key(memory, reader, 5)?;
        let trace = decode_trace(memory, vm, reader)?;
        if trace != 0 {
            let reference =
                ValueRef::from_parts(exs_value::layout::type_kind(exs_abi::TYPE_BYTES), 0, trace)
                    .ok_or(CborError::Malformed)?;
            state
                .push_root(memory, reference.raw())
                .ok_or(CborError::OutOfMemory)?;
        }
        error_key(memory, reader, 6)?;
        let cause = if reader.peek(memory)? == NULL {
            reader.byte(memory)?;
            ValueRef::NONE.raw()
        } else {
            decode_value(memory, vm, reader, depth + 1)?
        };
        let parts = crate::errors::ErrorParts {
            fatal,
            kind: texts[0],
            message: texts[1],
            data,
            origin,
            trace: (0, 0),
            cause,
        };
        crate::errors::error_from_parts(memory, vm, &parts, trace).ok_or(CborError::OutOfMemory)
    })();
    state.pop_roots(memory, root);
    decoded
}

/// Decodes the trace entries of an Error into a new Bytes object, or returns 0 for none.
fn decode_trace(memory: &mut impl Memory, vm: u32, reader: &mut Reader) -> Result<u32, CborError> {
    let initial = reader.byte(memory)?;
    if initial >> 5 != MAJOR_ARRAY {
        return Err(CborError::Malformed);
    }
    let count = reader.count(memory, initial & 0x1f)?;
    if count == 0 {
        return Ok(0);
    }
    let len = count.checked_mul(8).ok_or(CborError::Malformed)?;
    let raw = new_bytes(memory, vm, len, |_, _| {}).ok_or(CborError::OutOfMemory)?;
    let object = raw as u32;
    for index in 0..count {
        let initial = reader.byte(memory)?;
        if initial >> 5 != MAJOR_ARRAY || reader.argument(memory, initial & 0x1f)? != 2 {
            return Err(CborError::Malformed);
        }
        let function = optional_unsigned(memory, reader)?.ok_or(CborError::Malformed)?;
        let site = optional_unsigned(memory, reader)?.ok_or(CborError::Malformed)?;
        let entry = (function & 0xffff_ffff) | (site & 0xffff_ffff) << 32;
        memory.store_u64(object + DATA + index * 8, entry);
    }
    Ok(object)
}

/// Reads one text string item and returns the address and length of its bytes.
fn text_item(memory: &impl Memory, reader: &mut Reader) -> Result<(u32, u32), CborError> {
    let initial = reader.byte(memory)?;
    match reader.scalar(memory, initial)? {
        Scalar::Text { start, len, .. } => Ok((start, len)),
        _ => Err(CborError::Malformed),
    }
}

/// Decodes a tagged `[identity, variant, fields]` enum of a type the program declares.
///
/// Enums of identities or variants the program does not know cannot be represented yet.
fn decode_enum(
    memory: &mut impl Memory,
    vm: u32,
    reader: &mut Reader,
    depth: u32,
) -> Result<u64, CborError> {
    if depth >= MAX_DEPTH {
        return Err(CborError::Unsupported);
    }
    let initial = reader.byte(memory)?;
    if initial >> 5 != MAJOR_ARRAY || reader.argument(memory, initial & 0x1f)? != 3 {
        return Err(CborError::Malformed);
    }
    let identity = text_item(memory, reader)?;
    let variant = text_item(memory, reader)?;
    let (type_id, index) =
        resolve_enum(memory, vm, identity, variant).ok_or(CborError::Unsupported)?;
    let initial = reader.byte(memory)?;
    if initial >> 5 != MAJOR_ARRAY {
        return Err(CborError::Malformed);
    }
    let count = reader.count(memory, initial & 0x1f)?;
    let raw = allocate_enum(memory, vm, type_id, index, count).ok_or(CborError::OutOfMemory)?;
    let root = Vm(vm)
        .push_root(memory, raw)
        .ok_or(CborError::OutOfMemory)?;
    let mut decoded = Ok(raw);
    for field in 0..count {
        match decode_value(memory, vm, reader, depth + 1) {
            Ok(value) => memory.store_u64(enum_field(raw as u32, field), value),
            Err(error) => {
                decoded = Err(error);
                break;
            }
        }
    }
    Vm(vm).pop_roots(memory, root);
    decoded
}

/// Finds the enum type whose identity and the variant whose name equal the text at the
/// `(address, length)` ranges, returning the type id and variant index.
pub(crate) fn resolve_enum(
    memory: &impl Memory,
    vm: u32,
    identity: (u32, u32),
    variant: (u32, u32),
) -> Option<(u32, u32)> {
    let same = |object: u32, text: (u32, u32)| {
        crate::strings::text(memory, object).as_bytes() == memory.bytes(text.0, text.1)
    };
    let count = Vm(vm).program_type_count(memory);
    (0..count).find_map(|type_id| {
        let entry = type_entry(memory, vm, type_id)?;
        let name = string_object(entry.identity)?;
        if !same(name, identity) {
            return None;
        }
        (0..entry.field_count).find_map(|index| {
            let candidate = string_object(memory.load_u64(entry.field_names + index * 8))?;
            same(candidate, variant).then_some((type_id, index))
        })
    })
}

/// Decodes the entries of one map with additional information `info` into a new untyped
/// Object, rejecting keys other than text and repeated keys.
fn decode_object(
    memory: &mut impl Memory,
    vm: u32,
    reader: &mut Reader,
    info: u8,
    depth: u32,
) -> Result<u64, CborError> {
    if depth >= MAX_DEPTH {
        return Err(CborError::Unsupported);
    }
    // Every entry takes at least two bytes.
    let count = reader.count(memory, info)?;
    let object = empty_object(memory, vm, count / 2 + 1).ok_or(CborError::OutOfMemory)?;
    let raw = object_reference(object).ok_or(CborError::OutOfMemory)?;
    let root = Vm(vm)
        .push_root(memory, raw)
        .ok_or(CborError::OutOfMemory)?;
    let mut decoded = Ok(raw);
    for _ in 0..count {
        if let Err(error) = decode_entry(memory, vm, reader, object, depth) {
            decoded = Err(error);
            break;
        }
    }
    Vm(vm).pop_roots(memory, root);
    decoded
}

/// Decodes one map entry into the rooted untyped Object at `object`.
fn decode_entry(
    memory: &mut impl Memory,
    vm: u32,
    reader: &mut Reader,
    object: u32,
    depth: u32,
) -> Result<(), CborError> {
    let initial = reader.byte(memory)?;
    if initial >> 5 != MAJOR_TEXT {
        return Err(CborError::Unsupported);
    }
    let Scalar::Text { start, len, chars } = reader.scalar(memory, initial)? else {
        return Err(CborError::Malformed);
    };
    let name = copy_string(memory, vm, start, len, chars).ok_or(CborError::OutOfMemory)?;
    let state = Vm(vm);
    let root = state
        .push_root(memory, name)
        .ok_or(CborError::OutOfMemory)?;
    let result = decode_value(memory, vm, reader, depth + 1).and_then(|value| {
        if find_value(memory, object, name as u32).is_some() {
            return Err(CborError::Malformed);
        }
        state
            .push_root(memory, value)
            .ok_or(CborError::OutOfMemory)?;
        object_set(memory, vm, object, name, value).ok_or(CborError::OutOfMemory)
    });
    state.pop_roots(memory, root);
    result
}

/// Converts one IEEE 754 binary16 value to binary64.
fn half_to_f64(bits: u16) -> f64 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let fraction = f64::from(bits & 0x3ff);
    sign * match exponent {
        0 => fraction * f64::from_bits(0x3E70_0000_0000_0000),
        31 if fraction == 0.0 => f64::INFINITY,
        31 => f64::NAN,
        _ => (1.0 + fraction / 1024.0) * power_of_two(exponent - 15),
    }
}

/// Returns `2^exponent` for exponents of normal binary64 values.
fn power_of_two(exponent: i32) -> f64 {
    f64::from_bits(((exponent + 1023) as u64) << 52)
}

/// Returns the character count of `len` bytes of UTF-8 at `start`, or `None` when invalid.
fn utf8_chars(memory: &impl Memory, start: u32, len: u32) -> Option<u32> {
    let byte = |offset: u32| memory.load_u8(start + offset);
    let continuation = |offset: u32| offset < len && byte(offset) & 0xc0 == 0x80;
    let (mut offset, mut chars) = (0, 0);
    while offset < len {
        let first = byte(offset);
        let width = match first {
            0x00..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => return None,
        };
        if (1..width).any(|index| !continuation(offset + index)) {
            return None;
        }
        // Reject overlong encodings, surrogates and code points above U+10FFFF.
        let second = if width > 1 { byte(offset + 1) } else { 0 };
        let valid = match (first, width) {
            (0xe0, 3) => second >= 0xa0,
            (0xed, 3) => second < 0xa0,
            (0xf0, 4) => second >= 0x90,
            (0xf4, 4) => second < 0x90,
            _ => true,
        };
        if !valid {
            return None;
        }
        offset += width;
        chars += 1;
    }
    Some(chars)
}

/// Encodes one result value into the VM's result buffer.
///
/// A result that cannot cross the boundary, such as a closure or a cyclic graph, is encoded as
/// v1's fatal SerializationError instead.
///
/// # Errors
///
/// Returns why the value could not be encoded.
pub fn encode_result(memory: &mut impl Memory, vm: u32, raw: u64) -> Result<(), CborError> {
    match encode(memory, vm, raw) {
        Err(CborError::Unsupported) => {
            let error = crate::boundary::serialization_error(memory, vm, raw, true)
                .ok_or(CborError::OutOfMemory)?;
            encode(memory, vm, error)
        }
        outcome => outcome,
    }
}

/// Encodes `raw` into the result buffer in two passes, measuring and then writing.
pub(crate) fn encode(memory: &mut impl Memory, vm: u32, raw: u64) -> Result<(), CborError> {
    let mut measure = Encoder(Buffer {
        start: None,
        len: 0,
    });
    measure.value(memory, vm, raw, 0)?;
    let address = reserve_result(memory, vm, measure.0.len).ok_or(CborError::OutOfMemory)?;
    let mut write = Encoder(Buffer {
        start: Some(address),
        len: 0,
    });
    write.value(memory, vm, raw, 0)
}

/// Encodes `raw`, a value in `memory` of the VM at `vm`, into `output`.
///
/// The runner reads results and host-call arguments of runtime v2 modules this way, on a copy
/// of their memory: every read stays inside `memory`, nesting is bounded and `output` bounds
/// the bytes, so even a corrupted heap cannot make the walk unbounded.
///
/// # Errors
///
/// Returns why the value could not be encoded; `Unsupported` for a value that cannot cross
/// the boundary.
pub fn encode_value<M: Memory>(
    memory: &mut M,
    vm: u32,
    raw: u64,
    output: &mut impl CborOutput<M>,
) -> Result<(), CborError> {
    Encoder(output).value(memory, vm, raw, 0)
}

/// Destination of the encoder's bytes.
pub trait CborOutput<M: Memory> {
    /// Appends `bytes`.
    ///
    /// # Errors
    ///
    /// Returns an error when the output cannot take more bytes.
    fn bytes(&mut self, memory: &mut M, bytes: &[u8]) -> Result<(), CborError>;

    /// Appends the `len` bytes at `source` in `memory`.
    ///
    /// # Errors
    ///
    /// Returns an error when the output cannot take more bytes or the range is invalid.
    fn copy(&mut self, memory: &mut M, source: u32, len: u32) -> Result<(), CborError>;
}

impl<M: Memory, O: CborOutput<M>> CborOutput<M> for &mut O {
    fn bytes(&mut self, memory: &mut M, bytes: &[u8]) -> Result<(), CborError> {
        (**self).bytes(memory, bytes)
    }

    fn copy(&mut self, memory: &mut M, source: u32, len: u32) -> Result<(), CborError> {
        (**self).copy(memory, source, len)
    }
}

/// The result buffer as an encoding destination: it only measures, or it writes into reserved
/// bytes.
struct Buffer {
    /// Address of the first output byte, or `None` while measuring.
    start: Option<u32>,
    /// Bytes produced so far.
    len: u32,
}

impl Buffer {
    /// Accounts for `count` more output bytes.
    fn advance(&mut self, count: u32) -> Result<(), CborError> {
        self.len = self.len.checked_add(count).ok_or(CborError::OutOfMemory)?;
        Ok(())
    }
}

impl<M: Memory> CborOutput<M> for Buffer {
    fn bytes(&mut self, memory: &mut M, bytes: &[u8]) -> Result<(), CborError> {
        if let Some(start) = self.start {
            for (offset, byte) in (self.len..).zip(bytes) {
                memory.store_u8(start + offset, *byte);
            }
        }
        self.advance(bytes.len() as u32)
    }

    fn copy(&mut self, memory: &mut M, source: u32, len: u32) -> Result<(), CborError> {
        if let Some(start) = self.start {
            memory.copy(start + self.len, source, len);
        }
        self.advance(len)
    }
}

/// The CBOR encoder writing into one output.
struct Encoder<O>(O);

impl<O> Encoder<O> {
    /// Appends the shortest head of `major` with `argument`.
    fn head<M: Memory>(&mut self, memory: &mut M, major: u8, argument: u64) -> Result<(), CborError>
    where
        O: CborOutput<M>,
    {
        let mut bytes = [0_u8; 9];
        let length = encode_head(&mut bytes, major, argument) as usize;
        self.0.bytes(memory, &bytes[..length])
    }

    /// Appends the encoding of one value at nesting depth `depth`.
    fn value<M: Memory>(
        &mut self,
        memory: &mut M,
        vm: u32,
        raw: u64,
        depth: u32,
    ) -> Result<(), CborError>
    where
        O: CborOutput<M>,
    {
        if let Some(object) = string_object(raw) {
            return self.string(memory, object);
        }
        if let Some(object) = bytes_object(raw) {
            let len = bytes_len(memory, object);
            self.head(memory, MAJOR_BYTES, u64::from(len))?;
            return self.0.copy(memory, object + DATA, len);
        }
        if let Some(list) = list_object(raw) {
            return self.list(memory, vm, list, depth);
        }
        if let Some(object) = typed_object(memory, raw) {
            return self.typed_object(memory, vm, object, depth);
        }
        if let Some(object) = error_object(raw) {
            return self.error(memory, vm, object, depth);
        }
        if let Some(object) = enum_object(memory, raw) {
            return self.enumeration(memory, vm, object, depth);
        }
        if let Some(object) = untyped_object(memory, raw) {
            if depth >= MAX_DEPTH {
                return Err(CborError::Unsupported);
            }
            let count = key_count(memory, shape(memory, object));
            self.head(memory, MAJOR_MAP, u64::from(count))?;
            return self.entries(memory, vm, object, depth);
        }
        let mut bytes = [0_u8; 9];
        let length = match reference(raw).and_then(ValueRef::immediate) {
            Some(Immediate::None) => {
                bytes[0] = NULL;
                1
            }
            Some(Immediate::Bool(flag)) => {
                bytes[0] = if flag { TRUE } else { FALSE };
                1
            }
            _ => match number(memory, raw).ok_or(CborError::Unsupported)? {
                Number::Int(value) => encode_int(&mut bytes, value),
                Number::Float(value) => {
                    bytes[0] = FLOAT64;
                    bytes[1..].copy_from_slice(&value.to_bits().to_be_bytes());
                    9
                }
            },
        };
        self.0.bytes(memory, &bytes[..length as usize])
    }

    /// Appends the String at `object` as a text string.
    fn string<M: Memory>(&mut self, memory: &mut M, object: u32) -> Result<(), CborError>
    where
        O: CborOutput<M>,
    {
        let len = byte_len(memory, object);
        self.head(memory, MAJOR_TEXT, u64::from(len))?;
        self.0.copy(memory, object + DATA, len)
    }

    /// Appends the List at `list` as an array.
    fn list<M: Memory>(
        &mut self,
        memory: &mut M,
        vm: u32,
        list: u32,
        depth: u32,
    ) -> Result<(), CborError>
    where
        O: CborOutput<M>,
    {
        if depth >= MAX_DEPTH {
            return Err(CborError::Unsupported);
        }
        let count = len(memory, list);
        self.head(memory, MAJOR_ARRAY, u64::from(count))?;
        for index in 0..count {
            let item = memory.load_u64(item_address(memory, list, index));
            self.value(memory, vm, item, depth + 1)?;
        }
        Ok(())
    }

    /// Appends the typed Object at `object` as a map of its declared fields in entry order.
    fn typed_object<M: Memory>(
        &mut self,
        memory: &mut M,
        vm: u32,
        object: u32,
        depth: u32,
    ) -> Result<(), CborError>
    where
        O: CborOutput<M>,
    {
        if depth >= MAX_DEPTH {
            return Err(CborError::Unsupported);
        }
        let entry = type_entry(memory, vm, type_id(memory, object))
            .filter(|entry| entry.field_count <= slot_count(memory, object))
            .ok_or(CborError::Unsupported)?;
        let extra = extra(memory, object);
        let extra_count = if extra == 0 {
            0
        } else {
            key_count(memory, shape(memory, extra))
        };
        // Declared fields removed by `delete` or `clear` hold an empty slot and are skipped.
        let present = (0..entry.field_count)
            .filter(|index| memory.load_u64(slot_address(object, *index)) != 0)
            .count() as u32;
        let count = present.checked_add(extra_count);
        self.head(
            memory,
            MAJOR_MAP,
            u64::from(count.ok_or(CborError::Unsupported)?),
        )?;
        for position in 0..entry.field_count {
            let index = ordered_slot(memory, object, position);
            let value = memory.load_u64(slot_address(object, index));
            if value == 0 {
                continue;
            }
            let name = memory.load_u64(entry.field_names + index * 8);
            let name = string_object(name).ok_or(CborError::Unsupported)?;
            self.string(memory, name)?;
            self.value(memory, vm, value, depth + 1)?;
        }
        if extra != 0 {
            self.entries(memory, vm, extra, depth)?;
        }
        Ok(())
    }

    /// Appends the enum at `object` as the codec's tagged `[identity, variant, fields]` array.
    fn enumeration<M: Memory>(
        &mut self,
        memory: &mut M,
        vm: u32,
        object: u32,
        depth: u32,
    ) -> Result<(), CborError>
    where
        O: CborOutput<M>,
    {
        if depth >= MAX_DEPTH {
            return Err(CborError::Unsupported);
        }
        let (identity, name) = names(memory, vm, object).ok_or(CborError::Unsupported)?;
        self.head(memory, MAJOR_TAG, ENUM_TAG)?;
        self.head(memory, MAJOR_ARRAY, 3)?;
        self.string(memory, identity)?;
        self.string(memory, name)?;
        let count = field_count(memory, object);
        self.head(memory, MAJOR_ARRAY, u64::from(count))?;
        for index in 0..count {
            let value = memory.load_u64(enum_field(object, index));
            self.value(memory, vm, value, depth + 1)?;
        }
        Ok(())
    }

    /// Appends the Error at `object` as the tagged seven-field map of the runner's codec.
    fn error<M: Memory>(
        &mut self,
        memory: &mut M,
        vm: u32,
        object: u32,
        depth: u32,
    ) -> Result<(), CborError>
    where
        O: CborOutput<M>,
    {
        if depth >= MAX_DEPTH {
            return Err(CborError::Unsupported);
        }
        let parts = error_parts(memory, object);
        self.head(memory, MAJOR_TAG, ERROR_TAG)?;
        self.head(memory, MAJOR_MAP, 7)?;
        self.head(memory, MAJOR_UNSIGNED, 0)?;
        self.head(memory, MAJOR_UNSIGNED, u64::from(parts.fatal))?;
        for (key, text) in [(1, parts.kind), (2, parts.message)] {
            self.head(memory, MAJOR_UNSIGNED, key)?;
            self.string(memory, string_object(text).ok_or(CborError::Unsupported)?)?;
        }
        self.head(memory, MAJOR_UNSIGNED, 3)?;
        self.value(memory, vm, parts.data, depth + 1)?;
        self.head(memory, MAJOR_UNSIGNED, 4)?;
        if parts.origin == 0 {
            self.0.bytes(memory, &[NULL])?;
        } else {
            self.head(memory, MAJOR_UNSIGNED, u64::from(parts.origin))?;
        }
        self.head(memory, MAJOR_UNSIGNED, 5)?;
        let (entries, count) = parts.trace;
        self.head(memory, MAJOR_ARRAY, u64::from(count))?;
        for index in 0..count {
            let entry = memory.load_u64(entries + index * 8);
            self.head(memory, MAJOR_ARRAY, 2)?;
            self.head(memory, MAJOR_UNSIGNED, entry & 0xffff_ffff)?;
            self.head(memory, MAJOR_UNSIGNED, entry >> 32)?;
        }
        self.head(memory, MAJOR_UNSIGNED, 6)?;
        if parts.cause == ValueRef::NONE.raw() {
            self.0.bytes(memory, &[NULL])
        } else {
            self.value(memory, vm, parts.cause, depth + 1)
        }
    }

    /// Appends the keys and values of the untyped Object at `object` in key order.
    fn entries<M: Memory>(
        &mut self,
        memory: &mut M,
        vm: u32,
        object: u32,
        depth: u32,
    ) -> Result<(), CborError>
    where
        O: CborOutput<M>,
    {
        let shape = shape(memory, object);
        for index in 0..key_count(memory, shape) {
            let name = string_object(key(memory, shape, index)).ok_or(CborError::Unsupported)?;
            self.string(memory, name)?;
            let value = memory.load_u64(value_address(memory, object, index));
            self.value(memory, vm, value, depth + 1)?;
        }
        Ok(())
    }
}

/// Writes the shortest CBOR encoding of one Int and returns its length.
fn encode_int(bytes: &mut [u8; 9], value: i64) -> u32 {
    if value < 0 {
        encode_head(bytes, MAJOR_NEGATIVE, (-1 - value).cast_unsigned())
    } else {
        encode_head(bytes, MAJOR_UNSIGNED, value.cast_unsigned())
    }
}

/// Writes the shortest CBOR head of `major` with `argument` and returns its length.
fn encode_head(bytes: &mut [u8; 9], major: u8, argument: u64) -> u32 {
    let (info, width) = match argument {
        0..=23 => (argument as u8, 0),
        24..=0xff => (24, 1),
        0x100..=0xffff => (25, 2),
        0x1_0000..=0xffff_ffff => (26, 4),
        _ => (27, 8),
    };
    bytes[0] = (major << 5) | info;
    for index in 0..width {
        bytes[1 + index] = (argument >> (8 * (width - 1 - index))) as u8;
    }
    1 + width as u32
}
