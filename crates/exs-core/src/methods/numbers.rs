//! Int and Float methods with v1's exact results, kinds and messages.
//!
//! Unlike operators, methods take Int and Float literally: a Bool is neither.

use exs_layout::ValueRef;
use exs_layout::value::layout::{KIND_SHIFT, type_kind};
use exs_layout::{TYPE_FLOAT, TYPE_INT};

use crate::errors::recoverable;
use crate::memory::Memory;
use crate::numbers::{Number, float_value, int_value, number};

/// Returns the value of an Int, or `None` for every other value including Bools.
pub(crate) fn exact_int(memory: &impl Memory, raw: u64) -> Option<i64> {
    if raw >> KIND_SHIFT != u64::from(type_kind(TYPE_INT)) {
        return None;
    }
    match number(memory, raw)? {
        Number::Int(value) => Some(value),
        Number::Float(_) => None,
    }
}

/// Returns the value of a Float, or `None` for every other value.
pub(crate) fn exact_float(memory: &impl Memory, raw: u64) -> Option<f64> {
    if raw >> KIND_SHIFT != u64::from(type_kind(TYPE_FLOAT)) {
        return None;
    }
    match number(memory, raw)? {
        Number::Float(value) => Some(value),
        Number::Int(_) => None,
    }
}

/// Returns a recoverable TypeError with `message` about `data`.
pub(crate) fn type_error(
    memory: &mut impl Memory,
    vm: u32,
    message: &str,
    data: u64,
) -> Option<u64> {
    recoverable(memory, vm, "TypeError", message, data)
}

/// Returns an immediate Bool.
pub(crate) const fn boolean(flag: bool) -> u64 {
    ValueRef::boolean(flag).raw()
}

/// Reads an Int operand or creates the TypeError `message` about it.
fn int_operand(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
    message: &str,
) -> Result<i64, Option<u64>> {
    exact_int(memory, raw).ok_or_else(|| type_error(memory, vm, message, raw))
}

/// Reads a Float operand or creates the TypeError `message` about it.
fn float_operand(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
    message: &str,
) -> Result<f64, Option<u64>> {
    exact_float(memory, raw).ok_or_else(|| type_error(memory, vm, message, raw))
}

/// Applies one unary Float operation, or returns the TypeError `message`.
pub(crate) fn float_unary(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    operation: fn(f64) -> f64,
    message: &str,
) -> Option<u64> {
    match exact_float(memory, receiver) {
        Some(value) => float_value(memory, vm, operation(value)),
        None => type_error(memory, vm, message, receiver),
    }
}

/// Applies one binary Float operation like v1's `float_pair`, whose messages append
/// " requires a Float receiver" and " requires a Float argument" to `name`.
pub(crate) fn float_pair(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    other: u64,
    operation: fn(f64, f64) -> f64,
    messages: [&str; 2],
) -> Option<u64> {
    let left = float_operand(memory, vm, receiver, messages[0]);
    let right = float_operand(memory, vm, other, messages[1]);
    match (left, right) {
        (Ok(left), Ok(right)) => float_value(memory, vm, operation(left, right)),
        (Err(error), _) | (_, Err(error)) => error,
    }
}

/// Evaluates `abs` for an Int or Float.
pub(crate) fn abs(memory: &mut impl Memory, vm: u32, receiver: u64) -> Option<u64> {
    if let Some(value) = exact_int(memory, receiver) {
        return match value.checked_abs() {
            Some(result) => int_value(memory, vm, result),
            None => recoverable(
                memory,
                vm,
                "IntOverflowError",
                "absolute value is outside the ExS signed 64-bit range",
                receiver,
            ),
        };
    }
    if exact_float(memory, receiver).is_some() {
        return float_unary(
            memory,
            vm,
            receiver,
            f64::abs,
            "abs requires a Float receiver",
        );
    }
    type_error(
        memory,
        vm,
        "abs requires an Int or Float receiver",
        receiver,
    )
}

/// Evaluates `signum`, `is_even` or `is_odd` of an Int.
pub(crate) fn int_unary(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    operation: fn(i64) -> u64,
    message: &str,
) -> Option<u64> {
    match exact_int(memory, receiver) {
        Some(value) => Some(operation(value)),
        None => type_error(memory, vm, message, receiver),
    }
}

/// Returns the immediate Int `-1`, `0` or `1` for the sign of `value`.
pub(crate) fn signum(value: i64) -> u64 {
    ValueRef::small_int(value.signum()).map_or(0, ValueRef::raw)
}

