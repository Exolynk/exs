//! Dynamic Wasm operations shared across runtime value kinds.

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

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
fn values_equal(left: ValueRef, right: ValueRef) -> bool {
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
            Ok((start, end)) => bytes_slice(receiver, start, end),
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
            Ok(()) => list::operations::clear(receiver),
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
