//! Runtime state access, CBOR conversion, and linear-memory buffers.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::num::NonZeroU32;

use exs_abi::{ExsError, ExsValue};
use exs_value::{ValueRef, is_valid_int};

use crate::gc;
use crate::state::{HeapSlot, runtime};
use crate::value::{RtValue, RuntimeError, RuntimeList, RuntimeObject, RuntimeString};

/// Appends one runtime value and returns its one-based table index.
pub(crate) fn allocate(value: RtValue) -> ValueRef {
    gc::collect();
    let state = unsafe { runtime() };
    if let Some(index) = state.free_slots.pop() {
        let index = index as usize;
        let Some(slot) = state.values.get_mut(index) else {
            trap();
        };
        if slot.is_some() {
            trap();
        }
        *slot = Some(HeapSlot::new(value));
        let Some(index) = u32::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .and_then(NonZeroU32::new)
        else {
            trap();
        };
        return unsafe { ValueRef::from_runtime_index(index) };
    }
    let Some(next_index) = state.values.len().checked_add(1) else {
        trap();
    };
    let Ok(next_index) = u32::try_from(next_index) else {
        trap();
    };
    let Some(next_index) = NonZeroU32::new(next_index) else {
        trap();
    };
    state.values.push(Some(HeapSlot::new(value)));
    unsafe { ValueRef::from_runtime_index(next_index) }
}

/// Allocates one recoverable language Error using the active source position.
pub(crate) fn recoverable_error(kind: &str, message: &str, data: ValueRef) -> ValueRef {
    let origin = unsafe { runtime() }.current_source_position;
    allocate(RtValue::Error(Box::new(RuntimeError::recoverable(
        kind, message, data, origin,
    ))))
}

/// Allocates one fatal language Error using the active source position.
pub(crate) fn fatal_error(kind: &str, message: &str, data: ValueRef) -> ValueRef {
    let origin = unsafe { runtime() }.current_source_position;
    allocate(RtValue::Error(Box::new(RuntimeError::fatal(
        kind, message, data, origin,
    ))))
}

/// Returns the runtime payload stored at one value-table index.
pub(crate) fn value(reference: ValueRef) -> &'static RtValue {
    let index = reference.runtime_index() as usize - 1;
    match unsafe { runtime().values.get(index).and_then(Option::as_ref) } {
        Some(value) => &value.value,
        None => trap(),
    }
}

/// Returns mutable access to the runtime payload stored at one value-table index.
pub(crate) fn value_mut(reference: ValueRef) -> &'static mut RtValue {
    let index = reference.runtime_index() as usize - 1;
    match unsafe { runtime().values.get_mut(index).and_then(Option::as_mut) } {
        Some(value) => &mut value.value,
        None => trap(),
    }
}

/// Allocates a runtime-owned buffer for one compiler literal data segment.
pub(crate) fn literal_buffer_alloc(length: i32) -> i32 {
    let Ok(length) = usize::try_from(length) else {
        trap();
    };
    let buffer = &mut unsafe { runtime() }.literal_buffer;
    buffer.clear();
    buffer.resize(length, 0);
    pointer(buffer.as_ptr())
}

/// Creates an immutable runtime string from the compiler-populated literal buffer.
pub(crate) fn string_new(pointer_value: i32, length: i32) -> ValueRef {
    let Ok(length) = usize::try_from(length) else {
        trap();
    };
    let buffer = &unsafe { runtime() }.literal_buffer;
    if usize::try_from(pointer_value).ok() != Some(buffer.as_ptr() as usize)
        || length != buffer.len()
    {
        trap();
    }
    let value = RuntimeString::from_utf8(buffer).unwrap_or_else(|_| trap());
    allocate(RtValue::String(Box::new(value)))
}

/// Allocates a runtime-owned linear-memory buffer for one CBOR input value.
pub(crate) fn input_alloc(length: i32) -> i32 {
    let Ok(length) = usize::try_from(length) else {
        trap();
    };
    let buffer = &mut unsafe { runtime() }.input_buffer;
    buffer.clear();
    buffer.resize(length, 0);
    pointer(buffer.as_ptr())
}

/// Decodes the runner-provided CBOR input into one runtime value.
pub(crate) fn decode_input_value(pointer_value: i32, length: i32) -> ValueRef {
    let checkpoint = gc::temporary_root_checkpoint();
    let value = exs_value_to_runtime(decode_input(pointer_value, length));
    gc::restore_temporary_roots(checkpoint);
    value
}

/// Returns the number of CBOR-decoded values supplied to the generated entry point.
pub(crate) fn input_argument_count(arguments: ValueRef) -> i32 {
    let RtValue::List(arguments) = value(arguments) else {
        trap();
    };
    match i32::try_from(arguments.elements.len()) {
        Ok(length) => length,
        Err(_) => trap(),
    }
}

/// Returns one supplied entry argument or allocates None for a missing position.
pub(crate) fn input_argument(arguments: ValueRef, index: i32) -> ValueRef {
    let Ok(index) = usize::try_from(index) else {
        trap();
    };
    let RtValue::List(arguments) = value(arguments) else {
        trap();
    };
    arguments
        .elements
        .get(index)
        .copied()
        .unwrap_or_else(|| allocate(RtValue::None))
}

/// Creates the fatal Error returned when the entry receives too many arguments.
pub(crate) fn input_arity_error(arguments: ValueRef) -> ValueRef {
    fatal_error(
        "ArityError",
        "main received more input values than declared parameters",
        arguments,
    )
}

