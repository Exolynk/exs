//! Dynamic Wasm operations shared across runtime value kinds.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::str::FromStr;

use exs_abi::{
    BuiltinMethod, RANGE_BEGIN_FALLBACK, RANGE_STATE_FINISHED, RANGE_STATE_LAST,
    STANDARD_ITERATOR_STEP_TYPE_IDENTITY, STANDARD_ORDERING_TYPE_ID,
    STANDARD_ORDERING_TYPE_IDENTITY,
};
use exs_value::ValueRef;

use crate::gc;
use crate::runtime;
use crate::value::{
    RtValue, RuntimeBytes, RuntimeEnum, RuntimeIterator, RuntimeList, RuntimeObject, RuntimeString,
    SharedName, Value, clone, list, numeric, object,
};

/// Adds two runtime values through String, List, or numeric dispatch.
pub(crate) fn add(left: ValueRef, right: ValueRef) -> ValueRef {
    // String concatenation is the common heap case, so it skips the general value view.
    if let Some(value) = runtime::string_ref(left) {
        return string_add(value, right);
    }
    match runtime::value(left) {
        Value::List(_) => list::operations::add(left, right),
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

/// Appends the decimal rendering of one Int without going through `core::fmt`.
fn push_int(buffer: &mut String, number: i64) {
    let mut digits = [0_u8; 20];
    buffer.push_str(int_text(number, &mut digits));
}

/// Writes the decimal text of one Int into the end of `digits` and returns it.
///
/// Twenty bytes hold every `i64`, including the sign of `i64::MIN`.
fn int_text(number: i64, digits: &mut [u8; 20]) -> &str {
    let mut position = digits.len();
    let mut remaining = number.unsigned_abs();
    loop {
        position -= 1;
        // The remainder is always below ten, so the cast cannot truncate.
        digits[position] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    if number < 0 {
        position -= 1;
        digits[position] = b'-';
    }
    // SAFETY: only ASCII digits and the minus sign were written from `position` onward.
    unsafe { core::str::from_utf8_unchecked(&digits[position..]) }
}

/// Appends the `+` rendering of one String, Bool, Int, or Float, or returns false otherwise.
fn push_scalar(buffer: &mut String, value: ValueRef) -> bool {
    use core::fmt::Write;
    match runtime::value(value) {
        Value::String(text) => buffer.push_str(text.as_str()),
        Value::Bool(flag) => {
            let _ = write!(buffer, "{flag}");
        }
        Value::Int(number) => push_int(buffer, number),
        Value::Float(number) => {
            let _ = write!(buffer, "{number}");
        }
        _ => return false,
    }
    true
}

/// Concatenates a String receiver with one supported scalar right operand.
fn string_add(left: &RuntimeString, right: ValueRef) -> ValueRef {
    if let Some(right) = runtime::string_ref(right) {
        return concat_strings(left.as_str(), right.as_str());
    }
    let mut digits = [0_u8; 20];
    let rendered;
    let right_text = match runtime::value(right) {
        Value::Bool(flag) => {
            if flag {
                "true"
            } else {
                "false"
            }
        }
        Value::Int(number) => int_text(number, &mut digits),
        Value::Float(number) => {
            rendered = number.to_string();
            rendered.as_str()
        }
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "String addition requires a String, Bool, Int, or Float right operand",
                right,
            );
        }
    };
    concat_strings(left.as_str(), right_text)
}

/// Allocates one runtime String holding `left` followed by `right`.
fn concat_strings(left: &str, right: &str) -> ValueRef {
    // Exact capacity keeps the final conversion into the immutable String from reallocating.
    let mut result = String::with_capacity(left.len().saturating_add(right.len()));
    result.push_str(left);
    result.push_str(right);
    string_value_result(result)
}

/// Allocates an empty formatted-string builder with room for `capacity` bytes.
pub(crate) fn format_new(capacity: usize) -> ValueRef {
    runtime::allocate(RtValue::FormatBuilder(Box::new(String::with_capacity(
        capacity,
    ))))
}

/// Returns the growable contents of one builder, or `None` once it turned into an Error.
fn builder_buffer(builder: ValueRef) -> Option<&'static mut String> {
    match runtime::value_mut(builder) {
        Some(RtValue::FormatBuilder(buffer)) => Some(buffer),
        _ => None,
    }
}

/// Appends one formatted part with `+` semantics and returns the builder or resulting Error.
///
/// This matches the former `result + part` chain: unsupported parts produce the same TypeError,
/// and later parts keep being added onto that Error.
pub(crate) fn format_push(builder: ValueRef, part: ValueRef) -> ValueRef {
    match builder_buffer(builder) {
        Some(buffer) => {
            if push_scalar(buffer, part) {
                builder
            } else {
                runtime::recoverable_error(
                    "TypeError",
                    "String addition requires a String, Bool, Int, or Float right operand",
                    part,
                )
            }
        }
        None => add(builder, part),
    }
}

/// Turns a completed builder into an ordinary String in place and returns it.
///
/// A builder that already turned into an Error is returned unchanged.
pub(crate) fn format_finish(builder: ValueRef) -> ValueRef {
    if let Some(slot) = runtime::value_mut(builder)
        && let RtValue::FormatBuilder(buffer) = slot
    {
        let contents = core::mem::take(buffer.as_mut());
        *slot = RtValue::String(RuntimeString::from_string(contents));
    }
    builder
}

/// Appends the default `ToString` rendering of one value and returns the builder or Error.
pub(crate) fn format_push_default(builder: ValueRef, value: ValueRef) -> ValueRef {
    match builder_buffer(builder) {
        Some(buffer) => {
            push_default(buffer, value);
            builder
        }
        None => {
            let checkpoint = gc::temporary_root_checkpoint();
            gc::push_temporary_root(builder);
            let rendered = render_default(value);
            gc::push_temporary_root(rendered);
            let result = add(builder, rendered);
            gc::restore_temporary_roots(checkpoint);
            result
        }
    }
}

/// Tests two runtime values for equality.
pub(crate) fn equal(left: ValueRef, right: ValueRef) -> ValueRef {
    runtime::bool_value(values_equal(left, right))
}

/// One built-in comparison outcome before it becomes a source-visible `std::Ordering` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComparisonOrdering {
    /// The left value orders before the right value.
    Less,
    /// Both values are equal.
    Equal,
    /// The left value orders after the right value.
    Greater,
    /// The values have no defined order.
    Unordered,
}

impl ComparisonOrdering {
    /// Returns the matching `std::Ordering` variant name.
    const fn variant(self) -> &'static str {
        match self {
            Self::Less => "Less",
            Self::Equal => "Equal",
            Self::Greater => "Greater",
            Self::Unordered => "Unordered",
        }
    }

    /// Parses one `std::Ordering` variant name.
    fn from_variant(variant: &str) -> Option<Self> {
        match variant {
            "Less" => Some(Self::Less),
            "Equal" => Some(Self::Equal),
            "Greater" => Some(Self::Greater),
            "Unordered" => Some(Self::Unordered),
            _ => None,
        }
    }

    /// Applies one compiler comparison test code, or returns the TypeError message.
    fn test(self, test: i32) -> Result<bool, &'static str> {
        match (self, test) {
            (Self::Equal, 0)
            | (Self::Unordered | Self::Less | Self::Greater, 1)
            | (Self::Less, 2 | 3)
            | (Self::Equal, 3 | 5)
            | (Self::Greater, 4 | 5) => Ok(true),
            (Self::Unordered | Self::Less | Self::Greater, 0)
            | (Self::Equal, 1 | 2 | 4)
            | (Self::Less, 4 | 5)
            | (Self::Greater, 2 | 3) => Ok(false),
            (Self::Unordered, 2..=5) => Err("ordering comparison requires comparable values"),
            _ => Err("invalid Ordering value or comparison operator"),
        }
    }
}

