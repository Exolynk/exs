//! Dynamic Wasm operations shared across runtime value kinds.

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::str::FromStr;

use exs_abi::{
    STANDARD_ITERATOR_STEP_TYPE_IDENTITY, STANDARD_ORDERING_TYPE_ID,
    STANDARD_ORDERING_TYPE_IDENTITY,
};
use exs_value::ValueRef;

use crate::gc;
use crate::runtime;
use crate::value::{
    RtValue, RuntimeBytes, RuntimeEnum, RuntimeList, RuntimeObject, RuntimeString, clone, list,
    numeric, object,
};

/// Adds two runtime values through String, List, or numeric dispatch.
pub(crate) fn add(left: ValueRef, right: ValueRef) -> ValueRef {
    match runtime::value(left) {
        RtValue::String(value) => string_add(value, right),
        RtValue::List(_) => list::operations::add(left, right),
        _ => numeric::arithmetic(left, right, i64::checked_add, |left, right| left + right),
    }
}

/// Subtracts two runtime numeric values.
pub(crate) fn subtract(left: ValueRef, right: ValueRef) -> ValueRef {
    numeric::arithmetic(left, right, i64::checked_sub, |left, right| left - right)
}

/// Multiplies two runtime numeric values.
pub(crate) fn multiply(left: ValueRef, right: ValueRef) -> ValueRef {
    numeric::arithmetic(left, right, i64::checked_mul, |left, right| left * right)
}

/// Divides two runtime numeric values and always returns a Float.
pub(crate) fn divide(left: ValueRef, right: ValueRef) -> ValueRef {
    numeric::divide(left, right)
}

/// Concatenates a String receiver with one supported scalar right operand.
fn string_add(left: &RuntimeString, right: ValueRef) -> ValueRef {
    let right = match runtime::value(right) {
        RtValue::String(value) => String::from(value.as_str()),
        RtValue::Bool(value) => value.to_string(),
        RtValue::Int(value) => value.to_string(),
        RtValue::Float(value) => value.to_string(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "String addition requires a String, Bool, Int, or Float right operand",
                right,
            );
        }
    };
    let mut result = String::from(left.as_str());
    result.push_str(&right);
    string_value_result(result)
}

/// Tests two runtime values for equality.
pub(crate) fn equal(left: ValueRef, right: ValueRef) -> ValueRef {
    runtime::allocate(RtValue::Bool(values_equal(left, right)))
}

/// Compares two runtime values and returns the compiler-owned Ordering enum.
pub(crate) fn compare(left: ValueRef, right: ValueRef) -> ValueRef {
    let ordering = match (runtime::value(left), runtime::value(right)) {
        (left, right) if numeric::is_numeric(left) && numeric::is_numeric(right) => {
            match (numeric::number_of(left), numeric::number_of(right)) {
                (Some(left), Some(right)) => match numeric::numbers_comparison(left, right) {
                    numeric::Comparison::Less => "Less",
                    numeric::Comparison::Equal => "Equal",
                    numeric::Comparison::Greater => "Greater",
                    numeric::Comparison::Unordered => "Unordered",
                },
                _ => "Unordered",
            }
        }
        (RtValue::String(left), RtValue::String(right)) => {
            match left.as_str().cmp(right.as_str()) {
                core::cmp::Ordering::Less => "Less",
                core::cmp::Ordering::Equal => "Equal",
                core::cmp::Ordering::Greater => "Greater",
            }
        }
        _ if values_equal(left, right) => "Equal",
        _ => "Unordered",
    };
    ordering_value(ordering)
}

/// Interprets one compiler-owned Ordering value for a source comparison operator.
pub(crate) fn ordering_test(ordering: ValueRef, test: i32) -> ValueRef {
    let variant = match runtime::value(ordering) {
        RtValue::Object(object)
            if object.type_id == Some(STANDARD_ORDERING_TYPE_ID)
                && object.enum_data.as_ref().is_some_and(|data| {
                    data.type_identity.as_ref() == STANDARD_ORDERING_TYPE_IDENTITY
                }) =>
        {
            object.enum_data.as_ref().map(|data| data.variant.as_ref())
        }
        _ => None,
    };
    let Some(variant) = variant else {
        return runtime::recoverable_error(
            "TypeError",
            "Compare implementations must return an Ordering value",
            ordering,
        );
    };
    let result = match (variant, test) {
        ("Equal", 0) => true,
        ("Unordered", 1) => true,
        ("Less" | "Greater", 1) => true,
        ("Less", 2 | 3) => true,
        ("Equal", 3 | 5) => true,
        ("Greater", 4 | 5) => true,
        ("Unordered", 0) => false,
        ("Unordered", 2..=5) => {
            return runtime::recoverable_error(
                "TypeError",
                "ordering comparison requires comparable values",
                ordering,
            );
        }
        ("Less" | "Greater", 0) | ("Equal", 1 | 2 | 4) | ("Less", 4 | 5) | ("Greater", 2 | 3) => {
            false
        }
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "invalid Ordering value or comparison operator",
                ordering,
            );
        }
    };
    runtime::allocate(RtValue::Bool(result))
}

/// Returns whether two runtime values satisfy ExS equality semantics without allocating a Bool.
pub(crate) fn values_equal(left: ValueRef, right: ValueRef) -> bool {
    match (runtime::value(left), runtime::value(right)) {
        (RtValue::None, RtValue::None) => true,
        (left, right) if numeric::is_numeric(left) && numeric::is_numeric(right) => {
            match (numeric::number_of(left), numeric::number_of(right)) {
                (Some(left), Some(right)) => numeric::numbers_equal(left, right),
                _ => false,
            }
        }
        (RtValue::String(left), RtValue::String(right)) => left.as_str() == right.as_str(),
        (RtValue::Bytes(left), RtValue::Bytes(right)) => left.as_slice() == right.as_slice(),
        (RtValue::List(_), RtValue::List(_))
        | (RtValue::Object(_), RtValue::Object(_))
        | (RtValue::Error(_), RtValue::Error(_))
        | (RtValue::Closure(_), RtValue::Closure(_)) => left == right,
        _ => false,
    }
}

/// Allocates one zero-payload compiler-owned `std::Ordering` enum value.
fn ordering_value(variant: &str) -> ValueRef {
    runtime::allocate(RtValue::Object(Box::new(RuntimeObject::enumeration(
        Some(STANDARD_ORDERING_TYPE_ID),
        RuntimeEnum {
            type_identity: Box::from(STANDARD_ORDERING_TYPE_IDENTITY),
            variant: Box::from(variant),
            fields: Vec::new(),
        },
    ))))
}

/// Tests two runtime values for inequality.
pub(crate) fn not_equal(left: ValueRef, right: ValueRef) -> ValueRef {
    let equal = equal(left, right);
    runtime::allocate(RtValue::Bool(!numeric::boolean(equal)))
}

/// Appends a value through the receiver's runtime collection dispatch.
pub(crate) fn append(receiver: ValueRef, item: ValueRef) -> ValueRef {
    list::operations::append(receiver, item)
}

/// Reads one value through the receiver's runtime indexing dispatch.
pub(crate) fn index_get(receiver: ValueRef, index: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        RtValue::List(_) => list::operations::get(receiver, index),
        RtValue::Object(_) => object::operations::get(receiver, index),
        RtValue::Bytes(_) => bytes_get(receiver, index),
        _ => runtime::recoverable_error(
            "TypeError",
            "index access requires a Bytes, List, or Object receiver",
            receiver,
        ),
    }
}

