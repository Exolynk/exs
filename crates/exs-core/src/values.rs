//! Equality and ordering across value kinds.

use exs_layout::ValueRef;

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

/// Applies the comparison `test` (0 `==`, 1 `!=`, 2 `<`, 3 `<=`, 4 `>`, 5 `>=`) to the
/// Ordering a `Compare` implementation returned, like v1.
///
/// Returns the Bool, or a TypeError for a value that is no Ordering and for `Unordered`
/// under an ordering test.
pub fn ordering_test(memory: &mut impl Memory, vm: u32, ordering: u64, test: u32) -> Option<u64> {
    let order = crate::enums::enum_object(memory, ordering)
        .filter(|object| {
            crate::enums::enum_type(memory, *object) == exs_layout::STANDARD_ORDERING_TYPE_ID
        })
        .and_then(|object| match crate::enums::variant(memory, object) {
            0 => Some(Order::Less),
            1 => Some(Order::Equal),
            2 => Some(Order::Greater),
            3 => Some(Order::Unordered),
            _ => None,
        });
    let Some(order) = order else {
        return crate::errors::recoverable(
            memory,
            vm,
            "TypeError",
            "Compare implementations must return an Ordering value",
            ordering,
        );
    };
    let result = match (order, test) {
        (Order::Equal, 0)
        | (Order::Unordered | Order::Less | Order::Greater, 1)
        | (Order::Less, 2 | 3)
        | (Order::Equal, 3 | 5)
        | (Order::Greater, 4 | 5) => Ok(true),
        (Order::Unordered | Order::Less | Order::Greater, 0)
        | (Order::Equal, 1 | 2 | 4)
        | (Order::Less, 4 | 5)
        | (Order::Greater, 2 | 3) => Ok(false),
        (Order::Unordered, 2..=5) => Err("ordering comparison requires comparable values"),
        _ => Err("invalid Ordering value or comparison operator"),
    };
    match result {
        Ok(result) => Some(exs_layout::ValueRef::boolean(result).raw()),
        Err(message) => crate::errors::recoverable(memory, vm, "TypeError", message, ordering),
    }
}
