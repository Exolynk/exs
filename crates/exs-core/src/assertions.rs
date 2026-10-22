//! The test assertions `assert` and `assert_eq`.
//!
//! Like v1, a failed assertion is a fatal AssertionFailed Error with the description as its
//! message, and a description that is no String or a condition that is no Bool is a fatal
//! TypeError.

use exs_layout::ValueRef;

use crate::errors::{error_from_strings, new_error};
use crate::lists::{allocate_list, list_reference, push_unchecked};
use crate::memory::Memory;
use crate::strings::{string_object, text_string};
use crate::vm::Vm;

/// Evaluates `assert(condition, description)`: None when `condition` is true, else a fatal
/// AssertionFailed Error with `condition` as its data.
///
/// Both values must be rooted.
pub fn assert(memory: &mut impl Memory, vm: u32, condition: u64, description: u64) -> Option<u64> {
    if string_object(description).is_none() {
        return fatal_type_error(
            memory,
            vm,
            "std::assert description requires a String value",
            description,
        );
    }
    if condition == ValueRef::TRUE.raw() {
        Some(ValueRef::NONE.raw())
    } else if condition == ValueRef::FALSE.raw() {
        assertion_failed(memory, vm, description, condition)
    } else {
        fatal_type_error(
            memory,
            vm,
            "std::assert condition requires a Bool value",
            condition,
        )
    }
}

/// Evaluates `assert_eq(actual, expected, description)` given the result `comparison` of
/// `actual == expected`: None when it is true, else a fatal AssertionFailed Error with the List
/// `[actual, expected]` as its data.
///
/// Every value must be rooted.
pub fn assert_eq(
    memory: &mut impl Memory,
    vm: u32,
    comparison: u64,
    actual: u64,
    expected: u64,
    description: u64,
) -> Option<u64> {
    if string_object(description).is_none() {
        return fatal_type_error(
            memory,
            vm,
            "std::assert_eq description requires a String value",
            description,
        );
    }
    if comparison == ValueRef::TRUE.raw() {
        return Some(ValueRef::NONE.raw());
    }
    if comparison != ValueRef::FALSE.raw() {
        return fatal_type_error(
            memory,
            vm,
            "std::assert_eq comparison requires a Bool value",
            comparison,
        );
    }
    let list = allocate_list(memory, vm, 2)?;
    push_unchecked(memory, list, actual);
    push_unchecked(memory, list, expected);
    assertion_failed(memory, vm, description, list_reference(list)?)
}

/// Creates the fatal AssertionFailed Error with the String `description` and `data`.
fn assertion_failed(memory: &mut impl Memory, vm: u32, description: u64, data: u64) -> Option<u64> {
    let state = Vm(vm);
    let root = state.push_root(memory, data)?;
    let created = (|| {
        let kind = text_string(memory, vm, "AssertionFailed")?;
        state.push_root(memory, kind)?;
        error_from_strings(memory, vm, true, kind, description, data)
    })();
    state.pop_roots(memory, root);
    created
}

/// Creates a fatal TypeError with `message` and `data`.
fn fatal_type_error(memory: &mut impl Memory, vm: u32, message: &str, data: u64) -> Option<u64> {
    new_error(memory, vm, true, "TypeError", message, data)
}
