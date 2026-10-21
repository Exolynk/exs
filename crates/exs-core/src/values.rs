//! Equality and ordering across value kinds.

use crate::bytes::{bytes_object, octets};
use crate::memory::Memory;
use crate::numbers::{NumericError, Ordering, number, numbers_equal, order_numbers};
use crate::strings::{compare_strings, string_object, strings_equal};

/// Tests two references for the language's `==`.
///
/// Numbers compare by value with Float promotion, Strings by text and Bytes by octets; any
/// other pair is equal only when both raw references are identical.
#[must_use]
pub fn equal(memory: &impl Memory, left: u64, right: u64) -> bool {
    if let (Some(left), Some(right)) = (number(memory, left), number(memory, right)) {
        return numbers_equal(left, right);
    }
    if let (Some(left), Some(right)) = (string_object(left), string_object(right)) {
        return strings_equal(memory, left, right);
    }
    if let (Some(left), Some(right)) = (bytes_object(left), bytes_object(right)) {
        return octets(memory, left) == octets(memory, right);
    }
    left == right
}

/// Applies one ordering comparison to two references.
///
/// Numbers order numerically and Strings byte-wise. Values of other kinds only satisfy `<=`
/// and `>=` when they are equal.
///
/// # Errors
///
/// Returns [`NumericError::Unordered`] when the values have no order, such as NaN or a String
/// against a number.
pub fn compare(
    memory: &impl Memory,
    operator: Ordering,
    left: u64,
    right: u64,
) -> Result<bool, NumericError> {
    let ordering = if let (Some(left), Some(right)) = (number(memory, left), number(memory, right))
    {
        order_numbers(left, right)
    } else if let (Some(left), Some(right)) = (string_object(left), string_object(right)) {
        Some(compare_strings(memory, left, right))
    } else if equal(memory, left, right) {
        Some(core::cmp::Ordering::Equal)
    } else {
        None
    }
    .ok_or(NumericError::Unordered)?;
    Ok(match operator {
        Ordering::Less => ordering.is_lt(),
        Ordering::LessOrEqual => ordering.is_le(),
        Ordering::Greater => ordering.is_gt(),
        Ordering::GreaterOrEqual => ordering.is_ge(),
    })
}
