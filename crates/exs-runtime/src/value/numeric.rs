//! Numeric, comparison, and Boolean runtime operations.

use exs_value::{ValueRef, is_valid_int};

use crate::runtime;
use crate::value::RtValue;

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

/// Returns whether a payload participates in numeric operations.
pub(crate) fn is_numeric(value: &RtValue) -> bool {
    matches!(
        value,
        RtValue::Bool(_) | RtValue::Int(_) | RtValue::Float(_)
    )
}

/// Converts one runtime numeric payload into the shared numeric dispatch form.
pub(crate) fn number_of(value: &RtValue) -> Number {
    match value {
        RtValue::Bool(false) => Number::Int(0),
        RtValue::Bool(true) => Number::Int(1),
        RtValue::Int(value) => Number::Int(*value),
        RtValue::Float(value) => Number::Float(*value),
        RtValue::None
        | RtValue::Ok(_)
        | RtValue::Error(_)
        | RtValue::String(_)
        | RtValue::List(_)
        | RtValue::Object(_)
        | RtValue::BoxedFutureValue(_) => runtime::trap(),
    }
}

/// Converts one runtime numeric reference into the shared numeric dispatch form.
fn number_of_ref(reference: ValueRef) -> Number {
    number_of(runtime::value(reference))
}

/// Performs a binary numeric operation with Float promotion.
pub(crate) fn arithmetic(
    left: ValueRef,
    right: ValueRef,
    integer_operation: fn(i64, i64) -> Option<i64>,
    float_operation: fn(f64, f64) -> f64,
) -> ValueRef {
    match (number_of_ref(left), number_of_ref(right)) {
        (Number::Int(left), Number::Int(right)) => match integer_operation(left, right) {
            Some(value) if is_valid_int(value) => runtime::allocate(RtValue::Int(value)),
            _ => runtime::trap(),
        },
        (left, right) => runtime::allocate(RtValue::Float(float_operation(
            as_float(left),
            as_float(right),
        ))),
    }
}

/// Negates one runtime numeric value.
pub(crate) fn negate(value: ValueRef) -> ValueRef {
    match number_of_ref(value) {
        Number::Int(value) => match value.checked_neg() {
            Some(value) if is_valid_int(value) => runtime::allocate(RtValue::Int(value)),
            _ => runtime::trap(),
        },
        Number::Float(value) => runtime::allocate(RtValue::Float(-value)),
    }
}

/// Compares two runtime numeric values with Float promotion.
pub(crate) fn compare(left: ValueRef, right: ValueRef, ordering: Ordering) -> ValueRef {
    let result = match (number_of_ref(left), number_of_ref(right)) {
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
    runtime::allocate(RtValue::Bool(result))
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
    runtime::allocate(RtValue::Bool(!boolean(value)))
}

/// Converts one runtime Boolean value to a Wasm condition.
pub(crate) fn condition(value: ValueRef) -> i32 {
    i32::from(boolean(value))
}

/// Reads a runtime value as a strict Boolean.
pub(crate) fn boolean(reference: ValueRef) -> bool {
    match runtime::value(reference) {
        RtValue::Bool(result) => *result,
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
