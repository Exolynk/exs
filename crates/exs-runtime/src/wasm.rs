//! Rust implementations exported by the Wasm-target runtime.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::num::NonZeroU32;
use core::panic::PanicInfo;

use exs_abi::ExsValue;
use exs_value::{ValueRef, is_valid_int};

use crate::state::runtime;
use crate::{RtValue, RuntimeList, RuntimeObject, RuntimeString};

/// Allocates and returns the singular null value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_null_new() -> ValueRef {
    allocate(RtValue::Null)
}

/// Allocates a boolean value from its canonical Wasm representation.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_bool_new(value: i32) -> ValueRef {
    match value {
        0 => allocate(RtValue::Bool(false)),
        1 => allocate(RtValue::Bool(true)),
        _ => trap(),
    }
}

/// Allocates an ExS integer when it lies in the language range.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_int_new(value: i64) -> ValueRef {
    if !is_valid_int(value) {
        trap();
    }
    allocate(RtValue::Int(value))
}

/// Allocates an IEEE 754 binary64 floating-point value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_float_new(value: f64) -> ValueRef {
    allocate(RtValue::Float(value))
}

/// Adds two runtime numeric values.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_add(left: ValueRef, right: ValueRef) -> ValueRef {
    arithmetic(left, right, i64::checked_add, |left, right| left + right)
}

/// Subtracts two runtime numeric values.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_sub(left: ValueRef, right: ValueRef) -> ValueRef {
    arithmetic(left, right, i64::checked_sub, |left, right| left - right)
}

/// Multiplies two runtime numeric values.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_mul(left: ValueRef, right: ValueRef) -> ValueRef {
    arithmetic(left, right, i64::checked_mul, |left, right| left * right)
}

/// Negates one runtime numeric value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_neg(value: ValueRef) -> ValueRef {
    match number_of_ref(value) {
        Number::Int(value) => match value.checked_neg() {
            Some(value) if is_valid_int(value) => allocate(RtValue::Int(value)),
            _ => trap(),
        },
        Number::Float(value) => allocate(RtValue::Float(-value)),
    }
}

/// Tests two runtime values for equality.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_eq(left: ValueRef, right: ValueRef) -> ValueRef {
    let equal = match (value(left), value(right)) {
        (RtValue::Null, RtValue::Null) => true,
        (left, right) if is_numeric(left) && is_numeric(right) => {
            numbers_equal(number_of(left), number_of(right))
        }
        (RtValue::String(left), RtValue::String(right)) => left.as_str() == right.as_str(),
        (RtValue::List(_), RtValue::List(_)) => left == right,
        (RtValue::Object(_), RtValue::Object(_)) => left == right,
        _ => false,
    };
    allocate(RtValue::Bool(equal))
}

/// Tests two runtime values for inequality.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_ne(left: ValueRef, right: ValueRef) -> ValueRef {
    let equal = __exs_rt_eq(left, right);
    allocate(RtValue::Bool(!boolean(equal)))
}

/// Compares two runtime numeric values for less-than.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_lt(left: ValueRef, right: ValueRef) -> ValueRef {
    compare(left, right, Ordering::Less)
}

/// Compares two runtime numeric values for less-than-or-equal.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_le(left: ValueRef, right: ValueRef) -> ValueRef {
    compare(left, right, Ordering::LessOrEqual)
}

/// Compares two runtime numeric values for greater-than.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_gt(left: ValueRef, right: ValueRef) -> ValueRef {
    compare(left, right, Ordering::Greater)
}

/// Compares two runtime numeric values for greater-than-or-equal.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_ge(left: ValueRef, right: ValueRef) -> ValueRef {
    compare(left, right, Ordering::GreaterOrEqual)
}

/// Negates a runtime boolean value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_not(value: ValueRef) -> ValueRef {
    allocate(RtValue::Bool(!boolean(value)))
}

/// Converts a runtime boolean value to a Wasm condition.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_condition(value: ValueRef) -> i32 {
    i32::from(boolean(value))
}

/// Allocates a runtime-owned buffer for one compiler literal data segment.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_literal_buffer_alloc(length: i32) -> i32 {
    let Ok(length) = usize::try_from(length) else {
        trap();
    };
    let buffer = &mut unsafe { runtime() }.literal_buffer;
    buffer.clear();
    buffer.resize(length, 0);
    pointer(buffer.as_ptr())
}

