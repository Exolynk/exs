//! Numeric values and the language's numeric operations.
//!
//! Bool counts as the Int 0 or 1, mixed Int and Float operands promote to Float, Int arithmetic
//! is checked in the signed 64-bit range and `/` always produces a Float. Ints inside the `i32`
//! range are immediates; other Ints and all Floats are heap objects.

use core::mem::{offset_of, size_of};

use exs_layout::value::layout::{INT_BOXED, type_kind};
use exs_layout::{BigIntObject, FloatObject, ObjectKind};
use exs_layout::{Immediate, ValueRef};
use exs_layout::{TYPE_FLOAT, TYPE_INT};

use crate::allocator::allocate;
use crate::errors::recoverable;
use crate::memory::Memory;

/// A numeric operand after Bool-to-Int conversion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Number {
    /// An exact signed 64-bit Int.
    Int(i64),
    /// An IEEE 754 binary64 number.
    Float(f64),
}

/// Why a numeric operation produced no value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NumericError {
    /// This operand is not an Int or Float.
    NotNumeric(u64),
    /// Int arithmetic left the signed 64-bit range.
    Overflow,
    /// Two Floats have no order, because one is NaN.
    Unordered,
    /// The heap could not hold the result.
    OutOfMemory,
}

/// A binary arithmetic operator with Float promotion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Arithmetic {
    /// `+`
    Add,
    /// `-`
    Subtract,
    /// `*`
    Multiply,
}

/// An ordering comparison operator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ordering {
    /// `<`
    Less,
    /// `<=`
    LessOrEqual,
    /// `>`
    Greater,
    /// `>=`
    GreaterOrEqual,
}

/// Byte offset of [`BigIntObject::value`].
const BIG_INT_VALUE: u32 = offset_of!(BigIntObject, value) as u32;
/// Byte offset of [`FloatObject::value`].
const FLOAT_VALUE: u32 = offset_of!(FloatObject, value) as u32;

/// Decodes one raw reference, or `None` for an empty slot.
pub(crate) fn reference(raw: u64) -> Option<ValueRef> {
    // SAFETY: the reference is only decoded to inspect its bits and payload.
    unsafe { ValueRef::from_raw(raw) }
}

/// Returns the numeric value of one raw reference, or `None` for non-numeric values.
#[must_use]
pub fn number(memory: &impl Memory, raw: u64) -> Option<Number> {
    let value = reference(raw)?;
    match value.immediate() {
        Some(Immediate::Int(number)) => Some(Number::Int(i64::from(number))),
        Some(Immediate::Bool(_) | Immediate::None | Immediate::Unit) => None,
        None if value.kind() == type_kind(TYPE_INT) && value.extra() & INT_BOXED != 0 => {
            Some(Number::Int(
                memory
                    .load_u64(value.payload() + BIG_INT_VALUE)
                    .cast_signed(),
            ))
        }
        None if value.kind() == type_kind(TYPE_FLOAT) => Some(Number::Float(f64::from_bits(
            memory.load_u64(value.payload() + FLOAT_VALUE),
        ))),
        None => None,
    }
}

/// Returns the raw reference of one Int, allocating it when it leaves the `i32` range.
pub fn int_value(memory: &mut impl Memory, vm: u32, value: i64) -> Option<u64> {
    if let Some(immediate) = ValueRef::small_int(value) {
        return Some(immediate.raw());
    }
    let object = allocate(
        memory,
        vm,
        size_of::<BigIntObject>() as u32,
        ObjectKind::BigInt,
    )?;
    memory.store_u64(object + BIG_INT_VALUE, value.cast_unsigned());
    ValueRef::from_parts(type_kind(TYPE_INT), INT_BOXED, object).map(ValueRef::raw)
}

/// Returns the raw reference of one newly allocated Float.
pub fn float_value(memory: &mut impl Memory, vm: u32, value: f64) -> Option<u64> {
    let object = allocate(
        memory,
        vm,
        size_of::<FloatObject>() as u32,
        ObjectKind::Float,
    )?;
    memory.store_u64(object + FLOAT_VALUE, value.to_bits());
    ValueRef::from_parts(type_kind(TYPE_FLOAT), 0, object).map(ValueRef::raw)
}

/// Returns the raw reference of one numeric result.
fn number_value(memory: &mut impl Memory, vm: u32, number: Number) -> Result<u64, NumericError> {
    match number {
        Number::Int(value) => int_value(memory, vm, value),
        Number::Float(value) => float_value(memory, vm, value),
    }
    .ok_or(NumericError::OutOfMemory)
}

/// Returns both operands as numbers.
fn operands(memory: &impl Memory, left: u64, right: u64) -> Result<(Number, Number), NumericError> {
    Ok((
        number(memory, left).ok_or(NumericError::NotNumeric(left))?,
        number(memory, right).ok_or(NumericError::NotNumeric(right))?,
    ))
}

/// Converts one number to binary64.
const fn as_float(number: Number) -> f64 {
    match number {
        Number::Int(value) => value as f64,
        Number::Float(value) => value,
    }
}