/// Creates immutable Bytes from one List of integer octets.
pub(crate) fn bytes_from_list(values: ValueRef) -> ValueRef {
    let bytes = match runtime::value(values) {
        RtValue::List(values) => {
            let mut bytes = Vec::with_capacity(values.elements.len());
            for value in &values.elements {
                match runtime::value(*value) {
                    RtValue::Int(value) if (0..=255).contains(value) => bytes.push(*value as u8),
                    RtValue::Int(_) => {
                        return runtime::recoverable_error(
                            "ValueError",
                            "Bytes::from_list values must be between 0 and 255",
                            *value,
                        );
                    }
                    _ => {
                        return runtime::recoverable_error(
                            "TypeError",
                            "Bytes::from_list requires a List of Int values",
                            *value,
                        );
                    }
                }
            }
            bytes
        }
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "Bytes::from_list requires a List receiver",
                values,
            );
        }
    };
    bytes_value(bytes)
}

/// Encodes one UTF-8 String as immutable Bytes.
pub(crate) fn bytes_from_utf8(value: ValueRef) -> ValueRef {
    match runtime::value(value) {
        RtValue::String(value) => bytes_value(value.as_str().as_bytes().to_vec()),
        _ => runtime::recoverable_error(
            "TypeError",
            "Bytes::from_utf8 requires a String value",
            value,
        ),
    }
}

/// Parses one decimal String as an exact signed 64-bit Int.
pub(crate) fn integer_parse(value: ValueRef) -> ValueRef {
    let receiver = value;
    let value = match runtime::value(value) {
        RtValue::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "Int::parse requires a String value",
                value,
            );
        }
    };
    match i64::from_str(value) {
        Ok(value) => runtime::allocate(RtValue::Int(value)),
        Err(_) => runtime::recoverable_error(
            "ParseError",
            "Int::parse requires a base-10 signed 64-bit integer",
            receiver,
        ),
    }
}

/// Parses one decimal String as an IEEE-754 binary64 Float.
pub(crate) fn float_parse(value: ValueRef) -> ValueRef {
    let receiver = value;
    let value = match runtime::value(value) {
        RtValue::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "Float::parse requires a String value",
                receiver,
            );
        }
    };
    match f64::from_str(value) {
        Ok(value) => runtime::allocate(RtValue::Float(value)),
        Err(_) => runtime::recoverable_error(
            "ParseError",
            "Float::parse requires a decimal floating-point value",
            receiver,
        ),
    }
}

/// Returns the mathematical constant pi as a Float.
pub(crate) fn math_pi() -> ValueRef {
    runtime::allocate(RtValue::Float(core::f64::consts::PI))
}

/// Returns the mathematical constant tau as a Float.
pub(crate) fn math_tau() -> ValueRef {
    runtime::allocate(RtValue::Float(core::f64::consts::TAU))
}

/// Returns Euler's number as a Float.
pub(crate) fn math_e() -> ValueRef {
    runtime::allocate(RtValue::Float(core::f64::consts::E))
}

/// Evaluates one strict-Float unary mathematical function.
pub(crate) fn math_unary(value: ValueRef, operation: fn(f64) -> f64, name: &str) -> ValueRef {
    float_unary(
        value,
        operation,
        &format!("Math::{name} requires a Float argument"),
    )
}

/// Evaluates one strict-Float binary mathematical function.
pub(crate) fn math_binary(
    left: ValueRef,
    right: ValueRef,
    operation: fn(f64, f64) -> f64,
    name: &str,
) -> ValueRef {
    float_pair(
        left,
        right,
        operation,
        &format!("Math::{name} requires Float arguments"),
    )
}

/// Reads one immutable Bytes octet at a zero-based integer index.
fn bytes_get(receiver: ValueRef, index: ValueRef) -> ValueRef {
    let index = match bytes_index(index, false) {
        Ok(index) => index,
        Err(error) => return error,
    };
    match runtime::value(receiver) {
        RtValue::Bytes(bytes) => match bytes.as_slice().get(index) {
            Some(byte) => runtime::allocate(RtValue::Int(i64::from(*byte))),
            None => runtime::recoverable_error(
                "IndexError",
                "Bytes index is outside the Bytes bounds",
                receiver,
            ),
        },
        _ => runtime::recoverable_error(
            "TypeError",
            "index access requires a Bytes receiver",
            receiver,
        ),
    }
}

/// Returns a new immutable Bytes subsequence for one half-open byte range.
fn bytes_slice(receiver: ValueRef, start: ValueRef, end: ValueRef) -> ValueRef {
    let start = match bytes_index(start, true) {
        Ok(index) => index,
        Err(error) => return error,
    };
    let end = match bytes_index(end, true) {
        Ok(index) => index,
        Err(error) => return error,
    };
    match runtime::value(receiver) {
        RtValue::Bytes(bytes) if start <= end && end <= bytes.as_slice().len() => {
            bytes_value(bytes.as_slice()[start..end].to_vec())
        }
        RtValue::Bytes(_) => runtime::recoverable_error(
            "IndexError",
            "Bytes slice range is outside the Bytes bounds",
            receiver,
        ),
        _ => runtime::recoverable_error("TypeError", "slice requires a Bytes receiver", receiver),
    }
}

/// Concatenates two immutable Bytes values into one new Bytes value.
fn bytes_concat(receiver: ValueRef, other: ValueRef) -> ValueRef {
    match (runtime::value(receiver), runtime::value(other)) {
        (RtValue::Bytes(left), RtValue::Bytes(right)) => {
            let mut bytes = Vec::with_capacity(left.as_slice().len() + right.as_slice().len());
            bytes.extend_from_slice(left.as_slice());
            bytes.extend_from_slice(right.as_slice());
            bytes_value(bytes)
        }
        (RtValue::Bytes(_), _) => {
            runtime::recoverable_error("TypeError", "concat requires a Bytes argument", other)
        }
        _ => runtime::recoverable_error("TypeError", "concat requires a Bytes receiver", receiver),
    }
}

/// Returns a new List containing the receiver's octets as Int values.
fn bytes_to_list(receiver: ValueRef) -> ValueRef {
    let bytes = match runtime::value(receiver) {
        RtValue::Bytes(bytes) => bytes.as_slice().to_vec(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "to_list requires a Bytes receiver",
                receiver,
            );
        }
    };
    let checkpoint = gc::temporary_root_checkpoint();
    let list = runtime::allocate(RtValue::List(Box::new(RuntimeList::new())));
    gc::push_temporary_root(list);
    for byte in bytes {
        let value = runtime::allocate(RtValue::Int(i64::from(byte)));
        let RtValue::List(list_value) = runtime::value_mut(list) else {
            runtime::trap();
        };
        list_value.elements.push(value);
    }
    gc::restore_temporary_roots(checkpoint);
    list
}

/// Decodes one Bytes value as UTF-8 or returns an EncodingError.
fn bytes_decode_utf8(receiver: ValueRef) -> ValueRef {
    let text = match runtime::value(receiver) {
        RtValue::Bytes(bytes) => match core::str::from_utf8(bytes.as_slice()) {
            Ok(value) => String::from(value),
            Err(_) => {
                return runtime::recoverable_error(
                    "EncodingError",
                    "Bytes do not contain valid UTF-8",
                    receiver,
                );
            }
        },
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "decode_utf8 requires a Bytes receiver",
                receiver,
            );
        }
    };
    string_value_result(text)
}

/// Tests whether one Bytes value starts with another Bytes value.
fn bytes_starts_with(receiver: ValueRef, prefix: ValueRef) -> ValueRef {
    bytes_relation(
        receiver,
        prefix,
        |value, needle| value.starts_with(needle),
        "starts_with",
    )
}

/// Tests whether one Bytes value ends with another Bytes value.
fn bytes_ends_with(receiver: ValueRef, suffix: ValueRef) -> ValueRef {
    bytes_relation(
        receiver,
        suffix,
        |value, needle| value.ends_with(needle),
        "ends_with",
    )
}