/// Creates an immutable runtime string from the compiler-populated literal buffer.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_string_new(pointer: i32, length: i32) -> ValueRef {
    let Ok(length) = usize::try_from(length) else {
        trap();
    };
    let buffer = &unsafe { runtime() }.literal_buffer;
    if usize::try_from(pointer).ok() != Some(buffer.as_ptr() as usize) || length != buffer.len() {
        trap();
    }
    let value = RuntimeString::from_utf8(buffer).unwrap_or_else(|_| trap());
    allocate(RtValue::String(Box::new(value)))
}

/// Allocates an empty mutable runtime list.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_list_new() -> ValueRef {
    allocate(RtValue::List(Box::new(RuntimeList::new())))
}

/// Allocates an empty mutable runtime object.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_object_new() -> ValueRef {
    allocate(RtValue::Object(Box::new(RuntimeObject::new())))
}

/// Appends a value through the receiver's runtime collection dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_append(receiver: ValueRef, item: ValueRef) -> ValueRef {
    let length = match value_mut(receiver) {
        RtValue::List(list) => {
            list.elements.push(item);
            list.elements.len()
        }
        _ => trap(),
    };
    length_value(length)
}

/// Reads one element through the receiver's runtime indexing dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_index_get(receiver: ValueRef, index: ValueRef) -> ValueRef {
    match index_key(receiver, index) {
        IndexKey::List(index) => match value(receiver) {
            RtValue::List(list) => match list.elements.get(index) {
                Some(value) => *value,
                None => trap(),
            },
            _ => trap(),
        },
        IndexKey::Object(key) => {
            let result = match value(receiver) {
                RtValue::Object(object) => object
                    .entries
                    .iter()
                    .find_map(|(entry_key, value)| (entry_key.as_ref() == key).then_some(*value)),
                _ => trap(),
            };
            match result {
                Some(value) => value,
                None => allocate(RtValue::Null),
            }
        }
    }
}

/// Replaces one element through the receiver's runtime indexing dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_index_set(
    receiver: ValueRef,
    index: ValueRef,
    replacement: ValueRef,
) -> ValueRef {
    match index_key(receiver, index) {
        IndexKey::List(index) => match value_mut(receiver) {
            RtValue::List(list) => match list.elements.get_mut(index) {
                Some(value) => {
                    *value = replacement;
                    replacement
                }
                None => trap(),
            },
            _ => trap(),
        },
        IndexKey::Object(key) => match value_mut(receiver) {
            RtValue::Object(object) => {
                if let Some((_, value)) = object
                    .entries
                    .iter_mut()
                    .find(|(entry_key, _)| entry_key.as_ref() == key)
                {
                    *value = replacement;
                } else {
                    object.entries.push((key.into_boxed_str(), replacement));
                }
                replacement
            }
            _ => trap(),
        },
    }
}

/// Calls one statically named member method through runtime receiver dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_call_method(
    receiver: ValueRef,
    method: ValueRef,
    arguments: ValueRef,
) -> ValueRef {
    let method = string_value(method);
    match method.as_str() {
        "push" => __exs_rt_append(receiver, single_argument(arguments)),
        "has" => object_has(receiver, single_argument(arguments)),
        "delete" => object_delete(receiver, single_argument(arguments)),
        "keys" => {
            require_no_arguments(arguments);
            object_keys(receiver)
        }
        "values" => {
            require_no_arguments(arguments);
            object_values(receiver)
        }
        _ => trap(),
    }
}

/// Allocates a runtime-owned linear-memory buffer for one CBOR input value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_input_alloc(length: i32) -> i32 {
    let Ok(length) = usize::try_from(length) else {
        trap();
    };
    let buffer = &mut unsafe { runtime() }.input_buffer;
    buffer.clear();
    buffer.resize(length, 0);
    pointer(buffer.as_ptr())
}

/// Decodes the runner-provided CBOR input into one runtime value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_decode_input(pointer: i32, length: i32) -> ValueRef {
    let input = decode_input(pointer, length);
    exs_value_to_runtime(input)
}

/// Encodes a completed program result into the runtime-owned CBOR result buffer.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_set_result(value: ValueRef) {
    let result = runtime_to_exs_value(value);
    let encoded = result.to_cbor().unwrap_or_else(|_| trap());
    unsafe {
        runtime().result_buffer = encoded;
    }
}

/// Returns the linear-memory pointer of the CBOR result buffer.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_result_ptr() -> i32 {
    pointer(unsafe { runtime().result_buffer.as_ptr() })
}

/// Returns the length of the CBOR result buffer.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_result_len() -> i32 {
    let length = unsafe { runtime().result_buffer.len() };
    match i32::try_from(length) {
        Ok(length) => length,
        Err(_) => trap(),
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    trap()
}