/// Orders two built-in runtime values.
fn builtin_ordering(left: ValueRef, right: ValueRef) -> ComparisonOrdering {
    match (runtime::value(left), runtime::value(right)) {
        (left, right) if numeric::is_numeric(left) && numeric::is_numeric(right) => {
            match (numeric::number_of(left), numeric::number_of(right)) {
                (Some(left), Some(right)) => match numeric::numbers_comparison(left, right) {
                    numeric::Comparison::Less => ComparisonOrdering::Less,
                    numeric::Comparison::Equal => ComparisonOrdering::Equal,
                    numeric::Comparison::Greater => ComparisonOrdering::Greater,
                    numeric::Comparison::Unordered => ComparisonOrdering::Unordered,
                },
                _ => ComparisonOrdering::Unordered,
            }
        }
        (Value::String(left), Value::String(right)) => match left.as_str().cmp(right.as_str()) {
            core::cmp::Ordering::Less => ComparisonOrdering::Less,
            core::cmp::Ordering::Equal => ComparisonOrdering::Equal,
            core::cmp::Ordering::Greater => ComparisonOrdering::Greater,
        },
        _ if values_equal(left, right) => ComparisonOrdering::Equal,
        _ => ComparisonOrdering::Unordered,
    }
}

/// Compares two runtime values and returns the compiler-owned Ordering enum.
pub(crate) fn compare(left: ValueRef, right: ValueRef) -> ValueRef {
    ordering_value(builtin_ordering(left, right).variant())
}

/// Compares two built-in runtime values and applies one comparison test without an Ordering.
///
/// Produces the same Bool or TypeError as `compare` followed by `ordering_test`.
pub(crate) fn compare_test(left: ValueRef, right: ValueRef, test: i32) -> ValueRef {
    let ordering = builtin_ordering(left, right);
    match ordering.test(test) {
        Ok(result) => runtime::bool_value(result),
        Err(message) => {
            let data = ordering_value(ordering.variant());
            runtime::recoverable_error("TypeError", message, data)
        }
    }
}

/// Interprets one compiler-owned Ordering value for a source comparison operator.
pub(crate) fn ordering_test(ordering: ValueRef, test: i32) -> ValueRef {
    let variant = match runtime::value(ordering) {
        Value::Object(object)
            if object.type_id == Some(STANDARD_ORDERING_TYPE_ID)
                && object.enum_data.as_ref().is_some_and(|data| {
                    data.type_identity.as_ref() == STANDARD_ORDERING_TYPE_IDENTITY
                }) =>
        {
            object
                .enum_data
                .as_ref()
                .map(|data| ComparisonOrdering::from_variant(data.variant.as_ref()))
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
    let result = match variant {
        Some(variant) => variant.test(test),
        None => Err("invalid Ordering value or comparison operator"),
    };
    match result {
        Ok(result) => runtime::bool_value(result),
        Err(message) => runtime::recoverable_error("TypeError", message, ordering),
    }
}

/// Returns whether two runtime values satisfy ExS equality semantics without allocating a Bool.
pub(crate) fn values_equal(left: ValueRef, right: ValueRef) -> bool {
    match (runtime::value(left), runtime::value(right)) {
        (Value::None, Value::None) => true,
        (left, right) if numeric::is_numeric(left) && numeric::is_numeric(right) => {
            match (numeric::number_of(left), numeric::number_of(right)) {
                (Some(left), Some(right)) => numeric::numbers_equal(left, right),
                _ => false,
            }
        }
        (Value::String(left), Value::String(right)) => left.as_str() == right.as_str(),
        (Value::Bytes(left), Value::Bytes(right)) => left.as_slice() == right.as_slice(),
        (Value::List(_), Value::List(_))
        | (Value::Object(_), Value::Object(_))
        | (Value::Error(_), Value::Error(_))
        | (Value::Closure(_), Value::Closure(_)) => left == right,
        _ => false,
    }
}

/// Allocates one zero-payload compiler-owned `std::Ordering` enum value.
fn ordering_value(variant: &str) -> ValueRef {
    runtime::allocate(RtValue::Object(Box::new(RuntimeObject::enumeration(
        Some(STANDARD_ORDERING_TYPE_ID),
        RuntimeEnum {
            type_identity: STANDARD_ORDERING_TYPE_IDENTITY.into(),
            variant: variant.into(),
            fields: Vec::new(),
        },
    ))))
}

/// Tests two runtime values for inequality.
pub(crate) fn not_equal(left: ValueRef, right: ValueRef) -> ValueRef {
    let equal = equal(left, right);
    runtime::bool_value(!numeric::boolean(equal))
}

/// Appends a value through the receiver's runtime collection dispatch.
pub(crate) fn append(receiver: ValueRef, item: ValueRef) -> ValueRef {
    list::operations::append(receiver, item)
}

/// Reads one value through the receiver's runtime indexing dispatch.
pub(crate) fn index_get(receiver: ValueRef, index: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        Value::List(_) => list::operations::get(receiver, index),
        Value::Object(_) => object::operations::get(receiver, index),
        Value::Bytes(_) => bytes_get(receiver, index),
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
        Value::List(values) => {
            let mut bytes = Vec::with_capacity(values.elements.len());
            for value in &values.elements {
                match runtime::value(*value) {
                    Value::Int(value) if (0..=255).contains(&value) => bytes.push(value as u8),
                    Value::Int(_) => {
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
        Value::String(value) => bytes_value(value.as_str().as_bytes().to_vec()),
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
        Value::String(value) => value.as_str(),
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
        Value::String(value) => value.as_str(),
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
        Value::Bytes(bytes) => match bytes.as_slice().get(index) {
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
        Value::Bytes(bytes) if start <= end && end <= bytes.as_slice().len() => {
            bytes_value(bytes.as_slice()[start..end].to_vec())
        }
        Value::Bytes(_) => runtime::recoverable_error(
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
        (Value::Bytes(left), Value::Bytes(right)) => {
            let mut bytes = Vec::with_capacity(left.as_slice().len() + right.as_slice().len());
            bytes.extend_from_slice(left.as_slice());
            bytes.extend_from_slice(right.as_slice());
            bytes_value(bytes)
        }
        (Value::Bytes(_), _) => {
            runtime::recoverable_error("TypeError", "concat requires a Bytes argument", other)
        }
        _ => runtime::recoverable_error("TypeError", "concat requires a Bytes receiver", receiver),
    }
}

/// Returns a new List containing the receiver's octets as Int values.
fn bytes_to_list(receiver: ValueRef) -> ValueRef {
    let bytes = match runtime::value(receiver) {
        Value::Bytes(bytes) => bytes.as_slice().to_vec(),
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
        let Some(RtValue::List(list_value)) = runtime::value_mut(list) else {
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
        Value::Bytes(bytes) => match core::str::from_utf8(bytes.as_slice()) {
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
        Value::Bytes(value) => value.as_slice(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "contains requires a Bytes receiver",
                receiver,
            );
        }
    };
    let needle = match runtime::value(needle) {
        Value::Bytes(value) => value.as_slice(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "contains requires a Bytes argument",
                needle,
            );
        }
    };
    runtime::bool_value(
        needle.is_empty() || value.windows(needle.len()).any(|window| window == needle),
    )
}

/// Applies one binary Bytes predicate after validating both operands.
fn bytes_relation(
    receiver: ValueRef,
    other: ValueRef,
    predicate: fn(&[u8], &[u8]) -> bool,
    name: &str,
) -> ValueRef {
    let value = match runtime::value(receiver) {
        Value::Bytes(value) => value.as_slice(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                &format!("{name} requires a Bytes receiver"),
                receiver,
            );
        }
    };
    let other = match runtime::value(other) {
        Value::Bytes(value) => value.as_slice(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                &format!("{name} requires a Bytes argument"),
                other,
            );
        }
    };
    runtime::bool_value(predicate(value, other))
}

/// Repeats one Bytes value a non-negative number of times.
fn bytes_repeat(receiver: ValueRef, count: ValueRef) -> ValueRef {
    let count = match non_negative_count(count, "repeat") {
        Ok(count) => count,
        Err(error) => return error,
    };
    let bytes = match runtime::value(receiver) {
        Value::Bytes(bytes) => bytes.as_slice(),
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
        Value::Bytes(bytes) => bytes.as_slice(),
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
        Value::String(value) => value.as_str().as_bytes(),
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
        Value::Bytes(bytes) => bytes.as_slice(),
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
        Value::String(value) => value.as_str().as_bytes(),
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
        Value::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "slice requires a String receiver",
                receiver,
            );
        }
    };
    let scalar_count = if value.is_ascii() {
        value.len()
    } else {
        value.chars().count()
    };
    if start > end || end > scalar_count {
        return runtime::recoverable_error(
            "IndexError",
            "String slice range is outside the String bounds",
            receiver,
        );
    }
    let (start, end) = if value.is_ascii() {
        (start, end)
    } else {
        (
            scalar_byte_offset(value, start),
            scalar_byte_offset(value, end),
        )
    };
    string_value_result(String::from(&value[start..end]))
}

/// Applies the built-in slice operation without allocating dynamic method arguments.
pub(crate) fn slice(receiver: ValueRef, start: ValueRef, end: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        Value::Bytes(_) => bytes_slice(receiver, start, end),
        Value::String(_) => string_slice(receiver, start, end),
        Value::List(_) => list::operations::slice(receiver, start, end),
        _ => runtime::recoverable_error(
            "TypeError",
            "slice requires a Bytes, List, or String receiver",
            receiver,
        ),
    }
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
        Value::Int(number) if number >= 0 => usize::try_from(number).map_err(|_| {
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

/// Applies the built-in contains operation without allocating dynamic method arguments.
pub(crate) fn contains(receiver: ValueRef, other: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        Value::Bytes(_) => bytes_contains(receiver, other),
        Value::String(_) => string_contains(receiver, other),
        Value::List(_) => list::operations::contains(receiver, other),
        _ => runtime::recoverable_error(
            "TypeError",
            "contains requires a Bytes, List, or String receiver",
            receiver,
        ),
    }
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

/// Applies the built-in starts-with operation without allocating dynamic method arguments.
pub(crate) fn starts_with(receiver: ValueRef, other: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        Value::Bytes(_) => bytes_starts_with(receiver, other),
        Value::String(_) => string_starts_with(receiver, other),
        _ => runtime::recoverable_error(
            "TypeError",
            "starts_with requires a Bytes or String receiver",
            receiver,
        ),
    }
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

/// Applies the built-in ends-with operation without allocating dynamic method arguments.
pub(crate) fn ends_with(receiver: ValueRef, other: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        Value::Bytes(_) => bytes_ends_with(receiver, other),
        Value::String(_) => string_ends_with(receiver, other),
        _ => runtime::recoverable_error(
            "TypeError",
            "ends_with requires a Bytes or String receiver",
            receiver,
        ),
    }
}

/// Applies one binary String predicate after validating both operands.
fn string_relation(
    receiver: ValueRef,
    other: ValueRef,
    predicate: fn(&str, &str) -> bool,
    name: &str,
) -> ValueRef {
    let value = match runtime::value(receiver) {
        Value::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                &format!("{name} requires a String receiver"),
                receiver,
            );
        }
    };
    let other = match runtime::value(other) {
        Value::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                &format!("{name} requires a String argument"),
                other,
            );
        }
    };
    runtime::bool_value(predicate(value, other))
}

