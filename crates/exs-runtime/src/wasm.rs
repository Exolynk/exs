//! Stable Wasm ABI exports for the ExS runtime.

use alloc::boxed::Box;
use alloc::string::String;
use core::panic::PanicInfo;

use exs_abi::{
    TYPE_ANY, TYPE_BOOL, TYPE_ERROR, TYPE_FLOAT, TYPE_FN, TYPE_INT, TYPE_LIST, TYPE_NONE,
    TYPE_OBJECT, TYPE_STRING,
};
use exs_value::{ValueRef, is_valid_int};

use crate::gc;
use crate::runtime;
use crate::value::{self, RtValue};

/// Allocates and returns the singular None value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_none_new() -> ValueRef {
    runtime::allocate(RtValue::None)
}

/// Starts one fresh root execution scheduler context.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_execution_start() {
    runtime::execution_start();
}

/// Consumes one generated scheduler checkpoint for the current task.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_scheduler_checkpoint() {
    runtime::scheduler_checkpoint();
}

/// Cancels every live task in the active root scheduler context.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_execution_cancel() {
    runtime::execution_cancel();
}

/// Returns whether one runtime value is a language Error.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_is_error(value: ValueRef) -> ValueRef {
    runtime::allocate(RtValue::Bool(matches!(
        runtime::value(value),
        RtValue::Error(_)
    )))
}

/// Returns whether one runtime value is a fatal language Error.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_is_fatal_error(value: ValueRef) -> i32 {
    i32::from(runtime::is_fatal_error(value))
}

/// Returns whether one runtime value is a closure.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_is_closure(value: ValueRef) -> i32 {
    i32::from(runtime::is_closure(value))
}

/// Returns whether one runtime value is a List.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_is_list(value: ValueRef) -> i32 {
    i32::from(runtime::is_list(value))
}

/// Creates the recoverable Error returned when source code calls a non-closure value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_not_callable_error(value: ValueRef) -> ValueRef {
    runtime::not_callable_error(value)
}

/// Creates the recoverable Error returned when dynamic `par` receives a non-List value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_parallel_list_error(value: ValueRef) -> ValueRef {
    runtime::parallel_list_error(value)
}

/// Tests one function-boundary value against a compiler-emitted type mask.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_type_matches(value: ValueRef, allowed_types: i32) -> i32 {
    let Ok(allowed_types) = u32::try_from(allowed_types) else {
        runtime::trap();
    };
    if allowed_types & !TYPE_ANY != 0 {
        runtime::trap();
    }
    i32::from(value_type_mask(runtime::value(value)) & allowed_types != 0)
}

/// Returns whether a value is an Object carrying one compiler-owned nominal type tag.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_object_is_type(value: ValueRef, type_id: i32) -> i32 {
    let Ok(type_id) = u32::try_from(type_id) else {
        runtime::trap();
    };
    i32::from(value::object::operations::has_type(value, type_id))
}

/// Returns whether one enum value carries the requested stable type identity.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_enum_is_type(value: ValueRef, type_identity: ValueRef) -> i32 {
    i32::from(runtime::enum_has_type(value, type_identity))
}

/// Returns whether an enum value selects one stable identity and variant name.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_enum_matches(
    value: ValueRef,
    type_identity: ValueRef,
    variant: ValueRef,
) -> ValueRef {
    runtime::allocate(RtValue::Bool(runtime::enum_matches(
        value,
        type_identity,
        variant,
    )))
}

/// Returns one enum payload field by zero-based declaration index.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_enum_field(value: ValueRef, index: i32) -> ValueRef {
    let Ok(index) = usize::try_from(index) else {
        return runtime::recoverable_error("MatchError", "enum payload index is invalid", value);
    };
    runtime::enum_field(value, index)
}

/// Returns the recoverable Error produced when no match arm accepts a value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_match_error(value: ValueRef) -> ValueRef {
    runtime::recoverable_error("MatchError", "no match arm accepted the value", value)
}

