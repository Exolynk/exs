//! Numeric, comparison, and Boolean runtime operations.

use exs_value::ValueRef;

use crate::runtime;
use crate::value::{RtValue, Value};

/// A numeric value after Bool-to-Int conversion.
#[derive(Clone, Copy)]
pub(crate) enum Number {
    /// An exact ExS integer.
    Int(i64),
    /// An IEEE 754 binary64 value.
    Float(f64),
}

/// A supported runtime ordering operation.
pub(crate) enum Ordering {
    /// Strict less-than.
    Less,
    /// Less-than-or-equal.
    LessOrEqual,
    /// Strict greater-than.
    Greater,
    /// Greater-than-or-equal.
    GreaterOrEqual,
}

/// One normalized numeric comparison result.
#[derive(Clone, Copy)]
pub(crate) enum Comparison {
    /// The left value is less than the right value.
    Less,
    /// The values compare equal.
    Equal,
    /// The left value is greater than the right value.
    Greater,
    /// The values have no defined numeric ordering, such as NaN.
    Unordered,
}

/// Returns whether a payload participates in numeric operations.
pub(crate) fn is_numeric(value: Value<'_>) -> bool {
    matches!(value, Value::Bool(_) | Value::Int(_) | Value::Float(_))
}

/// Converts one runtime numeric payload into the shared numeric dispatch form.
pub(crate) fn number_of(value: Value<'_>) -> Option<Number> {
    match value {
        Value::Bool(false) => Some(Number::Int(0)),
        Value::Bool(true) => Some(Number::Int(1)),
        Value::Int(value) => Some(Number::Int(value)),
        Value::Float(value) => Some(Number::Float(value)),
        Value::None
        | Value::Error(_)
        | Value::String(_)
        | Value::Bytes(_)
        | Value::List(_)
        | Value::Iterator(_)
        | Value::Object(_)
        | Value::Cell(_)
        | Value::Closure(_)
        | Value::Future => None,
    }
}

/// Converts one runtime numeric reference into the shared numeric dispatch form.
fn number_of_ref(reference: ValueRef) -> Result<Number, ValueRef> {
    number_of(runtime::value(reference)).ok_or_else(|| {
        runtime::recoverable_error(
            "TypeError",
            "numeric operations require a Bool, Int, or Float value",
            reference,
        )
    })
}

/// Performs a binary numeric operation with Float promotion.
pub(crate) fn arithmetic(
    left: ValueRef,
    right: ValueRef,
    integer_operation: fn(i64, i64) -> Option<i64>,
    float_operation: fn(f64, f64) -> f64,
) -> ValueRef {
    let left_value = left;
    let left = match number_of_ref(left) {
        Ok(value) => value,
        Err(error) => return error,
    };
    let right = match number_of_ref(right) {
        Ok(value) => value,
        Err(error) => return error,
    };
    match (left, right) {
        (Number::Int(left), Number::Int(right)) => match integer_operation(left, right) {
            Some(value) => runtime::allocate(RtValue::Int(value)),
            _ => runtime::recoverable_error(
                "IntOverflowError",
                "integer arithmetic overflowed the ExS signed 64-bit range",
                left_value,
            ),
        },
        (left, right) => runtime::allocate(RtValue::Float(float_operation(
            as_float(left),
            as_float(right),
        ))),
    }
}

/// Divides two numeric values after Bool-to-Int conversion and returns a Float.
pub(crate) fn divide(left: ValueRef, right: ValueRef) -> ValueRef {
    let left = match number_of_ref(left) {
        Ok(value) => value,
        Err(error) => return error,
    };
    let right = match number_of_ref(right) {
        Ok(value) => value,
        Err(error) => return error,
    };
    runtime::allocate(RtValue::Float(as_float(left) / as_float(right)))
}

/// Divides two exact Int values with a Euclidean quotient.
pub(crate) fn divide_euclid(left: ValueRef, right: ValueRef) -> ValueRef {
    integer_euclidean(left, right, i64::checked_div_euclid, "div_euclid")
}

/// Returns the non-negative Euclidean remainder of two exact Int values.
pub(crate) fn remainder_euclid(left: ValueRef, right: ValueRef) -> ValueRef {
    integer_euclidean(left, right, i64::checked_rem_euclid, "rem_euclid")
}