/// Trims Unicode whitespace from both ends of one String.
fn string_trim(receiver: ValueRef) -> ValueRef {
    string_transform(receiver, str::trim, "trim")
}

/// Trims Unicode whitespace without allocating dynamic method arguments.
pub(crate) fn trim(receiver: ValueRef) -> ValueRef {
    string_trim(receiver)
}

/// Trims Unicode whitespace from the start of one String.
fn string_trim_start(receiver: ValueRef) -> ValueRef {
    string_transform(receiver, str::trim_start, "trim_start")
}

/// Trims leading Unicode whitespace without allocating dynamic method arguments.
pub(crate) fn trim_start(receiver: ValueRef) -> ValueRef {
    string_trim_start(receiver)
}

/// Trims Unicode whitespace from the end of one String.
fn string_trim_end(receiver: ValueRef) -> ValueRef {
    string_transform(receiver, str::trim_end, "trim_end")
}

/// Trims trailing Unicode whitespace without allocating dynamic method arguments.
pub(crate) fn trim_end(receiver: ValueRef) -> ValueRef {
    string_trim_end(receiver)
}

/// Applies one String-to-String operation after validating the receiver.
fn string_transform(receiver: ValueRef, transform: fn(&str) -> &str, name: &str) -> ValueRef {
    let value = match runtime::value(receiver) {
        Value::String(value) => value.as_str(),
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
        Value::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "replace requires a String receiver",
                receiver,
            );
        }
    };
    let from = match runtime::value(from) {
        Value::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "replace requires a String search value",
                from,
            );
        }
    };
    let to = match runtime::value(to) {
        Value::String(value) => value.as_str(),
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

/// Replaces String fragments without allocating dynamic method arguments.
pub(crate) fn replace(receiver: ValueRef, from: ValueRef, to: ValueRef) -> ValueRef {
    string_replace(receiver, from, to)
}

/// Splits one String around an exact String delimiter into a new List.
fn string_split(receiver: ValueRef, delimiter: ValueRef) -> ValueRef {
    let value = match runtime::value(receiver) {
        Value::String(value) => value.as_str(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "split requires a String receiver",
                receiver,
            );
        }
    };
    let delimiter = match runtime::value(delimiter) {
        Value::String(value) => value.as_str(),
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

/// Splits one String without allocating dynamic method arguments.
pub(crate) fn split(receiver: ValueRef, delimiter: ValueRef) -> ValueRef {
    string_split(receiver, delimiter)
}

/// Returns a new List of runtime Strings while preserving temporary roots during allocation.
fn string_list(values: Vec<String>) -> ValueRef {
    let checkpoint = gc::temporary_root_checkpoint();
    let list = runtime::allocate(RtValue::List(Box::new(RuntimeList::new())));
    gc::push_temporary_root(list);
    for value in values {
        let value = string_value_result(value);
        gc::push_temporary_root(value);
        let Some(RtValue::List(list_value)) = runtime::value_mut(list) else {
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
        Value::String(value) => value.as_str(),
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
        Value::String(value) => value.as_str(),
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
        Value::String(value) => value.as_str(),
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
        Value::Int(number) if number >= 0 => usize::try_from(number).map_err(|_| {
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
        Value::Int(index) if index >= 0 => usize::try_from(index).map_err(|_| {
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
    runtime::allocate(RtValue::Bytes(RuntimeBytes::from_vec(value)))
}

/// Replaces one value through the receiver's runtime indexing dispatch.
pub(crate) fn index_set(receiver: ValueRef, index: ValueRef, replacement: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        Value::List(_) => list::operations::set(receiver, index, replacement),
        Value::Object(_) => object::operations::set(receiver, index, replacement),
        _ => runtime::recoverable_error(
            "TypeError",
            "index assignment requires a List or Object receiver",
            receiver,
        ),
    }
}

/// Creates the shallow List snapshot or lazy immutable iterator consumed by a for loop.
pub(crate) fn iter_snapshot(iterable: ValueRef) -> ValueRef {
    match runtime::value(iterable) {
        Value::List(list) => {
            let mut elements = list.elements.clone();
            elements.reverse();
            runtime::allocate(RtValue::List(Box::new(RuntimeList { elements })))
        }
        Value::String(_) => {
            let checkpoint = gc::temporary_root_checkpoint();
            gc::push_temporary_root(iterable);
            let snapshot =
                runtime::allocate(RtValue::Iterator(Box::new(RuntimeIterator::String {
                    source: iterable,
                    next_byte: 0,
                })));
            gc::restore_temporary_roots(checkpoint);
            snapshot
        }
        Value::Bytes(_) => {
            let checkpoint = gc::temporary_root_checkpoint();
            gc::push_temporary_root(iterable);
            let snapshot = runtime::allocate(RtValue::Iterator(Box::new(RuntimeIterator::Bytes {
                source: iterable,
                next_index: 0,
            })));
            gc::restore_temporary_roots(checkpoint);
            snapshot
        }
        Value::Object(_) => iterable,
        _ => runtime::recoverable_error(
            "NotIterable",
            "for requires an iterable or Iterator receiver",
            iterable,
        ),
    }
}

/// Advances one compiler-created iterator and returns a prelude IteratorStep value.
fn iterator_next(receiver: ValueRef) -> ValueRef {
    match builtin_iterator_item(receiver) {
        Some(item) => iterator_step(item),
        None => runtime::recoverable_error(
            "NotIterable",
            "next requires an Iterator receiver",
            receiver,
        ),
    }
}

/// Advances one runtime List snapshot or String/Bytes iterator without an IteratorStep.
///
/// Returns `None` for receivers that are not compiler-created built-in iterators, and
/// `Some(None)` once the iterator is exhausted.
fn builtin_iterator_item(receiver: ValueRef) -> Option<Option<ValueRef>> {
    let item = match runtime::value(receiver) {
        Value::List(list) => {
            if list.elements.is_empty() {
                None
            } else {
                match runtime::value_mut(receiver) {
                    Some(RtValue::List(list)) => list.elements.pop(),
                    _ => crate::runtime::trap(),
                }
            }
        }
        Value::Iterator(iterator) => match iterator {
            RuntimeIterator::Range { .. } => native_range_next(receiver),
            RuntimeIterator::String { source, next_byte } => {
                let source = *source;
                let next_byte = *next_byte;
                let Value::String(value) = runtime::value(source) else {
                    crate::runtime::trap();
                };
                let Some(suffix) = value.as_str().get(next_byte..) else {
                    crate::runtime::trap();
                };
                match suffix.chars().next() {
                    Some(scalar) => {
                        let next_byte = next_byte.saturating_add(scalar.len_utf8());
                        match runtime::value_mut(receiver) {
                            Some(RtValue::Iterator(iterator)) => match iterator.as_mut() {
                                RuntimeIterator::String {
                                    next_byte: cursor, ..
                                } => *cursor = next_byte,
                                _ => crate::runtime::trap(),
                            },
                            _ => crate::runtime::trap(),
                        }
                        Some(runtime::allocate(RtValue::String(
                            RuntimeString::from_string(scalar.to_string()),
                        )))
                    }
                    None => None,
                }
            }
            RuntimeIterator::Bytes { source, next_index } => {
                let source = *source;
                let next_index = *next_index;
                let Value::Bytes(value) = runtime::value(source) else {
                    crate::runtime::trap();
                };
                let item = value.as_slice().get(next_index).copied();
                item.map(|byte| {
                    match runtime::value_mut(receiver) {
                        Some(RtValue::Iterator(iterator)) => match iterator.as_mut() {
                            RuntimeIterator::Bytes {
                                next_index: cursor, ..
                            } => {
                                *cursor = next_index.saturating_add(1);
                            }
                            _ => crate::runtime::trap(),
                        },
                        _ => crate::runtime::trap(),
                    }
                    runtime::allocate(RtValue::Int(i64::from(byte)))
                })
            }
        },
        _ => return None,
    };
    Some(item)
}

/// `iter_native_next` result for receivers that need `Iterator::next` dispatch.
///
/// All bits set can never be a valid `ValueRef`, which carries exactly one type flag.
const NATIVE_ITER_FALLBACK: i64 = -1;
/// `iter_native_next` result for an exhausted iterator.
const NATIVE_ITER_DONE: i64 = 0;

/// Advances one for-loop snapshot natively when it is a built-in iterator or prelude Range.
///
/// Returns the raw item ValueRef, `NATIVE_ITER_DONE`, or `NATIVE_ITER_FALLBACK` when the
/// caller must dispatch `Iterator::next` instead. A fallback never mutates the receiver.
pub(crate) fn iter_native_next(snapshot: ValueRef, range_type_id: i32) -> i64 {
    let item = match builtin_iterator_item(snapshot) {
        Some(item) => Some(item),
        None => match u32::try_from(range_type_id) {
            Ok(range_type_id) => range_next_fast(snapshot, range_type_id)
                .or_else(|| range_next(snapshot, range_type_id)),
            Err(_) => None,
        },
    };
    match item {
        Some(Some(item)) => item.raw().cast_signed(),
        Some(None) => NATIVE_ITER_DONE,
        None => NATIVE_ITER_FALLBACK,
    }
}

/// Returns a recoverable error for a for-loop snapshot the synchronous lowering cannot advance.
pub(crate) fn iter_unsupported(snapshot: ValueRef) -> ValueRef {
    runtime::recoverable_error(
        "NotIterable",
        "for requires an iterable or Iterator receiver",
        snapshot,
    )
}

/// Current field values of one prelude Range object.
struct RangeState {
    /// Reference to the next candidate item, reused as the yielded Int.
    current_ref: ValueRef,
    /// Next candidate item.
    current: i64,
    /// Range bound.
    end: i64,
    /// Signed distance between items.
    step: i64,
    /// Whether `end` itself is yielded.
    inclusive: bool,
    /// Whether the Range is exhausted.
    done: bool,
    /// Entry indexes of the `current` and `done` fields, which `range_next` rewrites.
    slots: RangeSlots,
}

/// Entry indexes of the mutable prelude Range fields inside one Object.
#[derive(Clone, Copy)]
struct RangeSlots {
    /// Index of the `current` entry.
    current: usize,
    /// Index of the `done` entry.
    done: usize,
}

/// Outcome of advancing one Range cursor by a single step.
enum RangeAdvance {
    /// The cursor has passed its bound and must be marked done.
    Exhausted,
    /// The current item is the bound itself and is the last item.
    Last,
    /// The current item is yielded and the cursor moves to the given value.
    Next(i64),
    /// Moving the cursor would overflow, which the prelude reports as an error.
    Overflow,
}

/// Entry index of the prelude Range `current` field in declaration order.
const RANGE_CURRENT: usize = 0;
/// Entry index of the prelude Range `end` field.
const RANGE_END: usize = 1;
/// Entry index of the prelude Range `step` field.
const RANGE_STEP: usize = 2;
/// Entry index of the prelude Range `inclusive` field.
const RANGE_INCLUSIVE: usize = 3;
/// Entry index of the prelude Range `done` field.
const RANGE_DONE: usize = 4;

/// Mirrors one step of the prelude `Range::next` on plain Int fields that are not yet done.
const fn range_advance(current: i64, end: i64, step: i64, inclusive: bool) -> RangeAdvance {
    let exhausted = match (step > 0, inclusive) {
        (true, true) => current > end,
        (true, false) => current >= end,
        (false, true) => current < end,
        (false, false) => current <= end,
    };
    if exhausted {
        RangeAdvance::Exhausted
    } else if current == end {
        RangeAdvance::Last
    } else {
        match current.checked_add(step) {
            Some(next) => RangeAdvance::Next(next),
            None => RangeAdvance::Overflow,
        }
    }
}

/// Creates the native for-loop iterator of a Range temporary that no source code can observe.
///
/// The compiler only uses this for `start..end` and `start..=end` iterables, whose Range
/// object is unreachable after the snapshot, so its fields need no write-back. Anything that
/// is not a well-formed unit-step Range takes the generic `iter_snapshot` path instead.
pub(crate) fn iter_range_snapshot(iterable: ValueRef, range_type_id: i32) -> ValueRef {
    let state = u32::try_from(range_type_id)
        .ok()
        .and_then(|range_type_id| range_state(iterable, range_type_id));
    match state {
        // A unit step toward the bound can never overflow, so no prelude error is lost.
        Some(state) if state.step.unsigned_abs() == 1 => {
            runtime::allocate(RtValue::Iterator(Box::new(RuntimeIterator::Range {
                current: state.current,
                end: state.end,
                step: state.step,
                inclusive: state.inclusive,
                done: state.done,
            })))
        }
        _ => iter_snapshot(iterable),
    }
}

/// Advances one native Range iterator, returning the yielded Int or `None` once exhausted.
fn native_range_next(receiver: ValueRef) -> Option<ValueRef> {
    let Some(RtValue::Iterator(iterator)) = runtime::value_mut(receiver) else {
        crate::runtime::trap();
    };
    let RuntimeIterator::Range {
        current,
        end,
        step,
        inclusive,
        done,
    } = iterator.as_mut()
    else {
        crate::runtime::trap();
    };
    if *done {
        return None;
    }
    let item = *current;
    match range_advance(item, *end, *step, *inclusive) {
        RangeAdvance::Exhausted => {
            *done = true;
            return None;
        }
        RangeAdvance::Last => *done = true,
        RangeAdvance::Next(next) => *current = next,
        // `iter_range_snapshot` only creates unit-step iterators, which never overflow.
        RangeAdvance::Overflow => crate::runtime::trap(),
    }
    Some(runtime::allocate(RtValue::Int(item)))
}

/// Advances a well-formed prelude Range Object of immediate fields with one Object lookup.
///
/// Returns `None` without mutating for anything else, which then takes `range_next`.
fn range_next_fast(receiver: ValueRef, range_type_id: u32) -> Option<Option<ValueRef>> {
    let [current_ref, end, step, inclusive, done] = prelude_range_fields(receiver, range_type_id)?;
    let item = *current_ref;
    let (current, end, step) = (
        immediate_int(item)?,
        immediate_int(*end)?,
        immediate_int(*step)?,
    );
    let (inclusive, is_done) = (immediate_bool(*inclusive)?, immediate_bool(*done)?);
    if is_done {
        return Some(None);
    }
    match range_advance(current, end, step, inclusive) {
        RangeAdvance::Exhausted => {
            *done = ValueRef::TRUE;
            Some(None)
        }
        RangeAdvance::Last => {
            *done = ValueRef::TRUE;
            Some(Some(item))
        }
        RangeAdvance::Next(next) => {
            *current_ref = ValueRef::small_int(next)?;
            Some(Some(item))
        }
        RangeAdvance::Overflow => None,
    }
}

/// Starts counting an unobserved Range variable in a compiled `for` loop.
///
/// For a well-formed unit-step Range of immediate fields that is not done and not exhausted,
/// returns the current value in the low and the last item in the high 32 bits, both as
/// `i32`. Every other Range, including one whose current value is `i32::MIN`, returns
/// `RANGE_BEGIN_FALLBACK` and is iterated as an Object. Never mutates the Range.
pub(crate) fn range_begin(range: ValueRef, range_type_id: i32) -> i64 {
    let begin = || {
        let [current, end, step, inclusive, done] =
            prelude_range_fields(range, u32::try_from(range_type_id).ok()?)?;
        let (current, end, step) = (
            immediate_int(*current)?,
            immediate_int(*end)?,
            immediate_int(*step)?,
        );
        let (inclusive, done) = (immediate_bool(*inclusive)?, immediate_bool(*done)?);
        if done
            || step.unsigned_abs() != 1
            || current == i64::from(i32::MIN)
            || matches!(
                range_advance(current, end, step, inclusive),
                RangeAdvance::Exhausted
            )
        {
            return None;
        }
        // Not being exhausted, the Range moves toward a last item between current and end.
        let last = i32::try_from(if inclusive { end } else { end - step }).ok()?;
        let current = i32::try_from(current).ok()?;
        Some(i64::from(current.cast_unsigned()) | i64::from(last.cast_unsigned()) << 32)
    };
    begin().unwrap_or(RANGE_BEGIN_FALLBACK)
}

/// Writes the state of an inline Range back into the Range variable once its loop exits.
///
/// `cursor` and `state` are the compiled loop's counter and progress, where `state` is 0
/// before the last item, `RANGE_STATE_LAST` right after it and `RANGE_STATE_FINISHED` once
/// the loop asked for another item. The fields then match what `Range::next` would have left.
pub(crate) fn range_finish(range: ValueRef, range_type_id: i32, cursor: i64, state: i64) {
    let Some([current, end, step, inclusive, done]) = u32::try_from(range_type_id)
        .ok()
        .and_then(|range_type_id| prelude_range_fields(range, range_type_id))
    else {
        return;
    };
    let (Some(inclusive), Some(end), Some(step)) = (
        immediate_bool(*inclusive),
        immediate_int(*end),
        immediate_int(*step),
    ) else {
        return;
    };
    // An exclusive Range moves past its last item onto the end and finishes one call later.
    let (cursor, finished) = match state {
        RANGE_STATE_LAST | RANGE_STATE_FINISHED if inclusive => (cursor, true),
        RANGE_STATE_LAST => (cursor + step, false),
        RANGE_STATE_FINISHED => (end, true),
        _ => (cursor, false),
    };
    if let Some(cursor) = ValueRef::small_int(cursor) {
        *current = cursor;
    }
    if finished {
        *done = ValueRef::TRUE;
    }
}

/// Returns the five fields of a prelude Range Object in declaration order.
///
/// Returns `None` for any other value or a Range whose keys differ from the declaration.
fn prelude_range_fields(
    receiver: ValueRef,
    range_type_id: u32,
) -> Option<[&'static mut ValueRef; 5]> {
    let Some(RtValue::Object(object)) = runtime::value_mut(receiver) else {
        return None;
    };
    if object.type_id != Some(range_type_id) || object.enum_data.is_some() {
        return None;
    }
    let [
        (current_key, current),
        (end_key, end),
        (step_key, step),
        (inclusive_key, inclusive),
        (done_key, done),
    ] = object.entries.as_mut_slice()
    else {
        return None;
    };
    // Prelude Ranges share pinned literal keys, which are never freed, so after one full check
    // their addresses identify the declared layout without comparing bytes.
    let addresses = literal_key_addresses([
        &*current_key,
        &*end_key,
        &*step_key,
        &*inclusive_key,
        &*done_key,
    ]);
    let cache = &mut unsafe { crate::state::runtime() }.range_key_addresses;
    if addresses.is_none() || *cache != addresses {
        if !(key_is(current_key, b"current")
            && key_is(end_key, b"end")
            && key_is(step_key, b"step")
            && key_is(inclusive_key, b"inclusive")
            && key_is(done_key, b"done"))
        {
            return None;
        }
        if addresses.is_some() {
            *cache = addresses;
        }
    }
    Some([current, end, step, inclusive, done])
}

/// Returns the text addresses of five pinned literal keys, or `None` if any key is owned.
fn literal_key_addresses(keys: [&SharedName; 5]) -> Option<[usize; 5]> {
    let mut addresses = [0; 5];
    for (address, key) in addresses.iter_mut().zip(keys) {
        let SharedName::Literal(text) = key else {
            return None;
        };
        *address = text.as_ptr() as usize;
    }
    Some(addresses)
}

/// Returns the value of an immediate Int reference.
fn immediate_int(value: ValueRef) -> Option<i64> {
    match value.immediate() {
        Some(exs_value::Immediate::Int(number)) => Some(i64::from(number)),
        _ => None,
    }
}

/// Returns the value of an immediate Bool reference.
fn immediate_bool(value: ValueRef) -> Option<bool> {
    match value.immediate() {
        Some(exs_value::Immediate::Bool(flag)) => Some(flag),
        _ => None,
    }
}

/// Mirrors the prelude `Range::next` natively, including its field mutations.
///
/// Returns `None` without mutating when the object is not a well-formed Range or the next
/// step would overflow, so the prelude implementation reports the error instead.
fn range_next(receiver: ValueRef, range_type_id: u32) -> Option<Option<ValueRef>> {
    let state = range_state(receiver, range_type_id)?;
    if state.done {
        return Some(None);
    }
    match range_advance(state.current, state.end, state.step, state.inclusive) {
        RangeAdvance::Exhausted => {
            set_object_entry(receiver, state.slots.done, runtime::bool_value(true));
            return Some(None);
        }
        RangeAdvance::Last => {
            set_object_entry(receiver, state.slots.done, runtime::bool_value(true));
        }
        RangeAdvance::Next(next) => {
            // The old current value stays reachable through the Range until it is replaced.
            let next = runtime::allocate(RtValue::Int(next));
            set_object_entry(receiver, state.slots.current, next);
        }
        RangeAdvance::Overflow => return None,
    }
    Some(Some(state.current_ref))
}

/// Reads the fields of one prelude Range object, or `None` when it is not well formed.
///
/// Objects built by the prelude keep the declaration order, which is checked first with
/// fixed-width key comparisons; reordered or extended objects fall back to a key scan.
fn range_state(receiver: ValueRef, range_type_id: u32) -> Option<RangeState> {
    let Value::Object(object) = runtime::value(receiver) else {
        return None;
    };
    if object.type_id != Some(range_type_id) || object.enum_data.is_some() {
        return None;
    }
    let entries = object.entries.as_slice();
    let slots = match entries {
        [(current, _), (end, _), (step, _), (inclusive, _), (done, _)]
            if key_is(current, b"current")
                && key_is(end, b"end")
                && key_is(step, b"step")
                && key_is(inclusive, b"inclusive")
                && key_is(done, b"done") =>
        {
            [
                RANGE_CURRENT,
                RANGE_END,
                RANGE_STEP,
                RANGE_INCLUSIVE,
                RANGE_DONE,
            ]
        }
        _ => {
            let position = |name: &str| entries.iter().position(|(key, _)| key.as_ref() == name);
            [
                position("current")?,
                position("end")?,
                position("step")?,
                position("inclusive")?,
                position("done")?,
            ]
        }
    };
    let field = |index: usize| entries.get(index).map(|(_, value)| *value);
    let int = |index: usize| match runtime::value(field(index)?) {
        Value::Int(value) => Some(value),
        _ => None,
    };
    let boolean = |index: usize| match runtime::value(field(index)?) {
        Value::Bool(value) => Some(value),
        _ => None,
    };
    let [current, end, step, inclusive, done] = slots;
    let current_ref = field(current)?;
    Some(RangeState {
        current_ref,
        current: int(current)?,
        end: int(end)?,
        step: int(step)?,
        inclusive: boolean(inclusive)?,
        done: boolean(done)?,
        slots: RangeSlots { current, done },
    })
}

/// Compares one Object key with a short constant name without a `memcmp` call.
fn key_is<const N: usize>(key: &str, expected: &[u8; N]) -> bool {
    <&[u8; N]>::try_from(key.as_bytes()).is_ok_and(|key| key == expected)
}

/// Replaces the value of one existing Object entry in place.
fn set_object_entry(receiver: ValueRef, index: usize, value: ValueRef) {
    let Some(RtValue::Object(object)) = runtime::value_mut(receiver) else {
        crate::runtime::trap();
    };
    match object.entries.get_mut(index) {
        Some((_, slot)) => *slot = value,
        None => crate::runtime::trap(),
    }
}

/// Wraps one optional iterator item in the compiler-owned IteratorStep enum.
fn iterator_step(item: Option<ValueRef>) -> ValueRef {
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
        Value::String(value) => {
            let value = value.as_str();
            if value.is_ascii() {
                value.len()
            } else {
                value.chars().count()
            }
        }
        Value::Bytes(value) => value.as_slice().len(),
        Value::List(value) => value.elements.len(),
        Value::Object(value) => value.entries.len(),
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
        Value::String(value) => value.as_str().is_empty(),
        Value::Bytes(value) => value.as_slice().is_empty(),
        Value::List(value) => value.elements.len() == 0,
        Value::Object(value) => value.entries.is_empty(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "is_empty requires a Bytes, String, List, or Object receiver",
                value,
            );
        }
    };
    runtime::bool_value(empty)
}

/// Returns the absolute value of one Int when it fits in the ExS signed 64-bit range.
pub(crate) fn integer_abs(receiver: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        Value::Int(value) => match value.checked_abs() {
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
        Value::Int(value) => runtime::allocate(RtValue::Int(value.signum())),
        _ => runtime::recoverable_error("TypeError", "signum requires an Int receiver", receiver),
    }
}

/// Returns whether one Int is evenly divisible by two.
fn integer_is_even(receiver: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        Value::Int(value) => runtime::bool_value(value % 2 == 0),
        _ => runtime::recoverable_error("TypeError", "is_even requires an Int receiver", receiver),
    }
}

/// Returns whether one Int is not evenly divisible by two.
fn integer_is_odd(receiver: ValueRef) -> ValueRef {
    match runtime::value(receiver) {
        Value::Int(value) => runtime::bool_value(value % 2 != 0),
        _ => runtime::recoverable_error("TypeError", "is_odd requires an Int receiver", receiver),
    }
}

/// Raises one Int to a non-negative integer exponent.
fn integer_pow(receiver: ValueRef, exponent: ValueRef) -> ValueRef {
    let base = match runtime::value(receiver) {
        Value::Int(value) => value,
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "pow requires an Int receiver",
                receiver,
            );
        }
    };
    let exponent = match runtime::value(exponent) {
        Value::Int(value) if value >= 0 => value as u64,
        Value::Int(_) => {
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
        Value::Int(value) => value.unsigned_abs(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "gcd requires an Int receiver",
                receiver,
            );
        }
    };
    let right = match runtime::value(other) {
        Value::Int(value) => value.unsigned_abs(),
        _ => return runtime::recoverable_error("TypeError", "gcd requires an Int argument", other),
    };
    integer_from_unsigned(gcd(left, right), receiver, "gcd")
}

/// Returns the least common multiple of two Int values when it fits in Int.
fn integer_lcm(receiver: ValueRef, other: ValueRef) -> ValueRef {
    let left = match runtime::value(receiver) {
        Value::Int(value) => value.unsigned_abs(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "lcm requires an Int receiver",
                receiver,
            );
        }
    };
    let right = match runtime::value(other) {
        Value::Int(value) => value.unsigned_abs(),
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
        Value::Int(value) => Ok(value),
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
        Value::Int(value) => runtime::allocate(RtValue::Float(value as f64)),
        _ => runtime::recoverable_error("TypeError", "to_float requires an Int receiver", receiver),
    }
}

/// Converts one finite Float to a truncated Int when the result fits in Int.
fn float_to_int(receiver: ValueRef) -> ValueRef {
    let value = match runtime::value(receiver) {
        Value::Float(value) => value,
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
        Value::Float(value) => Ok(value),
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
        Value::Float(value) => runtime::bool_value(predicate(value)),
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
        Value::Error(error) => String::from(error.kind.as_ref()),
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
        Value::Error(error) => String::from(error.message.as_ref()),
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
        Value::Error(error) => error.data,
        _ => runtime::recoverable_error("TypeError", "data requires an Error receiver", value),
    }
}

/// Returns the related Error cause or None when no cause is available.
pub(crate) fn error_cause(value: ValueRef) -> ValueRef {
    let cause = match runtime::value(value) {
        Value::Error(error) => error.cause,
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "cause requires an Error receiver",
                value,
            );
        }
    };
    cause.unwrap_or(runtime::none_value())
}