/// Creates a type-contract Error after one failed function-boundary type check.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_type_mismatch(value: ValueRef, error_allowed: i32) -> ValueRef {
    if error_allowed == 1 {
        runtime::recoverable_error(
            "TypeError",
            "value does not satisfy the declared function type",
            value,
        )
    } else if error_allowed == 0 {
        runtime::fatal_error(
            "TypeError",
            "value does not satisfy the declared function type",
            value,
        )
    } else {
        runtime::trap()
    }
}

/// Creates a recoverable language Error from String kind and message values.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_error_new(
    kind: ValueRef,
    message: ValueRef,
    data: ValueRef,
) -> ValueRef {
    let kind = match runtime::value(kind) {
        RtValue::String(value) => String::from(value.as_str()),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "error kind requires a String value",
                kind,
            );
        }
    };
    let message = match runtime::value(message) {
        RtValue::String(value) => String::from(value.as_str()),
        _ => {
            return runtime::recoverable_error(
                "TypeError",
                "error message requires a String value",
                message,
            );
        }
    };
    runtime::recoverable_error(&kind, &message, data)
}

/// Returns the compiler-visible type-mask bit for one runtime value.
fn value_type_mask(value: &RtValue) -> u32 {
    match value {
        RtValue::None => TYPE_NONE,
        RtValue::Error(_) => TYPE_ERROR,
        RtValue::Bool(_) => TYPE_BOOL,
        RtValue::Int(_) => TYPE_INT,
        RtValue::Float(_) => TYPE_FLOAT,
        RtValue::String(_) => TYPE_STRING,
        RtValue::List(_) => TYPE_LIST,
        RtValue::Object(_) => TYPE_OBJECT,
        RtValue::Closure(_) => TYPE_FN,
        RtValue::Cell(_) => 0,
        RtValue::BoxedFutureValue(_) => 0,
    }
}

/// Converts None into an Error while preserving direct values and Error values.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_propagate(value: ValueRef) -> ValueRef {
    match runtime::value(value) {
        RtValue::Error(_) => value,
        RtValue::None => {
            runtime::recoverable_error("MissingValue", "cannot propagate a missing value", value)
        }
        _ => value,
    }
}

/// Sets the source position attached to newly created runtime Errors.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_set_source_position(position: i32) {
    let Ok(position) = u32::try_from(position) else {
        runtime::trap();
    };
    unsafe { crate::state::runtime() }.current_source_position =
        Some(exs_abi::SourcePositionId(position));
}

/// Records the source call site consumed by the next generated function entry.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_set_call_site(position: i32) {
    let Ok(position) = u32::try_from(position) else {
        runtime::trap();
    };
    unsafe { crate::state::runtime() }.pending_call_site =
        Some(exs_abi::SourcePositionId(position));
}

/// Registers one generated direct function invocation.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_frame_push(function_id: i32) {
    let Ok(function_id) = u32::try_from(function_id) else {
        runtime::trap();
    };
    let state = unsafe { crate::state::runtime() };
    let call_site = state
        .pending_call_site
        .take()
        .unwrap_or(exs_abi::SourcePositionId(0));
    state.frames.push(exs_abi::ExsStackFrame {
        function_id,
        call_site,
    });
}

/// Removes the innermost generated direct function invocation.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_frame_pop() {
    if unsafe { crate::state::runtime() }.frames.pop().is_none() {
        runtime::trap();
    }
}

/// Allocates a Boolean value from its canonical Wasm representation.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_bool_new(value: i32) -> ValueRef {
    match value {
        0 => runtime::allocate(RtValue::Bool(false)),
        1 => runtime::allocate(RtValue::Bool(true)),
        _ => runtime::trap(),
    }
}

/// Allocates an ExS integer when it lies in the language range.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_int_new(value: i64) -> ValueRef {
    if !is_valid_int(value) {
        runtime::trap();
    }
    runtime::allocate(RtValue::Int(value))
}