/// Evaluates `gcd` or `lcm` of two Ints.
pub(crate) fn gcd_or_lcm(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    other: u64,
    lcm: bool,
) -> Option<u64> {
    let name = if lcm { "lcm" } else { "gcd" };
    let receiver_message = if lcm {
        "lcm requires an Int receiver"
    } else {
        "gcd requires an Int receiver"
    };
    let argument_message = if lcm {
        "lcm requires an Int argument"
    } else {
        "gcd requires an Int argument"
    };
    let left = match int_operand(memory, vm, receiver, receiver_message) {
        Ok(value) => value.unsigned_abs(),
        Err(error) => return error,
    };
    let right = match int_operand(memory, vm, other, argument_message) {
        Ok(value) => value.unsigned_abs(),
        Err(error) => return error,
    };
    let value = if lcm {
        if left == 0 || right == 0 {
            return int_value(memory, vm, 0);
        }
        match (left / gcd(left, right)).checked_mul(right) {
            Some(value) => value,
            None => {
                return recoverable(
                    memory,
                    vm,
                    "IntOverflowError",
                    "lcm overflowed the ExS signed 64-bit range",
                    receiver,
                );
            }
        }
    } else {
        gcd(left, right)
    };
    match i64::try_from(value) {
        Ok(value) => int_value(memory, vm, value),
        Err(_) => recoverable(
            memory,
            vm,
            "IntOverflowError",
            if name == "lcm" {
                "lcm is outside the ExS signed 64-bit range"
            } else {
                "gcd is outside the ExS signed 64-bit range"
            },
            receiver,
        ),
    }
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

/// Evaluates `pow` for an Int or Float receiver.
pub(crate) fn pow(memory: &mut impl Memory, vm: u32, receiver: u64, exponent: u64) -> Option<u64> {
    if let Some(base) = exact_int(memory, receiver) {
        let exponent = match exact_int(memory, exponent) {
            Some(value) if value >= 0 => value.cast_unsigned(),
            Some(_) => {
                return recoverable(
                    memory,
                    vm,
                    "ValueError",
                    "Int pow requires a non-negative exponent",
                    exponent,
                );
            }
            None => return type_error(memory, vm, "Int pow requires an Int exponent", exponent),
        };
        return match checked_pow(base, exponent) {
            Some(value) => int_value(memory, vm, value),
            None => recoverable(
                memory,
                vm,
                "IntOverflowError",
                "Int pow overflowed the ExS signed 64-bit range",
                receiver,
            ),
        };
    }
    if exact_float(memory, receiver).is_some() {
        return float_pair(
            memory,
            vm,
            receiver,
            exponent,
            libm::pow,
            [
                "pow requires a Float receiver",
                "pow requires a Float argument",
            ],
        );
    }
    type_error(
        memory,
        vm,
        "pow requires an Int or Float receiver",
        receiver,
    )
}

/// Raises `base` to `exponent` by squaring, like v1, failing on any intermediate overflow.
fn checked_pow(base: i64, mut exponent: u64) -> Option<i64> {
    let (mut result, mut factor) = (1_i64, base);
    while exponent > 0 {
        if exponent & 1 == 1 {
            result = result.checked_mul(factor)?;
        }
        exponent >>= 1;
        if exponent > 0 {
            factor = factor.checked_mul(factor)?;
        }
    }
    Some(result)
}

/// Evaluates `min` or `max` for an Int or Float receiver.
pub(crate) fn min_or_max(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    other: u64,
    max: bool,
) -> Option<u64> {
    if exact_int(memory, receiver).is_some() {
        let messages = if max {
            [
                "max requires an Int receiver",
                "max requires an Int argument",
            ]
        } else {
            [
                "min requires an Int receiver",
                "min requires an Int argument",
            ]
        };
        let left = int_operand(memory, vm, receiver, messages[0]);
        let right = int_operand(memory, vm, other, messages[1]);
        return match (left, right) {
            (Ok(left), Ok(right)) => int_value(
                memory,
                vm,
                if max {
                    left.max(right)
                } else {
                    left.min(right)
                },
            ),
            (Err(error), _) | (_, Err(error)) => error,
        };
    }
    if exact_float(memory, receiver).is_some() {
        return if max {
            float_pair(
                memory,
                vm,
                receiver,
                other,
                libm::fmax,
                [
                    "max requires a Float receiver",
                    "max requires a Float argument",
                ],
            )
        } else {
            float_pair(
                memory,
                vm,
                receiver,
                other,
                libm::fmin,
                [
                    "min requires a Float receiver",
                    "min requires a Float argument",
                ],
            )
        };
    }
    let message = if max {
        "max requires an Int or Float receiver"
    } else {
        "min requires an Int or Float receiver"
    };
    type_error(memory, vm, message, receiver)
}

/// Evaluates `clamp` for an Int or Float receiver.
pub(crate) fn clamp(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    minimum: u64,
    maximum: u64,
) -> Option<u64> {
    let inverted = |memory: &mut _| {
        recoverable(
            memory,
            vm,
            "ValueError",
            "clamp minimum must not exceed maximum",
            receiver,
        )
    };
    if exact_int(memory, receiver).is_some() {
        let value = int_operand(memory, vm, receiver, "clamp requires an Int receiver");
        let low = int_operand(memory, vm, minimum, "clamp requires an Int minimum");
        let high = int_operand(memory, vm, maximum, "clamp requires an Int maximum");
        return match (value, low, high) {
            (Ok(value), Ok(low), Ok(high)) if low <= high => {
                int_value(memory, vm, value.clamp(low, high))
            }
            (Ok(_), Ok(_), Ok(_)) => inverted(memory),
            (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => error,
        };
    }
    if exact_float(memory, receiver).is_some() {
        let value = float_operand(memory, vm, receiver, "clamp requires a Float receiver");
        let low = float_operand(memory, vm, minimum, "clamp requires a Float minimum");
        let high = float_operand(memory, vm, maximum, "clamp requires a Float maximum");
        return match (value, low, high) {
            (Ok(value), Ok(low), Ok(high)) if low <= high => {
                float_value(memory, vm, libm::fmin(libm::fmax(value, low), high))
            }
            (Ok(_), Ok(_), Ok(_)) => inverted(memory),
            (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => error,
        };
    }
    type_error(
        memory,
        vm,
        "clamp requires an Int or Float receiver",
        receiver,
    )
}

/// Evaluates `to_float` of an Int.
pub(crate) fn to_float(memory: &mut impl Memory, vm: u32, receiver: u64) -> Option<u64> {
    match exact_int(memory, receiver) {
        Some(value) => float_value(memory, vm, value as f64),
        None => type_error(memory, vm, "to_float requires an Int receiver", receiver),
    }
}

/// Evaluates `to_int` of a finite Float inside the Int range, truncating toward zero.
pub(crate) fn to_int(memory: &mut impl Memory, vm: u32, receiver: u64) -> Option<u64> {
    let Some(value) = exact_float(memory, receiver) else {
        return type_error(memory, vm, "to_int requires a Float receiver", receiver);
    };
    let value = libm::trunc(value);
    if !value.is_finite() {
        return recoverable(
            memory,
            vm,
            "ValueError",
            "to_int requires a finite Float",
            receiver,
        );
    }
    if value < i64::MIN as f64 || value >= 9_223_372_036_854_775_808.0 {
        return recoverable(
            memory,
            vm,
            "IntOverflowError",
            "Float is outside the ExS Int range",
            receiver,
        );
    }
    int_value(memory, vm, value as i64)
}

/// Evaluates `is_nan`, `is_finite` or `is_infinite` of a Float.
pub(crate) fn float_predicate(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    predicate: fn(f64) -> bool,
    message: &str,
) -> Option<u64> {
    match exact_float(memory, receiver) {
        Some(value) => Some(boolean(predicate(value))),
        None => type_error(memory, vm, message, receiver),
    }
}

/// Evaluates `div_euclid` or `rem_euclid` of two Ints.
pub(crate) fn euclidean(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    divisor: u64,
    remainder: bool,
) -> Option<u64> {
    let (Some(left), Some(right)) = (exact_int(memory, receiver), exact_int(memory, divisor))
    else {
        return type_error(
            memory,
            vm,
            "Euclidean division requires Int operands",
            receiver,
        );
    };
    if right == 0 {
        return recoverable(
            memory,
            vm,
            "DivisionByZeroError",
            "Euclidean division requires a non-zero divisor",
            divisor,
        );
    }
    let result = if remainder {
        left.checked_rem_euclid(right)
    } else {
        left.checked_div_euclid(right)
    };
    match result {
        Some(value) => int_value(memory, vm, value),
        None => recoverable(
            memory,
            vm,
            "IntOverflowError",
            if remainder {
                "rem_euclid overflowed the ExS signed 64-bit range"
            } else {
                "div_euclid overflowed the ExS signed 64-bit range"
            },
            receiver,
        ),
    }
}