/// Tests whether one Bytes value contains another Bytes value.
fn bytes_contains(receiver: ValueRef, needle: ValueRef) -> ValueRef {
    let value = match runtime::value(receiver) {
        RtValue::Bytes(value) => value.as_slice(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "contains requires a Bytes receiver",
                receiver,
            );
        }
    };
    let needle = match runtime::value(needle) {
        RtValue::Bytes(value) => value.as_slice(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "contains requires a Bytes argument",
                needle,
            );
        }
    };
    runtime::allocate(RtValue::Bool(
        needle.is_empty() || value.windows(needle.len()).any(|window| window == needle),
    ))
}

/// Applies one binary Bytes predicate after validating both operands.
fn bytes_relation(
    receiver: ValueRef,
    other: ValueRef,
    predicate: fn(&[u8], &[u8]) -> bool,
    name: &str,
) -> ValueRef {
    let value = match runtime::value(receiver) {
        RtValue::Bytes(value) => value.as_slice(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                &format!("{name} requires a Bytes receiver"),
                receiver,
            );
        }
    };
    let other = match runtime::value(other) {
        RtValue::Bytes(value) => value.as_slice(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                &format!("{name} requires a Bytes argument"),
                other,
            );
        }
    };
    runtime::allocate(RtValue::Bool(predicate(value, other)))
}

/// Repeats one Bytes value a non-negative number of times.
fn bytes_repeat(receiver: ValueRef, count: ValueRef) -> ValueRef {
    let count = match non_negative_count(count, "repeat") {
        Ok(count) => count,
        Err(error) => return error,
    };
    let bytes = match runtime::value(receiver) {
        RtValue::Bytes(bytes) => bytes.as_slice(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "repeat requires a Bytes receiver",
                receiver,
            );
        }
    };
    let Some(length) = bytes.len().checked_mul(count) else {
        return runtime::recoverable_error("ValueError", "repeated Bytes are too large", receiver);
    };
    let mut result = Vec::with_capacity(length);
    for _ in 0..count {
        result.extend_from_slice(bytes);
    }
    bytes_value(result)
}

/// Encodes one Bytes value as lowercase hexadecimal text.
fn bytes_to_hex(receiver: ValueRef) -> ValueRef {
    let bytes = match runtime::value(receiver) {
        RtValue::Bytes(bytes) => bytes.as_slice(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "to_hex requires a Bytes receiver",
                receiver,
            );
        }
    };
    let Some(length) = bytes.len().checked_mul(2) else {
        return runtime::recoverable_error("ValueError", "hex output is too large", receiver);
    };
    let mut output = String::with_capacity(length);
    for byte in bytes {
        output.push(hex_digit(byte >> 4));
        output.push(hex_digit(byte & 0x0f));
    }
    string_value_result(output)
}

/// Decodes lowercase or uppercase hexadecimal text into immutable Bytes.
pub(crate) fn bytes_from_hex(value: ValueRef) -> ValueRef {
    let receiver = value;
    let value = match runtime::value(value) {
        RtValue::String(value) => value.as_str().as_bytes(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "Bytes::from_hex requires a String value",
                receiver,
            );
        }
    };
    if value.len() % 2 != 0 {
        return runtime::recoverable_error(
            "ParseError",
            "hex input must contain an even number of digits",
            receiver,
        );
    }
    let mut output = Vec::with_capacity(value.len() / 2);
    for digits in value.chunks_exact(2) {
        let Some(high) = hex_value(digits[0]) else {
            return runtime::recoverable_error(
                "ParseError",
                "hex input contains an invalid digit",
                receiver,
            );
        };
        let Some(low) = hex_value(digits[1]) else {
            return runtime::recoverable_error(
                "ParseError",
                "hex input contains an invalid digit",
                receiver,
            );
        };
        output.push((high << 4) | low);
    }
    bytes_value(output)
}

/// Returns one lowercase hexadecimal digit.
fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + (value - 10)),
        _ => runtime::trap(),
    }
}

/// Decodes one hexadecimal ASCII digit.
fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

/// Encodes one Bytes value as padded RFC 4648 base64 text.
fn bytes_to_base64(receiver: ValueRef) -> ValueRef {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let bytes = match runtime::value(receiver) {
        RtValue::Bytes(bytes) => bytes.as_slice(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "to_base64 requires a Bytes receiver",
                receiver,
            );
        }
    };
    let Some(length) = bytes.len().checked_add(2).map(|length| length / 3 * 4) else {
        return runtime::recoverable_error("ValueError", "base64 output is too large", receiver);
    };
    let mut output = String::with_capacity(length);
    for group in bytes.chunks(3) {
        let first = group[0];
        let second = group.get(1).copied().unwrap_or(0);
        let third = group.get(2).copied().unwrap_or(0);
        output.push(char::from(ALPHABET[(first >> 2) as usize]));
        output.push(char::from(
            ALPHABET[(((first & 0x03) << 4) | (second >> 4)) as usize],
        ));
        output.push(if group.len() > 1 {
            char::from(ALPHABET[(((second & 0x0f) << 2) | (third >> 6)) as usize])
        } else {
            '='
        });
        output.push(if group.len() > 2 {
            char::from(ALPHABET[(third & 0x3f) as usize])
        } else {
            '='
        });
    }
    string_value_result(output)
}

/// Decodes padded RFC 4648 base64 text into immutable Bytes.
pub(crate) fn bytes_from_base64(value: ValueRef) -> ValueRef {
    let receiver = value;
    let value = match runtime::value(value) {
        RtValue::String(value) => value.as_str().as_bytes(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "Bytes::from_base64 requires a String value",
                receiver,
            );
        }
    };
    if value.len() % 4 != 0 {
        return runtime::recoverable_error(
            "ParseError",
            "base64 input must have a length divisible by four",
            receiver,
        );
    }
    let mut output = Vec::with_capacity(value.len() / 4 * 3);
    for (index, group) in value.chunks_exact(4).enumerate() {
        let final_group = index + 1 == value.len() / 4;
        let Some(first) = base64_value(group[0]) else {
            return base64_parse_error(receiver);
        };
        let Some(second) = base64_value(group[1]) else {
            return base64_parse_error(receiver);
        };
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
            return base64_parse_error(receiver);
        }
        let third = third.unwrap_or_default();
        let fourth = fourth.unwrap_or_default();
        if (group[2] == b'=' && second & 0x0f != 0)
            || (group[3] == b'=' && group[2] != b'=' && third & 0x03 != 0)
        {
            return base64_parse_error(receiver);
        }
        output.push((first << 2) | (second >> 4));
        if group[2] != b'=' {
            output.push((second << 4) | (third >> 2));
        }
        if group[3] != b'=' {
            output.push((third << 6) | fourth);
        }
    }
    bytes_value(output)
}

/// Returns one standard parse Error for malformed base64 text.
fn base64_parse_error(receiver: ValueRef) -> ValueRef {
    runtime::recoverable_error(
        "ParseError",
        "base64 input is not valid padded RFC 4648 text",
        receiver,
    )
}