/// Allocates an IEEE 754 binary64 floating-point value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_float_new(value: f64) -> ValueRef {
    runtime::allocate(RtValue::Float(value))
}

/// Adds runtime numeric values or produces a new shallow List.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_add(left: ValueRef, right: ValueRef) -> ValueRef {
    value::operations::add(left, right)
}

/// Subtracts two runtime numeric values.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_sub(left: ValueRef, right: ValueRef) -> ValueRef {
    value::operations::subtract(left, right)
}

/// Multiplies two runtime numeric values.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_mul(left: ValueRef, right: ValueRef) -> ValueRef {
    value::operations::multiply(left, right)
}

/// Divides two runtime numeric values and returns a Float.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_div(left: ValueRef, right: ValueRef) -> ValueRef {
    value::operations::divide(left, right)
}

/// Negates one runtime numeric value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_neg(value: ValueRef) -> ValueRef {
    value::numeric::negate(value)
}

/// Tests two runtime values for equality.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_eq(left: ValueRef, right: ValueRef) -> ValueRef {
    value::operations::equal(left, right)
}

/// Tests two runtime values for inequality.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_ne(left: ValueRef, right: ValueRef) -> ValueRef {
    value::operations::not_equal(left, right)
}

/// Compares two runtime values and returns the compiler-owned Ordering enum.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_compare(left: ValueRef, right: ValueRef) -> ValueRef {
    value::operations::compare(left, right)
}

/// Interprets one Ordering value for a compiler-selected source comparison operator.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_ordering_test(ordering: ValueRef, test: i32) -> ValueRef {
    value::operations::ordering_test(ordering, test)
}

/// Compares two runtime numeric values for less-than.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_lt(left: ValueRef, right: ValueRef) -> ValueRef {
    value::numeric::compare(left, right, value::numeric::Ordering::Less)
}

/// Compares two runtime numeric values for less-than-or-equal.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_le(left: ValueRef, right: ValueRef) -> ValueRef {
    value::numeric::compare(left, right, value::numeric::Ordering::LessOrEqual)
}

/// Compares two runtime numeric values for greater-than.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_gt(left: ValueRef, right: ValueRef) -> ValueRef {
    value::numeric::compare(left, right, value::numeric::Ordering::Greater)
}

/// Compares two runtime numeric values for greater-than-or-equal.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_ge(left: ValueRef, right: ValueRef) -> ValueRef {
    value::numeric::compare(left, right, value::numeric::Ordering::GreaterOrEqual)
}

/// Negates a runtime Boolean value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_not(value: ValueRef) -> ValueRef {
    value::numeric::not(value)
}

/// Validates one source value as a Boolean condition or returns a language Error.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_condition_value(value: ValueRef) -> ValueRef {
    value::numeric::condition_value(value)
}

/// Converts a compiler-validated runtime Boolean value to a Wasm condition.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_condition(value: ValueRef) -> i32 {
    value::numeric::condition(value)
}

/// Creates one compiler-generated root frame with the requested local slot count.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_root_push(slot_count: i32) -> i32 {
    gc::push_root_frame(slot_count)
}

/// Stores one compiler-local value in the current root frame.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_root_set(frame: i32, slot: i32, value: ValueRef) {
    gc::set_root_frame_slot(frame, slot, value);
}

/// Clears one compiler-local value from the current root frame.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_root_clear(frame: i32, slot: i32) {
    gc::clear_root_frame_slot(frame, slot);
}

/// Removes the current compiler-generated root frame.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_root_pop(frame: i32) {
    gc::pop_root_frame(frame);
}

/// Immediately performs one stop-the-world mark-and-sweep collection.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_gc_collect() {
    gc::collect();
}

/// Allocates a runtime-owned buffer for one compiler literal data segment.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_literal_buffer_alloc(length: i32) -> i32 {
    runtime::literal_buffer_alloc(length)
}

