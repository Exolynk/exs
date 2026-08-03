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
pub(crate) fn is_numeric(value: &RtValue) -> bool {
    matches!(
        value,
        RtValue::Bool(_) | RtValue::Int(_) | RtValue::Float(_)
    )
}

/// Converts one runtime numeric payload into the shared numeric dispatch form.
pub(crate) fn number_of(value: &RtValue) -> Option<Number> {
    match value {
        RtValue::Bool(false) => Some(Number::Int(0)),
        RtValue::Bool(true) => Some(Number::Int(1)),
        RtValue::Int(value) => Some(Number::Int(*value)),
        RtValue::Float(value) => Some(Number::Float(*value)),
        RtValue::None
        | RtValue::Error(_)
        | RtValue::String(_)
        | RtValue::List(_)
        | RtValue::Object(_)
        | RtValue::Cell(_)
        | RtValue::Closure(_)
        | RtValue::BoxedFutureValue(_) => None,
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
            Some(value) if is_valid_int(value) => runtime::allocate(RtValue::Int(value)),
            _ => runtime::recoverable_error(
                "IntOverflowError",
                "integer arithmetic overflowed the ExS integer range",
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

/// Negates one runtime numeric value.
pub(crate) fn negate(value: ValueRef) -> ValueRef {
    match number_of_ref(value) {
        Ok(Number::Int(number)) => match number.checked_neg() {
            Some(value) if is_valid_int(value) => runtime::allocate(RtValue::Int(value)),
            _ => runtime::recoverable_error(
                "IntOverflowError",
                "integer negation overflowed the ExS integer range",
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
    runtime::allocate(RtValue::Bool(result))
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
        RtValue::Bool(result) => runtime::allocate(RtValue::Bool(!result)),
        _ => runtime::recoverable_error("TypeError", "! requires a Bool value", value),
    }
}

/// Validates one source value as a Boolean condition while retaining the ValueRef representation.
pub(crate) fn condition_value(value: ValueRef) -> ValueRef {
    match runtime::value(value) {
        RtValue::Bool(_) => value,
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