/// Dispatches a statically named runtime member method.
pub(crate) fn call_method(receiver: ValueRef, method: ValueRef, arguments: ValueRef) -> ValueRef {
    let method = match string_value(method) {
        Ok(method) => method,
        Err(error) => return error,
    };
    let Some(builtin) = BuiltinMethod::from_name(method.as_str()) else {
        return runtime::recoverable_error(
            "MethodNotFound",
            "receiver does not support this method",
            receiver,
        );
    };
    let none = runtime::none_value();
    let (first, second) = match builtin.arity() {
        0 => match list::operations::require_no_arguments(arguments) {
            Ok(()) => (none, none),
            Err(error) => return error,
        },
        1 => match list::operations::single_argument(arguments) {
            Ok(argument) => (argument, none),
            Err(error) => return error,
        },
        _ => match list::operations::two_arguments(arguments) {
            Ok(arguments) => arguments,
            Err(error) => return error,
        },
    };
    call_builtin(builtin, receiver, first, second)
}

/// Runs the built-in method with index `method`, as the compiler's direct calls do.
///
/// Traps on an index the compiler never emits.
pub(crate) fn call_builtin_id(
    method: i32,
    receiver: ValueRef,
    first: ValueRef,
    second: ValueRef,
) -> ValueRef {
    let Some(method) = u32::try_from(method).ok().and_then(BuiltinMethod::from_id) else {
        runtime::trap();
    };
    call_builtin(method, receiver, first, second)
}