/// Creates an immutable runtime string from the compiler-populated literal buffer.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_string_new(pointer: i32, length: i32) -> ValueRef {
    runtime::string_new(pointer, length)
}

/// Allocates an empty mutable runtime list.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_list_new() -> ValueRef {
    value::list::operations::new_value()
}

/// Allocates an empty mutable runtime object.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_object_new() -> ValueRef {
    value::object::operations::new_value()
}

/// Allocates an empty nominal Object with a compiler-owned type tag.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_object_typed_new(type_id: i32) -> ValueRef {
    let Ok(type_id) = u32::try_from(type_id) else {
        runtime::trap();
    };
    if type_id == 0 {
        runtime::trap();
    }
    value::object::operations::new_typed_value(type_id)
}

/// Allocates one nominal enum value with its selected variant and ordered payload fields.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_enum_new(
    type_id: i32,
    type_identity: ValueRef,
    variant: ValueRef,
    fields: ValueRef,
) -> ValueRef {
    let Ok(type_id) = u32::try_from(type_id) else {
        runtime::trap();
    };
    runtime::enum_new(type_id, type_identity, variant, fields)
}

/// Allocates internal shared storage for one compiler-captured lexical binding.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_cell_new(value: ValueRef) -> ValueRef {
    runtime::allocate(RtValue::Cell(Box::new(value::RuntimeCellValue::new(value))))
}

/// Reads the current value stored in one internal captured-binding Cell.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_cell_get(cell: ValueRef) -> ValueRef {
    match runtime::value(cell) {
        RtValue::Cell(cell) => cell.value,
        _ => runtime::trap(),
    }
}

/// Replaces the current value stored in one internal captured-binding Cell.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_cell_set(cell: ValueRef, value: ValueRef) -> ValueRef {
    match runtime::value_mut(cell) {
        RtValue::Cell(cell) => {
            cell.value = value;
            value
        }
        _ => runtime::trap(),
    }
}

/// Creates one callable closure from a generated function identity and a List of Cell captures.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_closure_new(
    function_id: i32,
    slot_count: i32,
    arity: i32,
    captures: ValueRef,
) -> ValueRef {
    let Ok(function_id) = u32::try_from(function_id) else {
        runtime::trap();
    };
    let Ok(slot_count) = u32::try_from(slot_count) else {
        runtime::trap();
    };
    let Ok(arity) = u32::try_from(arity) else {
        runtime::trap();
    };
    let RtValue::List(captures) = runtime::value(captures) else {
        runtime::trap();
    };
    let captures = captures.elements.clone();
    if captures
        .iter()
        .any(|capture| !matches!(runtime::value(*capture), RtValue::Cell(_)))
    {
        runtime::trap();
    }
    runtime::allocate(RtValue::Closure(Box::new(value::RuntimeClosure::new(
        function_id,
        slot_count,
        arity,
        captures,
    ))))
}

/// Returns the generated function identity carried by one callable closure.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_closure_function(closure: ValueRef) -> i32 {
    let RtValue::Closure(closure) = runtime::value(closure) else {
        runtime::trap();
    };
    match i32::try_from(closure.function_id) {
        Ok(function_id) => function_id,
        Err(_) => runtime::trap(),
    }
}

/// Returns the number of captured Cells carried by one callable closure.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_closure_capture_count(closure: ValueRef) -> i32 {
    let RtValue::Closure(closure) = runtime::value(closure) else {
        runtime::trap();
    };
    match i32::try_from(closure.captures.len()) {
        Ok(length) => length,
        Err(_) => runtime::trap(),
    }
}

/// Returns the durable frame capacity required to invoke one callable closure.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_closure_slot_count(closure: ValueRef) -> i32 {
    let RtValue::Closure(closure) = runtime::value(closure) else {
        runtime::trap();
    };
    match i32::try_from(closure.slot_count) {
        Ok(slot_count) => slot_count,
        Err(_) => runtime::trap(),
    }
}

