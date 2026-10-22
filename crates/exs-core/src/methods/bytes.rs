//! Bytes methods and constructors with the results, Errors and order of checks.

use exs_layout::ValueRef;

use crate::bytes::{bytes_len, bytes_object, new_bytes, octets};
use crate::errors::recoverable;
use crate::lists::{allocate_list, item_address, len, list_object, list_reference, set_len};
use crate::memory::Memory;
use crate::methods::numbers::{exact_int, type_error};
use crate::strings::{DATA, copy_string_object, string_object, text};

/// Unwraps a check, returning its Error value from the enclosing method.
macro_rules! check {
    ($result:expr) => {
        match $result {
            Ok(value) => value,
            Err(error) => return error,
        }
    };
}

/// Reads the Bytes receiver of a method, or creates the TypeError `message`.
fn receiver(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
    message: &str,
) -> Result<u32, Option<u64>> {
    bytes_object(raw).ok_or_else(|| type_error(memory, vm, message, raw))
}

/// Reads a Bytes index; `slice` indexes report their own message.
fn bytes_index(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
    slice: bool,
) -> Result<u32, Option<u64>> {
    match exact_int(memory, raw) {
        Some(index) if index >= 0 => u32::try_from(index).map_err(|_| {
            recoverable(
                memory,
                vm,
                "IndexError",
                "Bytes index is outside the supported range",
                raw,
            )
        }),
        _ => Err(recoverable(
            memory,
            vm,
            "IndexError",
            if slice {
                "Bytes slice indexes require non-negative Int values"
            } else {
                "Bytes index requires a non-negative Int value"
            },
            raw,
        )),
    }
}

/// Evaluates `bytes[index]`: the octet as an Int.
pub(crate) fn get(memory: &mut impl Memory, vm: u32, raw: u64, index: u64) -> Option<u64> {
    let position = check!(bytes_index(memory, vm, index, false));
    let object = check!(receiver(
        memory,
        vm,
        raw,
        "index access requires a Bytes receiver"
    ));
    match octets(memory, object).get(position as usize) {
        Some(octet) => Some(ValueRef::small_int(i64::from(*octet))?.raw()),
        None => recoverable(
            memory,
            vm,
            "IndexError",
            "Bytes index is outside the Bytes bounds",
            raw,
        ),
    }
}

/// Copies the octets `start..end` of the Bytes at `object` into new Bytes.
fn sub_bytes(memory: &mut impl Memory, vm: u32, object: u32, start: u32, end: u32) -> Option<u64> {
    new_bytes(memory, vm, end - start, |memory, position| {
        memory.copy(position, object + DATA + start, end - start);
    })
}

/// Evaluates `slice(start, end)` on Bytes.
pub(crate) fn slice(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
    start: u64,
    stop: u64,
) -> Option<u64> {
    let start = check!(bytes_index(memory, vm, start, true));
    let stop = check!(bytes_index(memory, vm, stop, true));
    let object = check!(receiver(memory, vm, raw, "slice requires a Bytes receiver"));
    if start > stop || stop > bytes_len(memory, object) {
        return recoverable(
            memory,
            vm,
            "IndexError",
            "Bytes slice range is outside the Bytes bounds",
            raw,
        );
    }
    sub_bytes(memory, vm, object, start, stop)
}

/// Evaluates `concat(other)`: the octets of both Bytes in new Bytes.
pub(crate) fn concat(memory: &mut impl Memory, vm: u32, raw: u64, other: u64) -> Option<u64> {
    let object = check!(receiver(
        memory,
        vm,
        raw,
        "concat requires a Bytes receiver"
    ));
    let Some(other_object) = bytes_object(other) else {
        return type_error(memory, vm, "concat requires a Bytes argument", other);
    };
    let (left, right) = (bytes_len(memory, object), bytes_len(memory, other_object));
    new_bytes(memory, vm, left.checked_add(right)?, |memory, position| {
        memory.copy(position, object + DATA, left);
        memory.copy(position + left, other_object + DATA, right);
    })
}