/// Message of the TypeError for a non-numeric operand.
const NOT_NUMERIC: &str = "numeric operations require an Int or Float value";

/// Converts the outcome of a numeric operation into a value, turning failures into the
/// language's Errors. `overflow` is the IntOverflowError message and `left` its data.
fn numeric_value(
    memory: &mut impl Memory,
    vm: u32,
    outcome: Result<u64, NumericError>,
    overflow: &str,
    left: u64,
) -> Option<u64> {
    match outcome {
        Ok(value) => Some(value),
        Err(NumericError::NotNumeric(operand)) => {
            recoverable(memory, vm, "TypeError", NOT_NUMERIC, operand)
        }
        Err(NumericError::Overflow) => recoverable(memory, vm, "IntOverflowError", overflow, left),
        Err(NumericError::Unordered | NumericError::OutOfMemory) => None,
    }
}

/// Applies `+`, `-` or `*` to two numeric references; failures are TypeError and
/// IntOverflowError values. Returns `None` when memory is exhausted.
pub fn arithmetic(
    memory: &mut impl Memory,
    vm: u32,
    operator: Arithmetic,
    left: u64,
    right: u64,
) -> Option<u64> {
    let outcome = arithmetic_result(memory, vm, operator, left, right);
    numeric_value(
        memory,
        vm,
        outcome,
        "integer arithmetic overflowed the ExS signed 64-bit range",
        left,
    )
}

/// Applies `+`, `-` or `*` to two numeric references.
///
/// # Errors
///
/// Returns why no value was produced.
fn arithmetic_result(
    memory: &mut impl Memory,
    vm: u32,
    operator: Arithmetic,
    left: u64,
    right: u64,
) -> Result<u64, NumericError> {
    let result = match operands(memory, left, right)? {
        (Number::Int(left), Number::Int(right)) => Number::Int(
            match operator {
                Arithmetic::Add => left.checked_add(right),
                Arithmetic::Subtract => left.checked_sub(right),
                Arithmetic::Multiply => left.checked_mul(right),
            }
            .ok_or(NumericError::Overflow)?,
        ),
        (left, right) => {
            let (left, right) = (as_float(left), as_float(right));
            Number::Float(match operator {
                Arithmetic::Add => left + right,
                Arithmetic::Subtract => left - right,
                Arithmetic::Multiply => left * right,
            })
        }
    };
    number_value(memory, vm, result)
}

/// Divides two numeric references into a Float; a non-numeric operand is a TypeError value.
/// Returns `None` when memory is exhausted.
pub fn divide(memory: &mut impl Memory, vm: u32, left: u64, right: u64) -> Option<u64> {
    let outcome = operands(memory, left, right).and_then(|(left, right)| {
        number_value(memory, vm, Number::Float(as_float(left) / as_float(right)))
    });
    numeric_value(memory, vm, outcome, "", left)
}

/// Evaluates `left % right`: the Euclidean remainder, which is never negative. Ints follow
/// `rem_euclid`; with a Float operand both operands are Floats and a zero divisor gives NaN.
/// Returns `None` when memory is exhausted.
pub fn remainder(memory: &mut impl Memory, vm: u32, left: u64, right: u64) -> Option<u64> {
    match operands(memory, left, right) {
        Ok((Number::Int(_), Number::Int(_))) => {
            crate::methods::euclidean(memory, vm, left, right, true)
        }
        Ok((dividend, divisor)) => {
            let (dividend, divisor) = (as_float(dividend), as_float(divisor));
            let mut result = libm::fmod(dividend, divisor);
            if result < 0.0 {
                result += divisor.abs();
            }
            let outcome = number_value(memory, vm, Number::Float(result));
            numeric_value(memory, vm, outcome, "", left)
        }
        Err(error) => numeric_value(memory, vm, Err(error), "", left),
    }
}

/// Negates one numeric reference; failures are TypeError and IntOverflowError values.
/// Returns `None` when memory is exhausted.
pub fn negate(memory: &mut impl Memory, vm: u32, value: u64) -> Option<u64> {
    let outcome = number(memory, value)
        .ok_or(NumericError::NotNumeric(value))
        .and_then(|number| match number {
            Number::Int(number) => number
                .checked_neg()
                .ok_or(NumericError::Overflow)
                .map(Number::Int),
            Number::Float(number) => Ok(Number::Float(-number)),
        })
        .and_then(|number| number_value(memory, vm, number));
    numeric_value(
        memory,
        vm,
        outcome,
        "integer negation overflowed the ExS signed 64-bit range",
        value,
    )
}

/// Orders two numbers, `None` when NaN takes part.
pub(crate) fn order_numbers(left: Number, right: Number) -> Option<core::cmp::Ordering> {
    match (left, right) {
        (Number::Int(left), Number::Int(right)) => Some(left.cmp(&right)),
        (left, right) => as_float(left).partial_cmp(&as_float(right)),
    }
}

/// Tests two numbers for equality with Float promotion.
pub(crate) fn numbers_equal(left: Number, right: Number) -> bool {
    match (left, right) {
        (Number::Int(left), Number::Int(right)) => left == right,
        (left, right) => as_float(left) == as_float(right),
    }
}