/// Returns the source argument count required by one callable closure.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_closure_arity(closure: ValueRef) -> i32 {
    let RtValue::Closure(closure) = runtime::value(closure) else {
        runtime::trap();
    };
    match i32::try_from(closure.arity) {
        Ok(arity) => arity,
        Err(_) => runtime::trap(),
    }
}

/// Creates the recoverable Error returned when a dynamic closure receives the wrong arity.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_closure_arity_error() -> ValueRef {
    let data = runtime::allocate(RtValue::None);
    runtime::recoverable_error(
        "ArityError",
        "closure received an incorrect number of arguments",
        data,
    )
}

/// Returns one captured Cell by its compiler-assigned closure environment index.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_closure_capture(closure: ValueRef, index: i32) -> ValueRef {
    let Ok(index) = usize::try_from(index) else {
        runtime::trap();
    };
    let RtValue::Closure(closure) = runtime::value(closure) else {
        runtime::trap();
    };
    match closure.captures.get(index) {
        Some(capture) => *capture,
        None => runtime::trap(),
    }
}

/// Creates the recoverable Error used for an implementation method arity mismatch.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_method_arity_error(receiver: ValueRef) -> ValueRef {
    runtime::recoverable_error(
        "ArityError",
        "method received an incorrect number of arguments",
        receiver,
    )
}

/// Appends a value through the receiver's runtime collection dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_append(receiver: ValueRef, item: ValueRef) -> ValueRef {
    value::operations::append(receiver, item)
}

/// Reads one value through the receiver's runtime indexing dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_index_get(receiver: ValueRef, index: ValueRef) -> ValueRef {
    value::operations::index_get(receiver, index)
}

/// Replaces one value through the receiver's runtime indexing dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_index_set(
    receiver: ValueRef,
    index: ValueRef,
    replacement: ValueRef,
) -> ValueRef {
    value::operations::index_set(receiver, index, replacement)
}

/// Creates the runtime-owned snapshot used by one compiled for loop.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_iter_snapshot(iterable: ValueRef) -> ValueRef {
    value::operations::iter_snapshot(iterable)
}

/// Returns the visible scalar or entry count of one runtime value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_length(value: ValueRef) -> ValueRef {
    value::operations::length(value)
}

/// Calls one statically named member method through runtime receiver dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_call_method(
    receiver: ValueRef,
    method: ValueRef,
    arguments: ValueRef,
) -> ValueRef {
    value::operations::call_method(receiver, method, arguments)
}

/// Allocates a runtime-owned linear-memory buffer for one CBOR input value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_input_alloc(length: i32) -> i32 {
    runtime::input_alloc(length)
}

/// Decodes the runner-provided CBOR input into one runtime value.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_decode_input(pointer: i32, length: i32) -> ValueRef {
    runtime::decode_input_value(pointer, length)
}

/// Returns the number of values in one runtime-owned entry argument array.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_input_argument_count(arguments: ValueRef) -> i32 {
    runtime::input_argument_count(arguments)
}

/// Returns one entry argument, substituting None when the requested argument is absent.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_input_argument(arguments: ValueRef, index: i32) -> ValueRef {
    runtime::input_argument(arguments, index)
}

/// Creates a fatal ArityError for an entry argument array that has excess values.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_input_arity_error(arguments: ValueRef) -> ValueRef {
    runtime::input_arity_error(arguments)
}

/// Encodes a completed program result into the runtime-owned CBOR result buffer.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_set_result(value: ValueRef) {
    runtime::set_result(value);
}

/// Returns the linear-memory pointer of the CBOR result buffer.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_result_ptr() -> i32 {
    runtime::result_pointer()
}

/// Returns the length of the CBOR result buffer.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_result_len() -> i32 {
    runtime::result_length()
}

/// Allocates one persistent frame for a compiler-generated resumable function.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_async_frame_new(function_id: i32, slot_count: i32) -> i32 {
    runtime::async_frame_new(function_id, slot_count)
}