/// Encodes a completed program result into the runtime-owned CBOR result buffer.
pub(crate) fn set_result(value: ValueRef) {
    let result = runtime_to_exs_value(value);
    let encoded = result.to_cbor().unwrap_or_else(|_| trap());
    unsafe {
        runtime().result_buffer = encoded;
    }
}

/// Returns the linear-memory pointer of the CBOR result buffer.
pub(crate) fn result_pointer() -> i32 {
    pointer(unsafe { runtime().result_buffer.as_ptr() })
}

/// Returns the length of the CBOR result buffer.
pub(crate) fn result_length() -> i32 {
    let length = unsafe { runtime().result_buffer.len() };
    match i32::try_from(length) {
        Ok(length) => length,
        Err(_) => trap(),
    }
}

/// Converts one runtime value into its host-safe ABI value.
fn runtime_to_exs_value(reference: ValueRef) -> ExsValue {
    let mut active_containers = Vec::new();
    runtime_to_exs_value_inner(reference, &mut active_containers)
}

/// Converts one runtime value while rejecting CBOR-inexpressible container cycles.
fn runtime_to_exs_value_inner(
    reference: ValueRef,
    active_containers: &mut Vec<ValueRef>,
) -> ExsValue {
    match value(reference) {
        RtValue::None => ExsValue::None,
        RtValue::Error(error) => ExsValue::Error(ExsError {
            severity: error.severity,
            kind: error.kind.as_ref().into(),
            message: error.message.as_ref().into(),
            data: Box::new(runtime_to_exs_value_inner(error.data, active_containers)),
            origin: error.origin,
            trace: error.trace.clone(),
            cause: error
                .cause
                .map(|cause| Box::new(runtime_to_exs_value_inner(cause, active_containers))),
        }),
        RtValue::Bool(value) => ExsValue::Bool(*value),
        RtValue::Int(value) => ExsValue::Int(*value),
        RtValue::Float(value) => ExsValue::Float(*value),
        RtValue::String(value) => ExsValue::String(value.as_str().into()),
        RtValue::List(list) => {
            if active_containers.contains(&reference) {
                trap();
            }
            active_containers.push(reference);
            let elements = list
                .elements
                .iter()
                .copied()
                .map(|element| runtime_to_exs_value_inner(element, active_containers))
                .collect();
            let _removed = active_containers.pop();
            ExsValue::List(elements)
        }
        RtValue::Object(object) => {
            if active_containers.contains(&reference) {
                trap();
            }
            active_containers.push(reference);
            let entries = object
                .entries
                .iter()
                .map(|(key, value)| {
                    (
                        key.as_ref().into(),
                        runtime_to_exs_value_inner(*value, active_containers),
                    )
                })
                .collect();
            let _removed = active_containers.pop();
            ExsValue::Object(entries)
        }
        RtValue::BoxedFutureValue(_) => trap(),
    }
}

/// Converts a host-safe ABI value into a runtime value table entry.
fn exs_value_to_runtime(value: ExsValue) -> ValueRef {
    let value = match value {
        ExsValue::None => RtValue::None,
        ExsValue::Error(error) => RtValue::Error(Box::new(RuntimeError {
            severity: error.severity,
            kind: error.kind.into_boxed_str(),
            message: error.message.into_boxed_str(),
            data: exs_value_to_runtime(*error.data),
            origin: error.origin,
            trace: error.trace,
            cause: error.cause.map(|cause| exs_value_to_runtime(*cause)),
        })),
        ExsValue::Bool(value) => RtValue::Bool(value),
        ExsValue::Int(value) if is_valid_int(value) => RtValue::Int(value),
        ExsValue::Int(_) => trap(),
        ExsValue::Float(value) => RtValue::Float(value),
        ExsValue::String(value) => RtValue::String(Box::new(RuntimeString::from_string(value))),
        ExsValue::List(elements) => RtValue::List(Box::new(RuntimeList {
            elements: elements.into_iter().map(exs_value_to_runtime).collect(),
        })),
        ExsValue::Object(entries) => RtValue::Object(Box::new(RuntimeObject {
            entries: entries
                .into_iter()
                .map(|(key, value)| (key.into_boxed_str(), exs_value_to_runtime(value)))
                .collect(),
        })),
    };
    let reference = allocate(value);
    gc::push_temporary_root(reference);
    reference
}

/// Decodes one runtime-owned input buffer after validating the runner-provided range.
fn decode_input(pointer_value: i32, length: i32) -> ExsValue {
    let Ok(length) = usize::try_from(length) else {
        trap();
    };
    let buffer = &unsafe { runtime() }.input_buffer;
    let expected_pointer = buffer.as_ptr() as usize;
    let Ok(pointer_value) = usize::try_from(pointer_value) else {
        trap();
    };
    if pointer_value != expected_pointer || length != buffer.len() {
        trap();
    }
    ExsValue::from_cbor(buffer).unwrap_or_else(|_| trap())
}

/// Converts one linear-memory pointer to the signed Wasm ABI representation.
fn pointer(value: *const u8) -> i32 {
    match i32::try_from(value as usize) {
        Ok(value) => value,
        Err(_) => trap(),
    }
}

/// Stops Wasm execution after an unrecoverable runtime fault.
pub(crate) fn trap() -> ! {
    core::arch::wasm32::unreachable()
}