/// Applies one exact Euclidean integer operation and reports recoverable language errors.
fn integer_euclidean(
    left: ValueRef,
    right: ValueRef,
    operation: fn(i64, i64) -> Option<i64>,
    name: &str,
) -> ValueRef {
    let left_value = left;
    let right_value = right;
    let (Value::Int(left), Value::Int(right)) = (runtime::value(left), runtime::value(right))
    else {
        return runtime::recoverable_error(
            "TypeError",
            "Euclidean division requires Int operands",
            left_value,
        );
    };
    if right == 0 {
        return runtime::recoverable_error(
            "DivisionByZeroError",
            "Euclidean division requires a non-zero divisor",
            right_value,
        );
    }
    match operation(left, right) {
        Some(value) => runtime::allocate(RtValue::Int(value)),
        None => runtime::recoverable_error(
            "IntOverflowError",
            &alloc::format!("{name} overflowed the ExS signed 64-bit range"),
            left_value,
        ),
    }
}

/// Negates one runtime numeric value.
pub(crate) fn negate(value: ValueRef) -> ValueRef {
    match number_of_ref(value) {
        Ok(Number::Int(number)) => match number.checked_neg() {
            Some(value) => runtime::allocate(RtValue::Int(value)),
            _ => runtime::recoverable_error(
                "IntOverflowError",
                "integer negation overflowed the ExS signed 64-bit range",
                value,
            ),
        },
        Ok(Number::Float(number)) => runtime::allocate(RtValue::Float(-number)),
        Err(error) => error,
    }
}

/// Compares two runtime numeric values with Float promotion.
pub(crate) fn compare(left: ValueRef, right: ValueRef, ordering: Ordering) -> ValueRef {
    let left = match number_of_ref(left) {
        Ok(value) => value,
        Err(error) => return error,
    };
    let right = match number_of_ref(right) {
        Ok(value) => value,
        Err(error) => return error,
    };
    let result = match (left, right) {
        (Number::Int(left), Number::Int(right)) => match ordering {
            Ordering::Less => left < right,
            Ordering::LessOrEqual => left <= right,
            Ordering::Greater => left > right,
            Ordering::GreaterOrEqual => left >= right,
        },
        (left, right) => match ordering {
            Ordering::Less => as_float(left) < as_float(right),
            Ordering::LessOrEqual => as_float(left) <= as_float(right),
            Ordering::Greater => as_float(left) > as_float(right),
            Ordering::GreaterOrEqual => as_float(left) >= as_float(right),
        },
    };
    runtime::bool_value(result)
}

/// Compares two already-normalized numeric values.
pub(crate) fn numbers_comparison(left: Number, right: Number) -> Comparison {
    let result = match (left, right) {
        (Number::Int(left), Number::Int(right)) => Some(left.cmp(&right)),
        (left, right) => as_float(left).partial_cmp(&as_float(right)),
    };
    match result {
        Some(core::cmp::Ordering::Less) => Comparison::Less,
        Some(core::cmp::Ordering::Equal) => Comparison::Equal,
        Some(core::cmp::Ordering::Greater) => Comparison::Greater,
        None => Comparison::Unordered,
    }
}

/// Tests two numeric values for equality with Float promotion.
pub(crate) fn numbers_equal(left: Number, right: Number) -> bool {
    match (left, right) {
        (Number::Int(left), Number::Int(right)) => left == right,
        (left, right) => as_float(left) == as_float(right),
    }
}

/// Negates one runtime Boolean value.
pub(crate) fn not(value: ValueRef) -> ValueRef {
    match runtime::value(value) {
        Value::Bool(result) => runtime::bool_value(!result),
        _ => runtime::recoverable_error("TypeError", "! requires a Bool value", value),
    }
}

/// Validates one source value as a Boolean condition while retaining the ValueRef representation.
pub(crate) fn condition_value(value: ValueRef) -> ValueRef {
    match runtime::value(value) {
        Value::Bool(_) => value,
        _ => runtime::recoverable_error("TypeError", "condition requires a Bool value", value),
    }
}

/// Converts an already validated runtime Boolean value to a Wasm condition.
pub(crate) fn condition(value: ValueRef) -> i32 {
    i32::from(boolean(value))
}

/// Reads a runtime value as a strict Boolean.
pub(crate) fn boolean(reference: ValueRef) -> bool {
    match runtime::value(reference) {
        Value::Bool(result) => result,
        _ => runtime::trap(),
    }
}

/// Converts a numeric dispatch value into binary64.
fn as_float(value: Number) -> f64 {
    match value {
        Number::Int(value) => value as f64,
        Number::Float(value) => value,
    }
}