#[derive(Clone, Copy)]
enum Number {
    Int(i64),
    Float(f64),
}

enum Ordering {
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

/// A validated runtime index representation selected from the receiver's value kind.
enum IndexKey {
    /// A zero-based list position.
    List(usize),
    /// An owned object property key.
    Object(String),
}

/// Appends one runtime value and returns its one-based table index.
fn allocate(value: RtValue) -> ValueRef {
    let state = unsafe { runtime() };
    let Some(next_index) = state.values.len().checked_add(1) else {
        trap();
    };
    let Ok(next_index) = u32::try_from(next_index) else {
        trap();
    };
    let Some(next_index) = NonZeroU32::new(next_index) else {
        trap();
    };
    state.values.push(value);
    unsafe { ValueRef::from_runtime_index(next_index) }
}

/// Returns the runtime payload stored at one value-table index.
fn value(reference: ValueRef) -> &'static RtValue {
    let index = reference.runtime_index() as usize - 1;
    match unsafe { runtime().values.get(index) } {
        Some(value) => value,
        None => trap(),
    }
}

/// Returns mutable access to the runtime payload stored at one value-table index.
fn value_mut(reference: ValueRef) -> &'static mut RtValue {
    let index = reference.runtime_index() as usize - 1;
    match unsafe { runtime().values.get_mut(index) } {
        Some(value) => value,
        None => trap(),
    }
}

/// Converts one runtime Phase-3 value into its host-safe ABI value.
fn runtime_to_exs_value(reference: ValueRef) -> ExsValue {
    let mut active_containers = Vec::new();
    runtime_to_exs_value_inner(reference, &mut active_containers)
}

