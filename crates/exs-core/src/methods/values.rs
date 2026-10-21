//! Methods shared by several value kinds: arithmetic aliases, rendering, sizes and Errors.

use crate::errors::{error_object, error_parts};
use crate::lists::{len as list_len, list_object};
use crate::memory::Memory;
use crate::methods::numbers::{boolean, type_error};
use crate::methods::objects::object_len;
use crate::numbers::{Arithmetic, arithmetic, divide, int_value};
use crate::strings::{char_count, concat, format, string_object};
use crate::vm::Vm;

/// Evaluates the `add`, `sub`, `mul` or `div` method exactly like the operator.
///
/// Both values must be rooted.
pub(crate) fn operator(
    memory: &mut impl Memory,
    vm: u32,
    operator: Option<Arithmetic>,
    receiver: u64,
    other: u64,
) -> Option<u64> {
    match operator {
        Some(Arithmetic::Add) => with_slots(memory, vm, &[receiver, other], |memory, slots| {
            concat(memory, vm, slots, 2, 0)
        }),
        Some(operator) => arithmetic(memory, vm, operator, receiver, other),
        None => divide(memory, vm, receiver, other),
    }
}

/// Runs `body` with `values` in consecutive temporary root slots.
fn with_slots<M: Memory>(
    memory: &mut M,
    vm: u32,
    values: &[u64],
    body: impl FnOnce(&mut M, u32) -> Option<u64>,
) -> Option<u64> {
    let state = Vm(vm);
    let first = state.root_stack_end(memory);
    for value in values {
        if state.push_root(memory, *value).is_none() {
            state.pop_roots(memory, first);
            return None;
        }
    }
    let result = body(memory, first);
    state.pop_roots(memory, first);
    result
}

/// Evaluates `to_string` and `debug`: v1's default text of any value as a String.
pub(crate) fn render(memory: &mut impl Memory, vm: u32, receiver: u64) -> Option<u64> {
    if string_object(receiver).is_some() {
        // Strings are immutable, so the receiver is its own rendering.
        return Some(receiver);
    }
    with_slots(memory, vm, &[receiver], |memory, slots| {
        format(memory, vm, slots, 1)
    })
}

/// Returns the visible length of a String, List or Object.
fn visible_len(memory: &impl Memory, vm: u32, receiver: u64) -> Option<u32> {
    if let Some(object) = string_object(receiver) {
        return Some(char_count(memory, object));
    }
    if let Some(list) = list_object(receiver) {
        return Some(list_len(memory, list));
    }
    if let Some(object) = crate::bytes::bytes_object(receiver) {
        return Some(crate::bytes::bytes_len(memory, object));
    }
    object_len(memory, vm, receiver)
}

/// Evaluates `length`.
pub(crate) fn length(memory: &mut impl Memory, vm: u32, receiver: u64) -> Option<u64> {
    match visible_len(memory, vm, receiver) {
        Some(len) => int_value(memory, vm, i64::from(len)),
        None => type_error(
            memory,
            vm,
            "length requires a Bytes, String, List, or Object receiver",
            receiver,
        ),
    }
}

/// Evaluates `is_empty`.
pub(crate) fn is_empty(memory: &mut impl Memory, vm: u32, receiver: u64) -> Option<u64> {
    match visible_len(memory, vm, receiver) {
        Some(len) => Some(boolean(len == 0)),
        None => type_error(
            memory,
            vm,
            "is_empty requires a Bytes, String, List, or Object receiver",
            receiver,
        ),
    }
}

/// The parts of an Error that methods expose.
#[derive(Clone, Copy)]
pub(crate) enum ErrorPart {
    /// `kind()`, a copy of the kind String.
    Kind,
    /// `message()`, a copy of the message String.
    Message,
    /// `data()`.
    Data,
    /// `cause()`, None when there is none.
    Cause,
}

/// Evaluates `kind`, `message`, `data` or `cause` of an Error.
pub(crate) fn error_part(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    part: ErrorPart,
) -> Option<u64> {
    let Some(object) = error_object(receiver) else {
        let message = match part {
            ErrorPart::Kind => "kind requires an Error receiver",
            ErrorPart::Message => "message requires an Error receiver",
            ErrorPart::Data => "data requires an Error receiver",
            ErrorPart::Cause => "cause requires an Error receiver",
        };
        return type_error(memory, vm, message, receiver);
    };
    let parts = error_parts(memory, object);
    Some(match part {
        ErrorPart::Kind => parts.kind,
        ErrorPart::Message => parts.message,
        ErrorPart::Data => parts.data,
        ErrorPart::Cause => parts.cause,
    })
}