/// Decodes one unpadded base64 ASCII digit.
fn base64_value(value: u8) -> Option<u8> {
    match value {
        b'A'..=b'Z' => Some(value - b'A'),
        b'a'..=b'z' => Some(value - b'a' + 26),
        b'0'..=b'9' => Some(value - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Returns one String range addressed by Unicode scalar offsets.
fn string_slice(receiver: ValueRef, start: ValueRef, end: ValueRef) -> ValueRef {
    let start = match scalar_index(start, "slice") {
        Ok(index) => index,
        Err(error) => return error,
    };
    let end = match scalar_index(end, "slice") {
        Ok(index) => index,
        Err(error) => return error,
    };
    let value = match runtime::value(receiver) {
        RtValue::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "slice requires a String receiver",
                receiver,
            );
        }
    };
    let scalar_count = value.chars().count();
    if start > end || end > scalar_count {
        return runtime::recoverable_error(
            "IndexError",
            "String slice range is outside the String bounds",
            receiver,
        );
    }
    let start = scalar_byte_offset(value, start);
    let end = scalar_byte_offset(value, end);
    string_value_result(String::from(&value[start..end]))
}

/// Converts one Unicode scalar position into its UTF-8 byte offset.
fn scalar_byte_offset(value: &str, index: usize) -> usize {
    value
        .char_indices()
        .nth(index)
        .map_or(value.len(), |(offset, _)| offset)
}

/// Reads one non-negative Int used as a String scalar offset.
fn scalar_index(value: ValueRef, name: &str) -> Result<usize, ValueRef> {
    match runtime::value(value) {
        RtValue::Int(number) if *number >= 0 => usize::try_from(*number).map_err(|_| {
            runtime::recoverable_error(
                "IndexError",
                &format!("{name} requires a supported non-negative Int index"),
                value,
            )
        }),
        _ => Err(runtime::recoverable_error(
            "IndexError",
            &format!("{name} requires non-negative Int indexes"),
            value,
        )),
    }
}

/// Tests whether one String contains another String.
fn string_contains(receiver: ValueRef, needle: ValueRef) -> ValueRef {
    string_relation(
        receiver,
        needle,
        |value, needle| value.contains(needle),
        "contains",
    )
}

/// Tests whether one String starts with another String.
fn string_starts_with(receiver: ValueRef, prefix: ValueRef) -> ValueRef {
    string_relation(
        receiver,
        prefix,
        |value, prefix| value.starts_with(prefix),
        "starts_with",
    )
}

/// Tests whether one String ends with another String.
fn string_ends_with(receiver: ValueRef, suffix: ValueRef) -> ValueRef {
    string_relation(
        receiver,
        suffix,
        |value, suffix| value.ends_with(suffix),
        "ends_with",
    )
}

/// Applies one binary String predicate after validating both operands.
fn string_relation(
    receiver: ValueRef,
    other: ValueRef,
    predicate: fn(&str, &str) -> bool,
    name: &str,
) -> ValueRef {
    let value = match runtime::value(receiver) {
        RtValue::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                &format!("{name} requires a String receiver"),
                receiver,
            );
        }
    };
    let other = match runtime::value(other) {
        RtValue::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                &format!("{name} requires a String argument"),
                other,
            );
        }
    };
    runtime::allocate(RtValue::Bool(predicate(value, other)))
}

/// Trims Unicode whitespace from both ends of one String.
fn string_trim(receiver: ValueRef) -> ValueRef {
    string_transform(receiver, str::trim, "trim")
}

/// Trims Unicode whitespace from the start of one String.
fn string_trim_start(receiver: ValueRef) -> ValueRef {
    string_transform(receiver, str::trim_start, "trim_start")
}

/// Trims Unicode whitespace from the end of one String.
fn string_trim_end(receiver: ValueRef) -> ValueRef {
    string_transform(receiver, str::trim_end, "trim_end")
}

/// Applies one String-to-String operation after validating the receiver.
fn string_transform(receiver: ValueRef, transform: fn(&str) -> &str, name: &str) -> ValueRef {
    let value = match runtime::value(receiver) {
        RtValue::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                &format!("{name} requires a String receiver"),
                receiver,
            );
        }
    };
    string_value_result(String::from(transform(value)))
}

/// Replaces every non-overlapping String match with another String.
fn string_replace(receiver: ValueRef, from: ValueRef, to: ValueRef) -> ValueRef {
    let value = match runtime::value(receiver) {
        RtValue::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "replace requires a String receiver",
                receiver,
            );
        }
    };
    let from = match runtime::value(from) {
        RtValue::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "replace requires a String search value",
                from,
            );
        }
    };
    let to = match runtime::value(to) {
        RtValue::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "replace requires a String replacement",
                to,
            );
        }
    };
    string_value_result(value.replace(from, to))
}

/// Splits one String around an exact String delimiter into a new List.
fn string_split(receiver: ValueRef, delimiter: ValueRef) -> ValueRef {
    let value = match runtime::value(receiver) {
        RtValue::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "split requires a String receiver",
                receiver,
            );
        }
    };
    let delimiter = match runtime::value(delimiter) {
        RtValue::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "split requires a String delimiter",
                delimiter,
            );
        }
    };
    let parts = value.split(delimiter).map(String::from).collect::<Vec<_>>();
    string_list(parts)
}

/// Returns a new List of runtime Strings while preserving temporary roots during allocation.
fn string_list(values: Vec<String>) -> ValueRef {
    let checkpoint = gc::temporary_root_checkpoint();
    let list = runtime::allocate(RtValue::List(Box::new(RuntimeList::new())));
    gc::push_temporary_root(list);
    for value in values {
        let value = string_value_result(value);
        gc::push_temporary_root(value);
        let RtValue::List(list_value) = runtime::value_mut(list) else {
            runtime::trap();
        };
        list_value.elements.push(value);
    }
    gc::restore_temporary_roots(checkpoint);
    list
}

/// Converts one String to its Unicode lowercase mapping.
fn string_to_lowercase(receiver: ValueRef) -> ValueRef {
    let value = match runtime::value(receiver) {
        RtValue::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "to_lowercase requires a String receiver",
                receiver,
            );
        }
    };
    string_value_result(value.to_lowercase())
}

/// Converts one String to its Unicode uppercase mapping.
fn string_to_uppercase(receiver: ValueRef) -> ValueRef {
    let value = match runtime::value(receiver) {
        RtValue::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "to_uppercase requires a String receiver",
                receiver,
            );
        }
    };
    string_value_result(value.to_uppercase())
}

/// Repeats one String a non-negative number of times.
fn string_repeat(receiver: ValueRef, count: ValueRef) -> ValueRef {
    let count = match non_negative_count(count, "repeat") {
        Ok(count) => count,
        Err(error) => return error,
    };
    let value = match runtime::value(receiver) {
        RtValue::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "repeat requires a String receiver",
                receiver,
            );
        }
    };
    let Some(length) = value.len().checked_mul(count) else {
        return runtime::recoverable_error("ValueError", "repeated String is too large", receiver);
    };
    let mut result = String::with_capacity(length);
    for _ in 0..count {
        result.push_str(value);
    }
    string_value_result(result)
}

/// Reads one non-negative Int repeat count.
fn non_negative_count(value: ValueRef, name: &str) -> Result<usize, ValueRef> {
    match runtime::value(value) {
        RtValue::Int(number) if *number >= 0 => usize::try_from(*number).map_err(|_| {
            runtime::recoverable_error(
                "ValueError",
                &format!("{name} count is outside the supported range"),
                value,
            )
        }),
        _ => Err(runtime::recoverable_error(
            "TypeError",
            &format!("{name} requires a non-negative Int count"),
            value,
        )),
    }
}

/// Validates one Bytes index and converts it to the native index type.
fn bytes_index(reference: ValueRef, allow_end: bool) -> Result<usize, ValueRef> {
    match runtime::value(reference) {
        RtValue::Int(index) if *index >= 0 => usize::try_from(*index).map_err(|_| {
            runtime::recoverable_error(
                "IndexError",
                "Bytes index is outside the supported range",
                reference,
            )
        }),
        _ => Err(runtime::recoverable_error(
            "IndexError",
            if allow_end {
                "Bytes slice indexes require non-negative Int values"
            } else {
                "Bytes index requires a non-negative Int value"
            },
            reference,
        )),
    }
}

/// Allocates one immutable Bytes result from owned raw octets.
fn bytes_value(value: Vec<u8>) -> ValueRef {
    runtime::allocate(RtValue::Bytes(Box::new(RuntimeBytes::from_vec(value))))
}

/// Replaces one value through the receiver's runtime indexing dispatch.
pub(crate) fn index_set(receiver: ValueRef, index: ValueRef, replacement: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        RtValue::List(_) => list::operations::set(receiver, index, replacement),
        RtValue::Object(_) => object::operations::set(receiver, index, replacement),
        _ => runtime::recoverable_error(
            "TypeError",
            "index assignment requires a List or Object receiver",
            receiver,
        ),
    }
}

