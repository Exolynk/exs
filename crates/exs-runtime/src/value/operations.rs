//! Dynamic Wasm operations shared across runtime value kinds.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use exs_abi::{STANDARD_ORDERING_TYPE_ID, STANDARD_ORDERING_TYPE_IDENTITY};
use exs_value::ValueRef;

use crate::gc;
use crate::runtime;
use crate::value::{
    RtValue, RuntimeEnum, RuntimeList, RuntimeObject, RuntimeString, clone, list, numeric, object,
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
        _ => runtime::recoverable_error(
            "TypeError",
            "index access requires a List or Object receiver",
            receiver,
        ),
    }
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
            let elements = list.elements.clone();
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
            let snapshot = runtime::allocate(RtValue::List(Box::new(RuntimeList { elements })));
            gc::restore_temporary_roots(checkpoint);
            snapshot
        }
        _ => runtime::recoverable_error(
            "NotIterable",
            "for requires a List or String iterable",
            iterable,
        ),
    }
}

/// Returns the scalar or entry count for runtime values with a visible length.
pub(crate) fn length(value: ValueRef) -> ValueRef {
    let length = match runtime::value(value) {
        RtValue::String(value) => value.as_str().chars().count(),
        RtValue::List(value) => value.elements.len(),
        RtValue::Object(value) => value.entries.len(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "length requires a String, List, or Object receiver",
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
        RtValue::List(value) => value.elements.is_empty(),
        RtValue::Object(value) => value.entries.is_empty(),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "is_empty requires a String, List, or Object receiver",
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
        "compare" => match list::operations::single_argument(arguments) {
            Ok(argument) => compare(receiver, argument),
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
