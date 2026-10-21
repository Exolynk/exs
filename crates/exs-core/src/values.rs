//! Equality and ordering across value kinds.

use exs_value::ValueRef;

use crate::bytes::{bytes_object, octets};
use crate::enums::{Order, ordering};
use crate::errors::recoverable;
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

/// Orders two values like v1's built-in ordering: numbers numerically with Float promotion,
/// Strings byte-wise, equal values as Equal and everything else as Unordered.
pub(crate) fn order(memory: &impl Memory, left: u64, right: u64) -> Order {
    let ordering = if let (Some(left), Some(right)) = (number(memory, left), number(memory, right))
    {
        order_numbers(left, right)
    } else if let (Some(left), Some(right)) = (string_object(left), string_object(right)) {
        Some(compare_strings(memory, left, right))
    } else if equal(memory, left, right) {
        Some(core::cmp::Ordering::Equal)
    } else {
        None
    };
    match ordering {
        Some(core::cmp::Ordering::Less) => Order::Less,
        Some(core::cmp::Ordering::Equal) => Order::Equal,
        Some(core::cmp::Ordering::Greater) => Order::Greater,
        None => Order::Unordered,
    }
}

/// Applies one ordering comparison and returns its Bool, or v1's TypeError, whose data is the
/// `std::Ordering::Unordered` value, when the values have no order.
pub fn compare_value(
    memory: &mut impl Memory,
    vm: u32,
    operator: Ordering,
    left: u64,
    right: u64,
) -> Option<u64> {
    let result = match (order(memory, left, right), operator) {
        (Order::Unordered, _) => {
            let data = ordering(memory, vm, Order::Unordered)?;
            return recoverable(
                memory,
                vm,
                "TypeError",
                "ordering comparison requires comparable values",
                data,
            );
        }
        (Order::Less, Ordering::Less | Ordering::LessOrEqual)
        | (Order::Equal, Ordering::LessOrEqual | Ordering::GreaterOrEqual)
        | (Order::Greater, Ordering::Greater | Ordering::GreaterOrEqual) => true,
        _ => false,
    };
    Some(ValueRef::boolean(result).raw())
}

/// Evaluates the `compare` method: the `std::Ordering` of two values.
pub(crate) fn compare_method(
    memory: &mut impl Memory,
    vm: u32,
    left: u64,
    right: u64,
) -> Option<u64> {
    let order = order(memory, left, right);
    ordering(memory, vm, order)
}