/// Creates the shallow List or scalar-String snapshot consumed by a for loop.
pub(crate) fn iter_snapshot(iterable: ValueRef) -> ValueRef {
    match runtime::value(iterable) {
        RtValue::List(list) => {
            let mut elements = list.elements.clone();
            elements.reverse();
            runtime::allocate(RtValue::List(Box::new(RuntimeList { elements })))
        }
        RtValue::String(string) => {
            let scalars = string
                .as_str()
                .chars()
                .map(|scalar| scalar.to_string())
                .collect::<Vec<_>>();
            let checkpoint = gc::temporary_root_checkpoint();
            let mut elements = Vec::with_capacity(scalars.len());
            for scalar in scalars {
                let value = runtime::allocate(RtValue::String(Box::new(
                    crate::value::RuntimeString::from_string(scalar),
                )));
                gc::push_temporary_root(value);
                elements.push(value);
            }
            elements.reverse();
            let snapshot = runtime::allocate(RtValue::List(Box::new(RuntimeList { elements })));
            gc::restore_temporary_roots(checkpoint);
            snapshot
        }
        RtValue::Bytes(bytes) => {
            let checkpoint = gc::temporary_root_checkpoint();
            let mut elements = Vec::with_capacity(bytes.as_slice().len());
            for byte in bytes.as_slice() {
                let value = runtime::allocate(RtValue::Int(i64::from(*byte)));
                gc::push_temporary_root(value);
                elements.push(value);
            }
            elements.reverse();
            let snapshot = runtime::allocate(RtValue::List(Box::new(RuntimeList { elements })));
            gc::restore_temporary_roots(checkpoint);
            snapshot
        }
        RtValue::Object(_) => iterable,
        _ => runtime::recoverable_error(
            "NotIterable",
            "for requires an iterable or Iterator receiver",
            iterable,
        ),
    }
}

/// Advances one built-in List snapshot and returns a prelude IteratorStep value.
fn iterator_next(receiver: ValueRef) -> ValueRef {
    let item = match runtime::value_mut(receiver) {
        RtValue::List(list) => {
            if list.elements.is_empty() {
                None
            } else {
                list.elements.pop()
            }
        }
        _ => {
            return runtime::recoverable_error(
                "NotIterable",
                "next requires an Iterator receiver",
                receiver,
            );
        }
    };
    let checkpoint = gc::temporary_root_checkpoint();
    let (variant, fields) = match item {
        Some(item) => {
            gc::push_temporary_root(item);
            ("Item", vec![item])
        }
        None => ("Done", Vec::new()),
    };
    let step = runtime::allocate(RtValue::Object(Box::new(RuntimeObject::enumeration(
        None,
        RuntimeEnum {
            type_identity: STANDARD_ITERATOR_STEP_TYPE_IDENTITY.into(),
            variant: variant.into(),
            fields,
        },
    ))));
    gc::restore_temporary_roots(checkpoint);
    step
}

/// Returns the scalar or entry count for runtime values with a visible length.
pub(crate) fn length(value: ValueRef) -> ValueRef {
    let length = match runtime::value(value) {
        RtValue::String(value) => value.as_str().chars().count(),
        RtValue::Bytes(value) => value.as_slice().len(),
        RtValue::List(value) => value.elements.len(),
        RtValue::Object(value) => value.entries.len(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "length requires a Bytes, String, List, or Object receiver",
                value,
            );
        }
    };
    list::operations::length_value(length)
}

/// Returns whether a String, List, or Object contains no visible entries.
pub(crate) fn is_empty(value: ValueRef) -> ValueRef {
    let empty = match runtime::value(value) {
        RtValue::String(value) => value.as_str().is_empty(),
        RtValue::Bytes(value) => value.as_slice().is_empty(),
        RtValue::List(value) => value.elements.len() == 0,
        RtValue::Object(value) => value.entries.is_empty(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "is_empty requires a Bytes, String, List, or Object receiver",
                value,
            );
        }
    };
    runtime::allocate(RtValue::Bool(empty))
}

/// Returns the absolute value of one Int when it fits in the ExS signed 64-bit range.
pub(crate) fn integer_abs(receiver: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        RtValue::Int(value) => match value.checked_abs() {
            Some(result) => runtime::allocate(RtValue::Int(result)),
            None => runtime::recoverable_error(
                "IntOverflowError",
                "absolute value is outside the ExS signed 64-bit range",
                receiver,
            ),
        },
        _ => runtime::recoverable_error("TypeError", "abs requires an Int receiver", receiver),
    }
}

/// Returns the absolute value of one Float.
pub(crate) fn float_abs(value: ValueRef) -> ValueRef {
    float_unary(value, f64::abs, "abs requires a Float receiver")
}

/// Returns the greatest integral Float not greater than the receiver.
pub(crate) fn float_floor(value: ValueRef) -> ValueRef {
    float_unary(value, libm::floor, "floor requires a Float receiver")
}

/// Returns the least integral Float not less than the receiver.
pub(crate) fn float_ceil(value: ValueRef) -> ValueRef {
    float_unary(value, libm::ceil, "ceil requires a Float receiver")
}

/// Rounds one Float to the nearest integral Float, with halves away from zero.
pub(crate) fn float_round(value: ValueRef) -> ValueRef {
    float_unary(value, libm::round, "round requires a Float receiver")
}

/// Removes the fractional component of one Float.
fn float_trunc(value: ValueRef) -> ValueRef {
    float_unary(value, libm::trunc, "trunc requires a Float receiver")
}

/// Returns the signed fractional component of one Float.
fn float_fract(value: ValueRef) -> ValueRef {
    float_unary(
        value,
        |value| value - libm::trunc(value),
        "fract requires a Float receiver",
    )
}

/// Returns the sign of one Int as -1, 0, or 1.
fn integer_signum(receiver: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        RtValue::Int(value) => runtime::allocate(RtValue::Int(value.signum())),
        _ => runtime::recoverable_error("TypeError", "signum requires an Int receiver", receiver),
    }
}

/// Returns whether one Int is evenly divisible by two.
fn integer_is_even(receiver: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        RtValue::Int(value) => runtime::allocate(RtValue::Bool(value % 2 == 0)),
        _ => runtime::recoverable_error("TypeError", "is_even requires an Int receiver", receiver),
    }
}

/// Returns whether one Int is not evenly divisible by two.
fn integer_is_odd(receiver: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        RtValue::Int(value) => runtime::allocate(RtValue::Bool(value % 2 != 0)),
        _ => runtime::recoverable_error("TypeError", "is_odd requires an Int receiver", receiver),
    }
}

/// Raises one Int to a non-negative integer exponent.
fn integer_pow(receiver: ValueRef, exponent: ValueRef) -> ValueRef {
    let base = match runtime::value(receiver) {
        RtValue::Int(value) => *value,
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "pow requires an Int receiver",
                receiver,
            );
        }
    };
    let exponent = match runtime::value(exponent) {
        RtValue::Int(value) if *value >= 0 => *value as u64,
        RtValue::Int(_) => {
            return runtime::recoverable_error(
                "ValueError",
                "Int pow requires a non-negative exponent",
                exponent,
            );
        }
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "Int pow requires an Int exponent",
                exponent,
            );
        }
    };
    let mut result = 1_i64;
    let mut factor = base;
    let mut exponent = exponent;
    while exponent > 0 {
        if exponent & 1 == 1 {
            let Some(value) = result.checked_mul(factor) else {
                return runtime::recoverable_error(
                    "IntOverflowError",
                    "Int pow overflowed the ExS signed 64-bit range",
                    receiver,
                );
            };
            result = value;
        }
        exponent >>= 1;
        if exponent > 0 {
            let Some(value) = factor.checked_mul(factor) else {
                return runtime::recoverable_error(
                    "IntOverflowError",
                    "Int pow overflowed the ExS signed 64-bit range",
                    receiver,
                );
            };
            factor = value;
        }
    }
    runtime::allocate(RtValue::Int(result))
}