/// Runs one built-in method with its already counted arguments.
///
/// Shared by the name-based `call_method` and the compiler's direct fixed-arity calls.
///
/// Arguments beyond the method's arity are ignored; callers pass None for them.
pub(crate) fn call_builtin(
    method: BuiltinMethod,
    receiver: ValueRef,
    first: ValueRef,
    second: ValueRef,
) -> ValueRef {
    match method {
        BuiltinMethod::Add => add(receiver, first),
        BuiltinMethod::Sub => subtract(receiver, first),
        BuiltinMethod::Mul => multiply(receiver, first),
        BuiltinMethod::Div => divide(receiver, first),
        BuiltinMethod::DivEuclid => numeric::divide_euclid(receiver, first),
        BuiltinMethod::RemEuclid => numeric::remainder_euclid(receiver, first),
        BuiltinMethod::Compare => compare(receiver, first),
        BuiltinMethod::ToString | BuiltinMethod::Debug => render_default(receiver),
        BuiltinMethod::Abs => match runtime::value(receiver) {
            Value::Int(_) => integer_abs(receiver),
            Value::Float(_) => float_abs(receiver),
            _ => runtime::recoverable_error(
                "TypeError",
                "abs requires an Int or Float receiver",
                receiver,
            ),
        },
        BuiltinMethod::Floor => float_floor(receiver),
        BuiltinMethod::Ceil => float_ceil(receiver),
        BuiltinMethod::Round => float_round(receiver),
        BuiltinMethod::Trunc => float_trunc(receiver),
        BuiltinMethod::Fract => float_fract(receiver),
        BuiltinMethod::Signum => integer_signum(receiver),
        BuiltinMethod::IsEven => integer_is_even(receiver),
        BuiltinMethod::IsOdd => integer_is_odd(receiver),
        BuiltinMethod::Gcd => integer_gcd(receiver, first),
        BuiltinMethod::Lcm => integer_lcm(receiver, first),
        BuiltinMethod::Pow => {
            let other = first;
            match runtime::value(receiver) {
                Value::Int(_) => integer_pow(receiver, other),
                Value::Float(_) => float_pow(receiver, other),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "pow requires an Int or Float receiver",
                    receiver,
                ),
            }
        }
        BuiltinMethod::Min => {
            let other = first;
            match runtime::value(receiver) {
                Value::Int(_) => integer_min(receiver, other),
                Value::Float(_) => float_min(receiver, other),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "min requires an Int or Float receiver",
                    receiver,
                ),
            }
        }
        BuiltinMethod::Max => {
            let other = first;
            match runtime::value(receiver) {
                Value::Int(_) => integer_max(receiver, other),
                Value::Float(_) => float_max(receiver, other),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "max requires an Int or Float receiver",
                    receiver,
                ),
            }
        }
        BuiltinMethod::Clamp => {
            let (minimum, maximum) = (first, second);
            match runtime::value(receiver) {
                Value::Int(_) => integer_clamp(receiver, minimum, maximum),
                Value::Float(_) => float_clamp(receiver, minimum, maximum),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "clamp requires an Int or Float receiver",
                    receiver,
                ),
            }
        }
        BuiltinMethod::ToFloat => integer_to_float(receiver),
        BuiltinMethod::ToInt => float_to_int(receiver),
        BuiltinMethod::IsNan => float_is_nan(receiver),
        BuiltinMethod::IsFinite => float_is_finite(receiver),
        BuiltinMethod::IsInfinite => float_is_infinite(receiver),
        BuiltinMethod::Sqrt => float_sqrt(receiver),
        BuiltinMethod::Clone => clone::deep_clone(receiver),
        BuiltinMethod::Length => length(receiver),
        BuiltinMethod::IsEmpty => is_empty(receiver),
        BuiltinMethod::ToList => bytes_to_list(receiver),
        BuiltinMethod::Slice => {
            let (start, end) = (first, second);
            match runtime::value(receiver) {
                Value::Bytes(_) => bytes_slice(receiver, start, end),
                Value::String(_) => string_slice(receiver, start, end),
                Value::List(_) => list::operations::slice(receiver, start, end),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "slice requires a Bytes, List, or String receiver",
                    receiver,
                ),
            }
        }
        BuiltinMethod::Concat => bytes_concat(receiver, first),
        BuiltinMethod::DecodeUtf8 => bytes_decode_utf8(receiver),
        BuiltinMethod::EncodeUtf8 => bytes_from_utf8(receiver),
        BuiltinMethod::Contains => {
            let other = first;
            match runtime::value(receiver) {
                Value::Bytes(_) => bytes_contains(receiver, other),
                Value::String(_) => string_contains(receiver, other),
                Value::List(_) => list::operations::contains(receiver, other),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "contains requires a Bytes, List, or String receiver",
                    receiver,
                ),
            }
        }
        BuiltinMethod::StartsWith => {
            let other = first;
            match runtime::value(receiver) {
                Value::Bytes(_) => bytes_starts_with(receiver, other),
                Value::String(_) => string_starts_with(receiver, other),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "starts_with requires a Bytes or String receiver",
                    receiver,
                ),
            }
        }
        BuiltinMethod::EndsWith => {
            let other = first;
            match runtime::value(receiver) {
                Value::Bytes(_) => bytes_ends_with(receiver, other),
                Value::String(_) => string_ends_with(receiver, other),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "ends_with requires a Bytes or String receiver",
                    receiver,
                ),
            }
        }
        BuiltinMethod::Repeat => {
            let count = first;
            match runtime::value(receiver) {
                Value::Bytes(_) => bytes_repeat(receiver, count),
                Value::String(_) => string_repeat(receiver, count),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "repeat requires a Bytes or String receiver",
                    receiver,
                ),
            }
        }
        BuiltinMethod::ToHex => bytes_to_hex(receiver),
        BuiltinMethod::ToBase64 => bytes_to_base64(receiver),
        BuiltinMethod::Trim => string_trim(receiver),
        BuiltinMethod::TrimStart => string_trim_start(receiver),
        BuiltinMethod::TrimEnd => string_trim_end(receiver),
        BuiltinMethod::Replace => string_replace(receiver, first, second),
        BuiltinMethod::Split => string_split(receiver, first),
        BuiltinMethod::ToLowercase => string_to_lowercase(receiver),
        BuiltinMethod::ToUppercase => string_to_uppercase(receiver),
        BuiltinMethod::Kind => error_kind(receiver),
        BuiltinMethod::Message => error_message(receiver),
        BuiltinMethod::Data => error_data(receiver),
        BuiltinMethod::Cause => error_cause(receiver),
        BuiltinMethod::Next => iterator_next(receiver),
        BuiltinMethod::Push => list::operations::append(receiver, first),
        BuiltinMethod::Get => {
            let index = first;
            match runtime::value(receiver) {
                Value::List(_) => list::operations::get_or_none(receiver, index),
                Value::Object(_) => object::operations::get(receiver, index),
                _ => runtime::recoverable_error(
                    "TypeError",
                    "get requires a List or Object receiver",
                    receiver,
                ),
            }
        }
        BuiltinMethod::First => list::operations::first(receiver),
        BuiltinMethod::Last => list::operations::last(receiver),
        BuiltinMethod::Extend => list::operations::extend(receiver, first),
        BuiltinMethod::Reverse => list::operations::reverse(receiver),
        BuiltinMethod::Reversed => list::operations::reversed(receiver),
        BuiltinMethod::IndexOf => list::operations::index_of(receiver, first),
        BuiltinMethod::LastIndexOf => list::operations::last_index_of(receiver, first),
        BuiltinMethod::Join => list::operations::join(receiver, first),
        BuiltinMethod::Pop => list::operations::pop(receiver),
        BuiltinMethod::Insert => list::operations::insert(receiver, first, second),
        BuiltinMethod::Remove => list::operations::remove(receiver, first),
        BuiltinMethod::Clear => match runtime::value(receiver) {
            Value::List(_) => list::operations::clear(receiver),
            Value::Object(_) => object::operations::clear(receiver),
            _ => runtime::recoverable_error(
                "TypeError",
                "clear requires a List or Object receiver",
                receiver,
            ),
        },
        BuiltinMethod::Has => object::operations::has(receiver, first),
        BuiltinMethod::Delete => object::operations::delete(receiver, first),
        BuiltinMethod::Keys => object::operations::keys(receiver),
        BuiltinMethod::Values => object::operations::values(receiver),
        BuiltinMethod::GetOr => object::operations::get_or(receiver, first, second),
        BuiltinMethod::Entries => object::operations::entries(receiver),
        BuiltinMethod::Merge => object::operations::merge(receiver, first),
    }
}