/// Allocates one untraced parallel child frame without replacing the active parent task.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_async_frame_new_parallel(
    group: ValueRef,
    index: i32,
    function_id: i32,
    slot_count: i32,
) -> i32 {
    runtime::async_frame_new_parallel(group, index, function_id, slot_count)
}

/// Creates one compiler-internal parallel result group.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_parallel_new(count: i32) -> ValueRef {
    runtime::parallel_new(count)
}

/// Suspends the active parent task until its parallel child group completes.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_parallel_wait(group: ValueRef) -> i32 {
    runtime::parallel_wait(group)
}

/// Returns the completed source-order result List for one parallel group.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_parallel_take_results(group: ValueRef) -> ValueRef {
    runtime::parallel_take_results(group)
}

/// Returns the number of source List elements supplied to dynamic `par`.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_parallel_list_count(list: ValueRef) -> i32 {
    runtime::parallel_list_count(list)
}

/// Returns one source List element supplied to dynamic `par`.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_parallel_list_get(list: ValueRef, index: i32) -> ValueRef {
    runtime::parallel_list_get(list, index)
}

/// Returns whether the scheduler selected another runnable task after a host suspension.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_scheduler_status() -> i32 {
    runtime::scheduler_status()
}

/// Removes the language trace frame owned by one completing continuation frame.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_async_frame_pop_trace(frame: i32) {
    runtime::async_frame_pop_trace(frame)
}

/// Stores one persistent frame value that must survive a pending host call.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_async_frame_set_slot(frame: i32, slot: i32, value: ValueRef) {
    runtime::async_frame_set_slot(frame, slot, value);
}

/// Loads one persistent frame value after a generated dispatch resumes it.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_async_frame_get_slot(frame: i32, slot: i32) -> ValueRef {
    runtime::async_frame_get_slot(frame, slot)
}

/// Stores the next continuation-graph state for one persistent frame.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_async_frame_set_state(frame: i32, state: i32) {
    runtime::async_frame_set_state(frame, state);
}

/// Returns the next continuation-graph state for one persistent frame.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_async_frame_state(frame: i32) -> i32 {
    runtime::async_frame_state(frame)
}

/// Returns the generated function identifier stored in one persistent frame.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_async_frame_function(frame: i32) -> i32 {
    runtime::async_frame_function(frame)
}

/// Returns the active persistent frame, or zero when no resumable call is active.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_async_frame_current() -> i32 {
    runtime::async_frame_current()
}

/// Selects the persistent frame that the generated dispatcher must execute next.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_async_frame_set_current(frame: i32) {
    runtime::async_frame_set_current(frame);
}

/// Records the parent result slot consumed after a child resumable function completes.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_async_frame_set_caller(frame: i32, caller: i32, slot: i32) {
    runtime::async_frame_set_caller(frame, caller, slot);
}

/// Completes one resumable frame and stores its result for its caller or root execution.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_async_frame_complete(frame: i32, value: ValueRef) -> i32 {
    runtime::async_frame_complete(frame, value)
}

/// Takes the completed root result after a generated resumable dispatcher finishes.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_async_frame_take_completed() -> ValueRef {
    runtime::async_frame_take_completed()
}

/// Starts one dynamic runner-resolved host call with a String name and List arguments.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_host_call_start(name: ValueRef, arguments: ValueRef) -> i32 {
    runtime::host_call_start(name, arguments)
}

/// Takes the decoded Value returned by a synchronously completed host call.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_host_call_take_ready() -> ValueRef {
    runtime::host_call_take_ready()
}

/// Delivers one completed asynchronous host response encoded in runtime-owned input memory.
#[unsafe(no_mangle)]
pub extern "C" fn __exs_rt_host_call_resume(call_id: i64, pointer: i32, length: i32) -> i32 {
    runtime::host_call_resume(call_id, pointer, length)
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    runtime::trap()
}