/// Returns the greatest common divisor of two Int values when it fits in Int.
fn integer_gcd(receiver: ValueRef, other: ValueRef) -> ValueRef {
    let left = match runtime::value(receiver) {
        RtValue::Int(value) => value.unsigned_abs(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "gcd requires an Int receiver",
                receiver,
            );
        }
    };
    let right = match runtime::value(other) {
        RtValue::Int(value) => value.unsigned_abs(),
        _ => return runtime::recoverable_error("TypeError", "gcd requires an Int argument", other),
    };
    integer_from_unsigned(gcd(left, right), receiver, "gcd")
}

/// Returns the least common multiple of two Int values when it fits in Int.
fn integer_lcm(receiver: ValueRef, other: ValueRef) -> ValueRef {
    let left = match runtime::value(receiver) {
        RtValue::Int(value) => value.unsigned_abs(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "lcm requires an Int receiver",
                receiver,
            );
        }
    };
    let right = match runtime::value(other) {
        RtValue::Int(value) => value.unsigned_abs(),
        _ => return runtime::recoverable_error("TypeError", "lcm requires an Int argument", other),
    };
    if left == 0 || right == 0 {
        return runtime::allocate(RtValue::Int(0));
    }
    let Some(value) = (left / gcd(left, right)).checked_mul(right) else {
        return runtime::recoverable_error(
            "IntOverflowError",
            "lcm overflowed the ExS signed 64-bit range",
            receiver,
        );
    };
    integer_from_unsigned(value, receiver, "lcm")
}

/// Finds the greatest common divisor of two unsigned magnitudes.
fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

/// Allocates an unsigned magnitude only when it is representable as an ExS Int.
fn integer_from_unsigned(value: u64, receiver: ValueRef, name: &str) -> ValueRef {
    match i64::try_from(value) {
        Ok(value) => runtime::allocate(RtValue::Int(value)),
        Err(_) => runtime::recoverable_error(
            "IntOverflowError",
            &format!("{name} is outside the ExS signed 64-bit range"),
            receiver,
        ),
    }
}

/// Returns the smaller of two Int values.
fn integer_min(receiver: ValueRef, other: ValueRef) -> ValueRef {
    integer_pair(receiver, other, i64::min, "min")
}

/// Returns the larger of two Int values.
fn integer_max(receiver: ValueRef, other: ValueRef) -> ValueRef {
    integer_pair(receiver, other, i64::max, "max")
}

/// Restricts one Int to an inclusive Int range.
fn integer_clamp(receiver: ValueRef, minimum: ValueRef, maximum: ValueRef) -> ValueRef {
    let value = integer_value(receiver, "clamp requires an Int receiver");
    let minimum = integer_value(minimum, "clamp requires an Int minimum");
    let maximum = integer_value(maximum, "clamp requires an Int maximum");
    match (value, minimum, maximum) {
        (Ok(value), Ok(minimum), Ok(maximum)) if minimum <= maximum => {
            runtime::allocate(RtValue::Int(value.clamp(minimum, maximum)))
        }
        (Ok(_), Ok(_), Ok(_)) => runtime::recoverable_error(
            "ValueError",
            "clamp minimum must not exceed maximum",
            receiver,
        ),
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => error,
    }
}

/// Reads one exact runtime Int for a numeric method.
fn integer_value(value: ValueRef, message: &str) -> Result<i64, ValueRef> {
    match runtime::value(value) {
        RtValue::Int(value) => Ok(*value),
        _ => Err(runtime::recoverable_error("TypeError", message, value)),
    }
}

/// Applies one binary Int operation after validating both operands.
fn integer_pair(
    receiver: ValueRef,
    other: ValueRef,
    operation: fn(i64, i64) -> i64,
    name: &str,
) -> ValueRef {
    match (
        integer_value(receiver, &format!("{name} requires an Int receiver")),
        integer_value(other, &format!("{name} requires an Int argument")),
    ) {
        (Ok(left), Ok(right)) => runtime::allocate(RtValue::Int(operation(left, right))),
        (Err(error), _) | (_, Err(error)) => error,
    }
}

/// Converts one Int to an IEEE-754 binary64 Float.
fn integer_to_float(receiver: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        RtValue::Int(value) => runtime::allocate(RtValue::Float(*value as f64)),
        _ => runtime::recoverable_error("TypeError", "to_float requires an Int receiver", receiver),
    }
}

/// Converts one finite Float to a truncated Int when the result fits in Int.
fn float_to_int(receiver: ValueRef) -> ValueRef {
    let value = match runtime::value(receiver) {
        RtValue::Float(value) => *value,
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "to_int requires a Float receiver",
                receiver,
            );
        }
    };
    let value = libm::trunc(value);
    if !value.is_finite() {
        return runtime::recoverable_error(
            "ValueError",
            "to_int requires a finite Float",
            receiver,
        );
    }
    if value < i64::MIN as f64 || value >= 9_223_372_036_854_775_808.0 {
        return runtime::recoverable_error(
            "IntOverflowError",
            "Float is outside the ExS Int range",
            receiver,
        );
    }
    runtime::allocate(RtValue::Int(value as i64))
}

/// Returns the smaller of two Float values using IEEE-754 minimum semantics.
fn float_min(receiver: ValueRef, other: ValueRef) -> ValueRef {
    float_pair(receiver, other, libm::fmin, "min")
}

/// Returns the larger of two Float values using IEEE-754 maximum semantics.
fn float_max(receiver: ValueRef, other: ValueRef) -> ValueRef {
    float_pair(receiver, other, libm::fmax, "max")
}

/// Restricts one Float to an inclusive Float range.
fn float_clamp(receiver: ValueRef, minimum: ValueRef, maximum: ValueRef) -> ValueRef {
    let value = float_value(receiver, "clamp requires a Float receiver");
    let minimum = float_value(minimum, "clamp requires a Float minimum");
    let maximum = float_value(maximum, "clamp requires a Float maximum");
    match (value, minimum, maximum) {
        (Ok(value), Ok(minimum), Ok(maximum)) if minimum <= maximum => runtime::allocate(
            RtValue::Float(libm::fmin(libm::fmax(value, minimum), maximum)),
        ),
        (Ok(_), Ok(_), Ok(_)) => runtime::recoverable_error(
            "ValueError",
            "clamp minimum must not exceed maximum",
            receiver,
        ),
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => error,
    }
}

/// Returns whether one Float is NaN.
fn float_is_nan(receiver: ValueRef) -> ValueRef {
    float_predicate(receiver, f64::is_nan, "is_nan")
}

/// Returns whether one Float is finite.
fn float_is_finite(receiver: ValueRef) -> ValueRef {
    float_predicate(receiver, f64::is_finite, "is_finite")
}

/// Returns whether one Float is infinite.
fn float_is_infinite(receiver: ValueRef) -> ValueRef {
    float_predicate(receiver, f64::is_infinite, "is_infinite")
}

/// Computes the square root of one Float with IEEE-754 semantics.
fn float_sqrt(receiver: ValueRef) -> ValueRef {
    float_unary(receiver, libm::sqrt, "sqrt requires a Float receiver")
}

/// Raises one Float to another Float power with IEEE-754 semantics.
fn float_pow(receiver: ValueRef, exponent: ValueRef) -> ValueRef {
    float_pair(receiver, exponent, libm::pow, "pow")
}

/// Reads one runtime Float for a numeric method.
fn float_value(value: ValueRef, message: &str) -> Result<f64, ValueRef> {
    match runtime::value(value) {
        RtValue::Float(value) => Ok(*value),
        _ => Err(runtime::recoverable_error("TypeError", message, value)),
    }
}