/// Evaluates `to_list`: the octets as a List of Ints.
pub(crate) fn to_list(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<u64> {
    let object = check!(receiver(
        memory,
        vm,
        raw,
        "to_list requires a Bytes receiver"
    ));
    let count = bytes_len(memory, object);
    let list = allocate_list(memory, vm, count)?;
    for index in 0..count {
        let octet = memory.load_u8(object + DATA + index);
        let value = ValueRef::small_int(i64::from(octet))?.raw();
        memory.store_u64(item_address(memory, list, index), value);
    }
    set_len(memory, list, count);
    list_reference(list)
}

/// Evaluates `decode_utf8`: the octets as a String, or an EncodingError.
pub(crate) fn decode_utf8(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<u64> {
    let object = check!(receiver(
        memory,
        vm,
        raw,
        "decode_utf8 requires a Bytes receiver"
    ));
    let Ok(decoded) = core::str::from_utf8(octets(memory, object)) else {
        return recoverable(
            memory,
            vm,
            "EncodingError",
            "Bytes do not contain valid UTF-8",
            raw,
        );
    };
    let (len, chars) = (decoded.len() as u32, decoded.chars().count() as u32);
    copy_string_object(memory, vm, len, chars, |memory, position| {
        memory.copy(position, object + DATA, len);
    })
}

/// Evaluates `Bytes::from_utf8(text)` and `text.encode_utf8()`.
pub(crate) fn from_utf8(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<u64> {
    let Some(object) = string_object(raw) else {
        return type_error(memory, vm, "Bytes::from_utf8 requires a String value", raw);
    };
    let len = text(memory, object).len() as u32;
    new_bytes(memory, vm, len, |memory, position| {
        memory.copy(position, object + DATA, len);
    })
}

/// A Bytes predicate between a receiver and an argument.
#[derive(Clone, Copy)]
pub(crate) enum Relation {
    /// `contains`.
    Contains,
    /// `starts_with`.
    StartsWith,
    /// `ends_with`.
    EndsWith,
}

/// Evaluates `contains`, `starts_with` or `ends_with` on Bytes.
pub(crate) fn relation(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
    other: u64,
    relation: Relation,
) -> Option<u64> {
    let (receiver_message, argument_message) = match relation {
        Relation::Contains => (
            "contains requires a Bytes receiver",
            "contains requires a Bytes argument",
        ),
        Relation::StartsWith => (
            "starts_with requires a Bytes receiver",
            "starts_with requires a Bytes argument",
        ),
        Relation::EndsWith => (
            "ends_with requires a Bytes receiver",
            "ends_with requires a Bytes argument",
        ),
    };
    let object = check!(receiver(memory, vm, raw, receiver_message));
    let Some(other_object) = bytes_object(other) else {
        return type_error(memory, vm, argument_message, other);
    };
    let (value, needle) = (octets(memory, object), octets(memory, other_object));
    let result = match relation {
        Relation::Contains => {
            needle.is_empty() || value.windows(needle.len()).any(|window| window == needle)
        }
        Relation::StartsWith => value.starts_with(needle),
        Relation::EndsWith => value.ends_with(needle),
    };
    Some(ValueRef::boolean(result).raw())
}

/// Evaluates `repeat(count)` on Bytes.
pub(crate) fn repeat(memory: &mut impl Memory, vm: u32, raw: u64, count: u64) -> Option<u64> {
    let count = match exact_int(memory, count) {
        Some(value) if value >= 0 => match u32::try_from(value) {
            Ok(value) => value,
            Err(_) => {
                return recoverable(
                    memory,
                    vm,
                    "ValueError",
                    "repeat count is outside the supported range",
                    count,
                );
            }
        },
        _ => {
            return type_error(
                memory,
                vm,
                "repeat requires a non-negative Int count",
                count,
            );
        }
    };
    let object = check!(receiver(
        memory,
        vm,
        raw,
        "repeat requires a Bytes receiver"
    ));
    let len = bytes_len(memory, object);
    let Some(total) = len.checked_mul(count) else {
        return recoverable(
            memory,
            vm,
            "ValueError",
            "repeated Bytes are too large",
            raw,
        );
    };
    new_bytes(memory, vm, total, |memory, position| {
        for index in 0..count {
            memory.copy(position + index * len, object + DATA, len);
        }
    })
}

/// Lowercase hexadecimal digits.
const HEX: &[u8; 16] = b"0123456789abcdef";
/// The RFC 4648 base64 alphabet.
const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Evaluates `to_hex`: lowercase hexadecimal text.
pub(crate) fn to_hex(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<u64> {
    let object = check!(receiver(
        memory,
        vm,
        raw,
        "to_hex requires a Bytes receiver"
    ));
    let len = bytes_len(memory, object);
    let Some(total) = len.checked_mul(2) else {
        return recoverable(memory, vm, "ValueError", "hex output is too large", raw);
    };
    copy_string_object(memory, vm, total, total, |memory, position| {
        for index in 0..len {
            let octet = memory.load_u8(object + DATA + index);
            memory.store_u8(position + index * 2, HEX[usize::from(octet >> 4)]);
            memory.store_u8(position + index * 2 + 1, HEX[usize::from(octet & 0x0f)]);
        }
    })
}

/// Evaluates `to_base64`: padded RFC 4648 text.
pub(crate) fn to_base64(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<u64> {
    let object = check!(receiver(
        memory,
        vm,
        raw,
        "to_base64 requires a Bytes receiver"
    ));
    let len = bytes_len(memory, object);
    let Some(total) = len.checked_add(2).map(|len| len / 3 * 4) else {
        return recoverable(memory, vm, "ValueError", "base64 output is too large", raw);
    };
    copy_string_object(memory, vm, total, total, |memory, position| {
        let mut write = position;
        let mut index = 0;
        while index < len {
            let octet = |offset: u32| {
                (index + offset < len).then(|| memory.load_u8(object + DATA + index + offset))
            };
            let (first, second, third) = (octet(0).unwrap_or(0), octet(1), octet(2));
            let symbols = [
                BASE64[usize::from(first >> 2)],
                BASE64[usize::from(((first & 0x03) << 4) | (second.unwrap_or(0) >> 4))],
                second.map_or(b'=', |second| {
                    BASE64[usize::from(((second & 0x0f) << 2) | (third.unwrap_or(0) >> 6))]
                }),
                third.map_or(b'=', |third| BASE64[usize::from(third & 0x3f)]),
            ];
            for (offset, symbol) in (0..).zip(symbols) {
                memory.store_u8(write + offset, symbol);
            }
            write += 4;
            index += 3;
        }
    })
}

/// Evaluates `Bytes::from_list(values)`: Ints from 0 to 255 as octets.
pub(crate) fn from_list(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<u64> {
    let Some(list) = list_object(raw) else {
        return type_error(memory, vm, "Bytes::from_list requires a List receiver", raw);
    };
    let count = len(memory, list);
    for index in 0..count {
        let value = memory.load_u64(item_address(memory, list, index));
        match exact_int(memory, value) {
            Some(octet) if (0..=255).contains(&octet) => {}
            Some(_) => {
                return recoverable(
                    memory,
                    vm,
                    "ValueError",
                    "Bytes::from_list values must be between 0 and 255",
                    value,
                );
            }
            None => {
                return type_error(
                    memory,
                    vm,
                    "Bytes::from_list requires a List of Int values",
                    value,
                );
            }
        }
    }
    new_bytes(memory, vm, count, |memory, position| {
        for index in 0..count {
            let value = memory.load_u64(item_address(memory, list, index));
            memory.store_u8(
                position + index,
                exact_int(memory, value).unwrap_or(0) as u8,
            );
        }
    })
}

/// Decodes one hexadecimal ASCII digit.
const fn hex_value(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

/// Evaluates `Bytes::from_hex(text)`.
pub(crate) fn from_hex(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<u64> {
    let Some(object) = string_object(raw) else {
        return type_error(memory, vm, "Bytes::from_hex requires a String value", raw);
    };
    let digits = text(memory, object).as_bytes();
    if digits.len() % 2 != 0 {
        return recoverable(
            memory,
            vm,
            "ParseError",
            "hex input must contain an even number of digits",
            raw,
        );
    }
    if !digits.iter().all(|digit| hex_value(*digit).is_some()) {
        return recoverable(
            memory,
            vm,
            "ParseError",
            "hex input contains an invalid digit",
            raw,
        );
    }
    let count = (digits.len() / 2) as u32;
    new_bytes(memory, vm, count, |memory, position| {
        for index in 0..count {
            let high = hex_value(memory.load_u8(object + DATA + index * 2)).unwrap_or(0);
            let low = hex_value(memory.load_u8(object + DATA + index * 2 + 1)).unwrap_or(0);
            memory.store_u8(position + index, (high << 4) | low);
        }
    })
}

/// Decodes one base64 ASCII symbol.
const fn base64_value(symbol: u8) -> Option<u8> {
    match symbol {
        b'A'..=b'Z' => Some(symbol - b'A'),
        b'a'..=b'z' => Some(symbol - b'a' + 26),
        b'0'..=b'9' => Some(symbol - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Decodes one four-symbol base64 group, returning its octets and their count.
fn base64_group(group: [u8; 4], final_group: bool) -> Option<([u8; 3], u32)> {
    let first = base64_value(group[0])?;
    let second = base64_value(group[1])?;
    let third = if group[2] == b'=' {
        None
    } else {
        base64_value(group[2])
    };
    let fourth = if group[3] == b'=' {
        None
    } else {
        base64_value(group[3])
    };
    if (!final_group && (group[2] == b'=' || group[3] == b'='))
        || (third.is_none() && fourth.is_some())
        || (group[2] != b'=' && third.is_none())
        || (group[3] != b'=' && fourth.is_none())
    {
        return None;
    }
    let (third, fourth) = (third.unwrap_or(0), fourth.unwrap_or(0));
    if (group[2] == b'=' && second & 0x0f != 0)
        || (group[3] == b'=' && group[2] != b'=' && third & 0x03 != 0)
    {
        return None;
    }
    let octets = [
        (first << 2) | (second >> 4),
        (second << 4) | (third >> 2),
        (third << 6) | fourth,
    ];
    let count = 1 + u32::from(group[2] != b'=') + u32::from(group[3] != b'=');
    Some((octets, count))
}

/// Returns the four symbols of base64 group `index` of `digits`.
fn group(digits: &[u8], index: usize) -> [u8; 4] {
    let mut group = [0_u8; 4];
    for (offset, symbol) in group.iter_mut().enumerate() {
        *symbol = digits.get(index * 4 + offset).copied().unwrap_or(0);
    }
    group
}

/// Evaluates `Bytes::from_base64(text)` for padded RFC 4648 text.
pub(crate) fn from_base64(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<u64> {
    let Some(object) = string_object(raw) else {
        return type_error(
            memory,
            vm,
            "Bytes::from_base64 requires a String value",
            raw,
        );
    };
    let digits = text(memory, object).as_bytes();
    if digits.len() % 4 != 0 {
        return recoverable(
            memory,
            vm,
            "ParseError",
            "base64 input must have a length divisible by four",
            raw,
        );
    }
    let groups = digits.len() / 4;
    let mut total = 0_u32;
    for index in 0..groups {
        match base64_group(group(digits, index), index + 1 == groups) {
            Some((_, count)) => total += count,
            None => {
                return recoverable(
                    memory,
                    vm,
                    "ParseError",
                    "base64 input is not valid padded RFC 4648 text",
                    raw,
                );
            }
        }
    }
    new_bytes(memory, vm, total, |memory, position| {
        let mut write = position;
        for index in 0..groups {
            let digits = text(memory, object).as_bytes();
            if let Some((octets, count)) = base64_group(group(digits, index), index + 1 == groups) {
                for (offset, octet) in (0..count).zip(octets) {
                    memory.store_u8(write + offset, octet);
                }
                write += count;
            }
        }
    })
}