/// Converts one runtime value into a host-safe value while rejecting CBOR-inexpressible cycles.
fn runtime_to_exs_value_inner(
    reference: ValueRef,
    active_containers: &mut Vec<ValueRef>,
) -> ExsValue {
    match value(reference) {
        RtValue::Null => ExsValue::Null,
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
        ExsValue::Null => RtValue::Null,
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
    allocate(value)
}

/// Decodes one runtime-owned input buffer after validating the runner-provided range.
fn decode_input(pointer: i32, length: i32) -> ExsValue {
    let Ok(length) = usize::try_from(length) else {
        trap();
    };
    let buffer = &unsafe { runtime() }.input_buffer;
    let expected_pointer = buffer.as_ptr() as usize;
    let Ok(pointer) = usize::try_from(pointer) else {
        trap();
    };
    if pointer != expected_pointer || length != buffer.len() {
        trap();
    }
    ExsValue::from_cbor(buffer).unwrap_or_else(|_| trap())
}

/// Returns whether a payload participates in numeric operations.
fn is_numeric(value: &RtValue) -> bool {
    matches!(
        value,
        RtValue::Bool(_) | RtValue::Int(_) | RtValue::Float(_)
    )
}

/// Converts one runtime numeric reference into the shared numeric dispatch form.
fn number_of_ref(reference: ValueRef) -> Number {
    number_of(value(reference))
}

/// Converts one runtime numeric payload into the shared numeric dispatch form.
fn number_of(value: &RtValue) -> Number {
    match value {
        RtValue::Bool(false) => Number::Int(0),
        RtValue::Bool(true) => Number::Int(1),
        RtValue::Int(value) => Number::Int(*value),
        RtValue::Float(value) => Number::Float(*value),
        RtValue::Null
        | RtValue::String(_)
        | RtValue::List(_)
        | RtValue::Object(_)
        | RtValue::BoxedFutureValue(_) => trap(),
    }
}

/// Performs a binary numeric operation with Float promotion.
fn arithmetic(
    left: ValueRef,
    right: ValueRef,
    integer_operation: fn(i64, i64) -> Option<i64>,
    float_operation: fn(f64, f64) -> f64,
) -> ValueRef {
    match (number_of_ref(left), number_of_ref(right)) {
        (Number::Int(left), Number::Int(right)) => match integer_operation(left, right) {
            Some(value) if is_valid_int(value) => allocate(RtValue::Int(value)),
            _ => trap(),
        },
        (left, right) => allocate(RtValue::Float(float_operation(
            as_float(left),
            as_float(right),
        ))),
    }
}

/// Compares two numeric values with Float promotion.
fn compare(left: ValueRef, right: ValueRef, ordering: Ordering) -> ValueRef {
    let result = match (number_of_ref(left), number_of_ref(right)) {
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
    allocate(RtValue::Bool(result))
}

/// Tests two numeric values for equality with Float promotion.
fn numbers_equal(left: Number, right: Number) -> bool {
    match (left, right) {
        (Number::Int(left), Number::Int(right)) => left == right,
        (left, right) => as_float(left) == as_float(right),
    }
}

/// Converts a numeric dispatch value into binary64.
fn as_float(value: Number) -> f64 {
    match value {
        Number::Int(value) => value as f64,
        Number::Float(value) => value,
    }
}

/// Reads a runtime value as a strict boolean.
fn boolean(reference: ValueRef) -> bool {
    match value(reference) {
        RtValue::Bool(result) => *result,
        _ => trap(),
    }
}

/// Reads one non-negative runtime integer as a list index.
fn list_index(reference: ValueRef) -> usize {
    match value(reference) {
        RtValue::Int(index) if *index >= 0 => match usize::try_from(*index) {
            Ok(index) => index,
            Err(_) => trap(),
        },
        _ => trap(),
    }
}

/// Returns a checked ExS integer containing a collection length.
fn length_value(length: usize) -> ValueRef {
    let Ok(length) = i64::try_from(length) else {
        trap();
    };
    if !is_valid_int(length) {
        trap();
    }
    allocate(RtValue::Int(length))
}

/// Reads the sole value in one runtime-provided argument list.
fn single_argument(arguments: ValueRef) -> ValueRef {
    match value(arguments) {
        RtValue::List(list) if list.elements.len() == 1 => list.elements[0],
        _ => trap(),
    }
}

/// Verifies that one runtime-provided argument list is empty.
fn require_no_arguments(arguments: ValueRef) {
    match value(arguments) {
        RtValue::List(list) if list.elements.is_empty() => {}
        _ => trap(),
    }
}

/// Classifies a dynamic index according to the runtime receiver value.
fn index_key(receiver: ValueRef, index: ValueRef) -> IndexKey {
    match value(receiver) {
        RtValue::List(_) => IndexKey::List(list_index(index)),
        RtValue::Object(_) => IndexKey::Object(string_value(index)),
        _ => trap(),
    }
}

/// Copies a runtime String value for use as an object key or method name.
fn string_value(reference: ValueRef) -> String {
    match value(reference) {
        RtValue::String(value) => value.as_str().into(),
        _ => trap(),
    }
}

/// Tests whether an Object contains one string key.
fn object_has(receiver: ValueRef, key: ValueRef) -> ValueRef {
    let key = string_value(key);
    let contains = match value(receiver) {
        RtValue::Object(object) => object
            .entries
            .iter()
            .any(|(entry_key, _)| entry_key.as_ref() == key),
        _ => trap(),
    };
    allocate(RtValue::Bool(contains))
}

/// Removes one object property and returns its previous value or Null.
fn object_delete(receiver: ValueRef, key: ValueRef) -> ValueRef {
    let key = string_value(key);
    let removed = match value_mut(receiver) {
        RtValue::Object(object) => object
            .entries
            .iter()
            .position(|(entry_key, _)| entry_key.as_ref() == key)
            .map(|index| object.entries.remove(index).1),
        _ => trap(),
    };
    match removed {
        Some(value) => value,
        None => allocate(RtValue::Null),
    }
}

/// Returns a new List containing object keys in insertion order.
fn object_keys(receiver: ValueRef) -> ValueRef {
    let keys = match value(receiver) {
        RtValue::Object(object) => object
            .entries
            .iter()
            .map(|(key, _)| String::from(key.as_ref()))
            .collect::<Vec<_>>(),
        _ => trap(),
    };
    let elements = keys
        .into_iter()
        .map(|key| allocate(RtValue::String(Box::new(RuntimeString::from_string(key)))))
        .collect();
    allocate(RtValue::List(Box::new(RuntimeList { elements })))
}

/// Returns a new shallow List containing object values in insertion order.
fn object_values(receiver: ValueRef) -> ValueRef {
    let elements = match value(receiver) {
        RtValue::Object(object) => object.entries.iter().map(|(_, value)| *value).collect(),
        _ => trap(),
    };
    allocate(RtValue::List(Box::new(RuntimeList { elements })))
}

/// Converts one linear-memory pointer to the signed Wasm ABI representation.
fn pointer(value: *const u8) -> i32 {
    match i32::try_from(value as usize) {
        Ok(value) => value,
        Err(_) => trap(),
    }
}

/// Stops Wasm execution after an unrecoverable Phase-1 runtime fault.
fn trap() -> ! {
    core::arch::wasm32::unreachable()
}