/// Creates the stable built-in representation shared by `ToString` and `Debug` defaults.
fn render_default(receiver: ValueRef) -> ValueRef {
    // Ints are the common case, so they skip the growing buffer and get an exact allocation.
    if let Value::Int(number) = runtime::value(receiver) {
        let mut digits = [0_u8; 20];
        return string_value_result(String::from(int_text(number, &mut digits)));
    }
    let mut rendered = String::new();
    push_default(&mut rendered, receiver);
    string_value_result(rendered)
}

/// Appends the default `ToString` and `Debug` rendering of one runtime value.
fn push_default(buffer: &mut String, receiver: ValueRef) {
    use core::fmt::Write;
    let _ = match runtime::value(receiver) {
        Value::None => buffer.write_str("None"),
        Value::Error(_) => buffer.write_str("Error"),
        Value::Bool(value) => write!(buffer, "{value}"),
        Value::Int(value) => {
            push_int(buffer, value);
            Ok(())
        }
        Value::Float(value) => write!(buffer, "{value}"),
        Value::String(value) => buffer.write_str(value.as_str()),
        Value::Bytes(value) => write!(buffer, "Bytes({})", value.as_slice().len()),
        Value::List(_) => buffer.write_str("[]"),
        Value::Iterator(_) => buffer.write_str("Iterator"),
        Value::Object(object) => match object.enum_data.as_ref() {
            Some(enumeration) => write!(
                buffer,
                "{}::{}",
                enumeration.type_identity, enumeration.variant
            ),
            None => buffer.write_str("{}"),
        },
        Value::Cell(_) => buffer.write_str("Cell"),
        Value::Closure(_) => buffer.write_str("fn main()"),
        Value::Future => buffer.write_str("Future"),
        Value::FormatBuilder => buffer.write_str("String"),
    };
}