/// Applies one binary Float operation after validating both operands.
fn float_pair(
    receiver: ValueRef,
    other: ValueRef,
    operation: fn(f64, f64) -> f64,
    name: &str,
) -> ValueRef {
    match (
        float_value(receiver, &format!("{name} requires a Float receiver")),
        float_value(other, &format!("{name} requires a Float argument")),
    ) {
        (Ok(left), Ok(right)) => runtime::allocate(RtValue::Float(operation(left, right))),
        (Err(error), _) | (_, Err(error)) => error,
    }
}

/// Tests one Float predicate after validating the receiver.
fn float_predicate(receiver: ValueRef, predicate: fn(f64) -> bool, name: &str) -> ValueRef {
    match runtime::value(receiver) {
        RtValue::Float(value) => runtime::allocate(RtValue::Bool(predicate(*value))),
        _ => runtime::recoverable_error(
            "TypeError",
            &format!("{name} requires a Float receiver"),
            receiver,
        ),
    }
}

/// Returns the stable kind string stored by one Error.
pub(crate) fn error_kind(value: ValueRef) -> ValueRef {
    let kind = match runtime::value(value) {
        RtValue::Error(error) => String::from(error.kind.as_ref()),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "kind requires an Error receiver",
                value,
            );
        }
    };
    string_value_result(kind)
}

/// Returns the human-readable message string stored by one Error.
pub(crate) fn error_message(value: ValueRef) -> ValueRef {
    let message = match runtime::value(value) {
        RtValue::Error(error) => String::from(error.message.as_ref()),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "message requires an Error receiver",
                value,
            );
        }
    };
    string_value_result(message)
}

/// Returns the associated language data stored by one Error.
pub(crate) fn error_data(value: ValueRef) -> ValueRef {
    match runtime::value(value) {
        RtValue::Error(error) => error.data,
        _ => runtime::recoverable_error("TypeError", "data requires an Error receiver", value),
    }
}

/// Returns the related Error cause or None when no cause is available.
pub(crate) fn error_cause(value: ValueRef) -> ValueRef {
    let cause = match runtime::value(value) {
        RtValue::Error(error) => error.cause,
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "cause requires an Error receiver",
                value,
            );
        }
    };
    cause.unwrap_or_else(|| runtime::allocate(RtValue::None))
}

