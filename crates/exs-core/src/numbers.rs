//! Numeric values and the language's numeric operations.
//!
//! Bool counts as the Int 0 or 1, mixed Int and Float operands promote to Float, Int arithmetic
//! is checked in the signed 64-bit range and `/` always produces a Float. Ints inside the `i32`
//! range are immediates; other Ints and all Floats are heap objects.

use core::mem::{offset_of, size_of};

use exs_abi::{TYPE_FLOAT, TYPE_INT};
use exs_layout::{BigIntObject, FloatObject, ObjectKind};
use exs_value::layout::{INT_BOXED, type_kind};
use exs_value::{Immediate, ValueRef};

use crate::allocator::allocate;
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
    /// An operand is not a Bool, Int or Float.
    NotNumeric,
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
        Some(Immediate::Bool(flag)) => Some(Number::Int(i64::from(flag))),
        Some(Immediate::Int(number)) => Some(Number::Int(i64::from(number))),
        Some(Immediate::None) => None,
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
        number(memory, left).ok_or(NumericError::NotNumeric)?,
        number(memory, right).ok_or(NumericError::NotNumeric)?,
    ))
}

/// Converts one number to binary64.
const fn as_float(number: Number) -> f64 {
    match number {
        Number::Int(value) => value as f64,
        Number::Float(value) => value,
    }
}

/// Applies `+`, `-` or `*` to two numeric references.
///
/// # Errors
///
/// Returns why no value was produced.
pub fn arithmetic(
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

/// Divides two numeric references into a Float.
///
/// # Errors
///
/// Returns why no value was produced.
pub fn divide(
    memory: &mut impl Memory,
    vm: u32,
    left: u64,
    right: u64,
) -> Result<u64, NumericError> {
    let (left, right) = operands(memory, left, right)?;
    number_value(memory, vm, Number::Float(as_float(left) / as_float(right)))
}

/// Negates one numeric reference.
///
/// # Errors
///
/// Returns why no value was produced.
pub fn negate(memory: &mut impl Memory, vm: u32, value: u64) -> Result<u64, NumericError> {
    let result = match number(memory, value).ok_or(NumericError::NotNumeric)? {
        Number::Int(value) => Number::Int(value.checked_neg().ok_or(NumericError::Overflow)?),
        Number::Float(value) => Number::Float(-value),
    };
    number_value(memory, vm, result)
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