/// Applies one unary Float operation after validating the receiver type.
fn float_unary(value: ValueRef, operation: fn(f64) -> f64, error_message: &str) -> ValueRef {
    match runtime::value(value) {
        Value::Float(value) => runtime::allocate(RtValue::Float(operation(value))),
        _ => runtime::recoverable_error("TypeError", error_message, value),
    }
}

/// Allocates one runtime String result from owned UTF-8 contents.
fn string_value_result(value: String) -> ValueRef {
    runtime::allocate(RtValue::String(RuntimeString::from_string(value)))
}

/// Borrows one runtime String value for use as a key or method name.
pub(crate) fn string_str(reference: ValueRef) -> Result<&'static str, ValueRef> {
    match runtime::value(reference) {
        Value::String(value) => Ok(value.as_str()),
        _ => Err(runtime::recoverable_error(
            "TypeError",
            "Object keys and method names require a String value",
            reference,
        )),
    }
}

/// Copies one runtime String value for use as a key or method name.
pub(crate) fn string_value(reference: ValueRef) -> Result<String, ValueRef> {
    match runtime::value(reference) {
        Value::String(value) => Ok(value.as_str().into()),
        _ => Err(runtime::recoverable_error(
            "TypeError",
            "Object keys and method names require a String value",
            reference,
        )),
    }
}