/// Dispatches a statically named runtime member method.
pub(crate) fn call_method(receiver: ValueRef, method: ValueRef, arguments: ValueRef) -> ValueRef {
    let method = match string_value(method) {
        Ok(method) => method,
        Err(error) => return error,
    };
    match method.as_str() {
        "add" => match list::operations::single_argument(arguments) {
            Ok(argument) => add(receiver, argument),
            Err(error) => error,
        },
        "sub" => match list::operations::single_argument(arguments) {
            Ok(argument) => subtract(receiver, argument),
            Err(error) => error,
        },
        "mul" => match list::operations::single_argument(arguments) {
            Ok(argument) => multiply(receiver, argument),
            Err(error) => error,
        },
        "div" => match list::operations::single_argument(arguments) {
            Ok(argument) => divide(receiver, argument),
            Err(error) => error,
        },
        "div_euclid" => match list::operations::single_argument(arguments) {
            Ok(argument) => numeric::divide_euclid(receiver, argument),
            Err(error) => error,
        },
        "rem_euclid" => match list::operations::single_argument(arguments) {
            Ok(argument) => numeric::remainder_euclid(receiver, argument),
            Err(error) => error,
        },
        "compare" => match list::operations::single_argument(arguments) {
            Ok(argument) => compare(receiver, argument),
            Err(error) => error,
        },
        "to_string" | "debug" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => render_default(receiver),
            Err(error) => error,
        },
        "abs" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => match runtime::value(receiver) {
                RtValue::Int(_) => integer_abs(receiver),
                RtValue::Float(_) => float_abs(receiver),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "abs requires an Int or Float receiver",
                    receiver,
                ),
            },
            Err(error) => error,
        },
        "floor" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => float_floor(receiver),
            Err(error) => error,
        },
        "ceil" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => float_ceil(receiver),
            Err(error) => error,
        },
        "round" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => float_round(receiver),
            Err(error) => error,
        },
        "trunc" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => float_trunc(receiver),
            Err(error) => error,
        },
        "fract" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => float_fract(receiver),
            Err(error) => error,
        },
        "signum" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => integer_signum(receiver),
            Err(error) => error,
        },
        "is_even" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => integer_is_even(receiver),
            Err(error) => error,
        },
        "is_odd" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => integer_is_odd(receiver),
            Err(error) => error,
        },
        "gcd" => match list::operations::single_argument(arguments) {
            Ok(other) => integer_gcd(receiver, other),
            Err(error) => error,
        },
        "lcm" => match list::operations::single_argument(arguments) {
            Ok(other) => integer_lcm(receiver, other),
            Err(error) => error,
        },
        "pow" => match list::operations::single_argument(arguments) {
            Ok(other) => match runtime::value(receiver) {
                RtValue::Int(_) => integer_pow(receiver, other),
                RtValue::Float(_) => float_pow(receiver, other),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "pow requires an Int or Float receiver",
                    receiver,
                ),
            },
            Err(error) => error,
        },
        "min" => match list::operations::single_argument(arguments) {
            Ok(other) => match runtime::value(receiver) {
                RtValue::Int(_) => integer_min(receiver, other),
                RtValue::Float(_) => float_min(receiver, other),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "min requires an Int or Float receiver",
                    receiver,
                ),
            },
            Err(error) => error,
        },
        "max" => match list::operations::single_argument(arguments) {
            Ok(other) => match runtime::value(receiver) {
                RtValue::Int(_) => integer_max(receiver, other),
                RtValue::Float(_) => float_max(receiver, other),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "max requires an Int or Float receiver",
                    receiver,
                ),
            },
            Err(error) => error,
        },
        "clamp" => match list::operations::two_arguments(arguments) {
            Ok((minimum, maximum)) => match runtime::value(receiver) {
                RtValue::Int(_) => integer_clamp(receiver, minimum, maximum),
                RtValue::Float(_) => float_clamp(receiver, minimum, maximum),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "clamp requires an Int or Float receiver",
                    receiver,
                ),
            },
            Err(error) => error,
        },
        "to_float" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => integer_to_float(receiver),
            Err(error) => error,
        },
        "to_int" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => float_to_int(receiver),
            Err(error) => error,
        },
        "is_nan" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => float_is_nan(receiver),
            Err(error) => error,
        },
        "is_finite" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => float_is_finite(receiver),
            Err(error) => error,
        },
        "is_infinite" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => float_is_infinite(receiver),
            Err(error) => error,
        },
        "sqrt" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => float_sqrt(receiver),
            Err(error) => error,
        },
        "clone" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => clone::deep_clone(receiver),
            Err(error) => error,
        },
        "length" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => length(receiver),
            Err(error) => error,
        },
        "is_empty" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => is_empty(receiver),
            Err(error) => error,
        },
        "to_list" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => bytes_to_list(receiver),
            Err(error) => error,
        },
        "slice" => match list::operations::two_arguments(arguments) {
            Ok((start, end)) => match runtime::value(receiver) {
                RtValue::Bytes(_) => bytes_slice(receiver, start, end),
                RtValue::String(_) => string_slice(receiver, start, end),
                RtValue::List(_) => list::operations::slice(receiver, start, end),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "slice requires a Bytes, List, or String receiver",
                    receiver,
                ),
            },
            Err(error) => error,
        },
        "concat" => match list::operations::single_argument(arguments) {
            Ok(other) => bytes_concat(receiver, other),
            Err(error) => error,
        },
        "decode_utf8" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => bytes_decode_utf8(receiver),
            Err(error) => error,
        },
        "encode_utf8" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => bytes_from_utf8(receiver),
            Err(error) => error,
        },
        "contains" => match list::operations::single_argument(arguments) {
            Ok(other) => match runtime::value(receiver) {
                RtValue::Bytes(_) => bytes_contains(receiver, other),
                RtValue::String(_) => string_contains(receiver, other),
                RtValue::List(_) => list::operations::contains(receiver, other),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "contains requires a Bytes, List, or String receiver",
                    receiver,
                ),
            },
            Err(error) => error,
        },
        "starts_with" => match list::operations::single_argument(arguments) {
            Ok(other) => match runtime::value(receiver) {
                RtValue::Bytes(_) => bytes_starts_with(receiver, other),
                RtValue::String(_) => string_starts_with(receiver, other),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "starts_with requires a Bytes or String receiver",
                    receiver,
                ),
            },
            Err(error) => error,
        },
        "ends_with" => match list::operations::single_argument(arguments) {
            Ok(other) => match runtime::value(receiver) {
                RtValue::Bytes(_) => bytes_ends_with(receiver, other),
                RtValue::String(_) => string_ends_with(receiver, other),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "ends_with requires a Bytes or String receiver",
                    receiver,
                ),
            },
            Err(error) => error,
        },
        "repeat" => match list::operations::single_argument(arguments) {
            Ok(count) => match runtime::value(receiver) {
                RtValue::Bytes(_) => bytes_repeat(receiver, count),
                RtValue::String(_) => string_repeat(receiver, count),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "repeat requires a Bytes or String receiver",
                    receiver,
                ),
            },
            Err(error) => error,
        },
        "to_hex" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => bytes_to_hex(receiver),
            Err(error) => error,
        },
        "to_base64" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => bytes_to_base64(receiver),
            Err(error) => error,
        },
        "trim" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => string_trim(receiver),
            Err(error) => error,
        },
        "trim_start" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => string_trim_start(receiver),
            Err(error) => error,
        },
        "trim_end" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => string_trim_end(receiver),
            Err(error) => error,
        },
        "replace" => match list::operations::two_arguments(arguments) {
            Ok((from, to)) => string_replace(receiver, from, to),
            Err(error) => error,
        },
        "split" => match list::operations::single_argument(arguments) {
            Ok(delimiter) => string_split(receiver, delimiter),
            Err(error) => error,
        },
        "to_lowercase" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => string_to_lowercase(receiver),
            Err(error) => error,
        },
        "to_uppercase" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => string_to_uppercase(receiver),
            Err(error) => error,
        },
        "kind" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => error_kind(receiver),
            Err(error) => error,
        },
        "message" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => error_message(receiver),
            Err(error) => error,
        },
        "data" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => error_data(receiver),
            Err(error) => error,
        },
        "cause" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => error_cause(receiver),
            Err(error) => error,
        },
        "next" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => iterator_next(receiver),
            Err(error) => error,
        },
        "push" => match list::operations::single_argument(arguments) {
            Ok(item) => list::operations::append(receiver, item),
            Err(error) => error,
        },
        "get" => match list::operations::single_argument(arguments) {
            Ok(index) => list::operations::get_or_none(receiver, index),
            Err(error) => error,
        },
        "first" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => list::operations::first(receiver),
            Err(error) => error,
        },
        "last" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => list::operations::last(receiver),
            Err(error) => error,
        },
        "extend" => match list::operations::single_argument(arguments) {
            Ok(other) => list::operations::extend(receiver, other),
            Err(error) => error,
        },
        "reverse" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => list::operations::reverse(receiver),
            Err(error) => error,
        },
        "reversed" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => list::operations::reversed(receiver),
            Err(error) => error,
        },
        "index_of" => match list::operations::single_argument(arguments) {
            Ok(item) => list::operations::index_of(receiver, item),
            Err(error) => error,
        },
        "last_index_of" => match list::operations::single_argument(arguments) {
            Ok(item) => list::operations::last_index_of(receiver, item),
            Err(error) => error,
        },
        "join" => match list::operations::single_argument(arguments) {
            Ok(separator) => list::operations::join(receiver, separator),
            Err(error) => error,
        },
        "pop" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => list::operations::pop(receiver),
            Err(error) => error,
        },
        "insert" => match list::operations::two_arguments(arguments) {
            Ok((index, value)) => list::operations::insert(receiver, index, value),
            Err(error) => error,
        },
        "remove" => match list::operations::single_argument(arguments) {
            Ok(index) => list::operations::remove(receiver, index),
            Err(error) => error,
        },
        "clear" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => match runtime::value(receiver) {
                RtValue::List(_) => list::operations::clear(receiver),
                RtValue::Object(_) => object::operations::clear(receiver),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "clear requires a List or Object receiver",
                    receiver,
                ),
            },
            Err(error) => error,
        },
        "has" => match list::operations::single_argument(arguments) {
            Ok(key) => object::operations::has(receiver, key),
            Err(error) => error,
        },
        "delete" => match list::operations::single_argument(arguments) {
            Ok(key) => object::operations::delete(receiver, key),
            Err(error) => error,
        },
        "keys" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => object::operations::keys(receiver),
            Err(error) => error,
        },
        "values" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => object::operations::values(receiver),
            Err(error) => error,
        },
        "get_or" => match list::operations::two_arguments(arguments) {
            Ok((key, default)) => object::operations::get_or(receiver, key, default),
            Err(error) => error,
        },
        "entries" => match list::operations::require_no_arguments(arguments) {
            Ok(()) => object::operations::entries(receiver),
            Err(error) => error,
        },
        "merge" => match list::operations::single_argument(arguments) {
            Ok(other) => object::operations::merge(receiver, other),
            Err(error) => error,
        },
        _ => runtime::recoverable_error(
            "MethodNotFound",
            "receiver does not support this method",
            receiver,
        ),
    }
}

/// Creates the stable built-in representation shared by `ToString` and `Debug` defaults.
fn render_default(receiver: ValueRef) -> ValueRef {
    let rendered = match runtime::value(receiver) {
        RtValue::None => "None".to_owned(),
        RtValue::Error(_) => "Error".to_owned(),
        RtValue::Bool(value) => value.to_string(),
        RtValue::Int(value) => value.to_string(),
        RtValue::Float(value) => value.to_string(),
        RtValue::String(value) => String::from(value.as_str()),
        RtValue::Bytes(value) => format!("Bytes({})", value.as_slice().len()),
        RtValue::List(_) => "[]".to_owned(),
        RtValue::Object(object) => object.enum_data.as_ref().map_or_else(
            || "{}".to_owned(),
            |enumeration| format!("{}::{}", enumeration.type_identity, enumeration.variant),
        ),
        RtValue::Cell(_) => "Cell".to_owned(),
        RtValue::Closure(_) => "fn main()".to_owned(),
        RtValue::BoxedFutureValue(_) => "Future".to_owned(),
    };
    string_value_result(rendered)
}

/// Applies one unary Float operation after validating the receiver type.
fn float_unary(value: ValueRef, operation: fn(f64) -> f64, error_message: &str) -> ValueRef {
    match runtime::value(value) {
        RtValue::Float(value) => runtime::allocate(RtValue::Float(operation(*value))),
        _ => runtime::recoverable_error("TypeError", error_message, value),
    }
}

/// Allocates one runtime String result from owned UTF-8 contents.
fn string_value_result(value: String) -> ValueRef {
    runtime::allocate(RtValue::String(Box::new(RuntimeString::from_string(value))))
}

/// Copies one runtime String value for use as a key or method name.
pub(crate) fn string_value(reference: ValueRef) -> Result<String, ValueRef> {
    match runtime::value(reference) {
        RtValue::String(value) => Ok(value.as_str().into()),
        _ => Err(runtime::recoverable_error(
            "TypeError",
            "Object keys and method names require a String value",
            reference,
        )),
    }
}
