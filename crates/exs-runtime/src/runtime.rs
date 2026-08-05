//! Runtime state access, CBOR conversion, and linear-memory buffers.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::num::NonZeroU32;

use exs_abi::{
    ExsError, ExsValue, HOST_CALL_FATAL, HOST_CALL_PENDING, HOST_CALL_READY, STATUS_PENDING,
    STATUS_READY,
};
use exs_value::{ValueRef, is_valid_int};

use crate::gc;
use crate::scheduler::{ExecutionContext, HostResume};
use crate::state::{AsyncFrame, FrameContinuation, HeapSlot, RuntimeState, runtime};
use crate::value::{RtValue, RuntimeEnum, RuntimeError, RuntimeList, RuntimeObject, RuntimeString};

#[link(wasm_import_module = "exs")]
unsafe extern "C" {
    /// Starts one runner-resolved host call.
    #[link_name = "__exs_host_call_start"]
    fn host_call_start_import(
        call_id: i64,
        name_pointer: i32,
        name_length: i32,
        request_pointer: i32,
        request_length: i32,
        source_position: i32,
    ) -> i32;

    /// Returns the byte length of one ready runner-owned host response.
    #[link_name = "__exs_host_call_response_len"]
    fn host_call_response_length_import(call_id: i64) -> i32;

    /// Copies one ready runner-owned host response into runtime-owned linear memory.
    #[link_name = "__exs_host_call_response_copy"]
    fn host_call_response_copy_import(
        call_id: i64,
        destination_pointer: i32,
        destination_length: i32,
    ) -> i32;
}

#[link(wasm_import_module = "runner")]
unsafe extern "C" {
    /// Acquires one runner-enforced active task permit.
    #[link_name = "__runner_task_acquire"]
    fn task_acquire_import() -> i32;

    /// Releases one runner-enforced active task permit.
    #[link_name = "__runner_task_release"]
    fn task_release_import() -> i32;
}

/// Acquires one active language-task permit from the language-neutral runner ABI.
pub(crate) fn task_acquire() {
    if unsafe { task_acquire_import() } != 0 {
        trap();
    }
}

/// Releases one active language-task permit through the language-neutral runner ABI.
pub(crate) fn task_release() {
    if unsafe { task_release_import() } != 0 {
        trap();
    }
}

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

/// Allocates one tagged enum Object after validating its runtime constructor arguments.
pub(crate) fn enum_new(
    type_id: u32,
    type_identity: ValueRef,
    variant: ValueRef,
    fields: ValueRef,
) -> ValueRef {
    let type_identity = match value(type_identity) {
        RtValue::String(value) => String::from(value.as_str()),
        _ => {
            return recoverable_error(
                "TypeError",
                "enum type identity requires a String value",
                type_identity,
            );
        }
    };
    let variant = match value(variant) {
        RtValue::String(value) => String::from(value.as_str()),
        _ => {
            return recoverable_error("TypeError", "enum variant requires a String value", variant);
        }
    };
    let fields = match value(fields) {
        RtValue::List(value) => value.elements.clone(),
        _ => {
            return recoverable_error("TypeError", "enum fields require a List value", fields);
        }
    };
    allocate(RtValue::Object(Box::new(RuntimeObject::enumeration(
        Some(type_id),
        RuntimeEnum {
            type_identity: type_identity.into_boxed_str(),
            variant: variant.into_boxed_str(),
            fields,
        },
    ))))
}

/// Returns whether one runtime value is an enum with the requested stable identity.
pub(crate) fn enum_has_type(value_ref: ValueRef, type_identity: ValueRef) -> bool {
    let RtValue::String(type_identity) = value(type_identity) else {
        return false;
    };
    matches!(
        value(value_ref),
        RtValue::Object(object)
            if object
                .enum_data
                .as_ref()
                .is_some_and(|enum_data| enum_data.type_identity.as_ref() == type_identity.as_str())
    )
}

/// Returns whether one value carries the requested enum identity and variant.
pub(crate) fn enum_matches(
    value_ref: ValueRef,
    type_identity: ValueRef,
    variant: ValueRef,
) -> bool {
    let (RtValue::String(type_identity), RtValue::String(variant)) =
        (value(type_identity), value(variant))
    else {
        return false;
    };
    matches!(
        value(value_ref),
        RtValue::Object(object)
            if object.enum_data.as_ref().is_some_and(|enum_data| {
                enum_data.type_identity.as_ref() == type_identity.as_str()
                    && enum_data.variant.as_ref() == variant.as_str()
            })
    )
}

/// Reads one ordered payload value from a matched enum or returns a recoverable Error.
pub(crate) fn enum_field(value_ref: ValueRef, index: usize) -> ValueRef {
    match value(value_ref) {
        RtValue::Object(object) => object
            .enum_data
            .as_ref()
            .and_then(|enum_data| enum_data.fields.get(index))
            .copied()
            .unwrap_or_else(|| {
                recoverable_error("MatchError", "enum payload field is unavailable", value_ref)
            }),
        _ => recoverable_error(
            "MatchError",
            "enum payload field requires an enum value",
            value_ref,
        ),
    }
}

/// Allocates one recoverable language Error using the active source position.
pub(crate) fn recoverable_error(kind: &str, message: &str, data: ValueRef) -> ValueRef {
    let origin = unsafe { runtime() }.current_source_position;
    let checkpoint = gc::temporary_root_checkpoint();
    gc::push_temporary_root(data);
    let error = allocate(RtValue::Error(Box::new(RuntimeError::recoverable(
        kind, message, data, origin,
    ))));
    gc::restore_temporary_roots(checkpoint);
    error
}

/// Allocates one fatal language Error using the active source position.
pub(crate) fn fatal_error(kind: &str, message: &str, data: ValueRef) -> ValueRef {
    let origin = unsafe { runtime() }.current_source_position;
    let checkpoint = gc::temporary_root_checkpoint();
    gc::push_temporary_root(data);
    let error = allocate(RtValue::Error(Box::new(RuntimeError::fatal(
        kind, message, data, origin,
    ))));
    gc::restore_temporary_roots(checkpoint);
    error
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

/// Starts one fresh root execution with a running scheduler task.
pub(crate) fn execution_start() {
    let state = unsafe { runtime() };
    if state
        .execution
        .as_ref()
        .is_some_and(|execution| !execution.is_complete())
    {
        trap();
    }
    state.execution = Some(ExecutionContext::start());
}

/// Consumes one compiler-emitted scheduler checkpoint for the active task.
pub(crate) fn scheduler_checkpoint() {
    execution(unsafe { runtime() }).checkpoint_current();
}

/// Cancels every live scheduler task in the active root execution.
pub(crate) fn execution_cancel() {
    execution(unsafe { runtime() }).cancel();
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

/// Allocates one persistent compiler-generated async frame and makes it active.
pub(crate) fn async_frame_new(function_id: i32, slot_count: i32) -> i32 {
    let Ok(function_id) = u32::try_from(function_id) else {
        trap();
    };
    let Ok(slot_count) = usize::try_from(slot_count) else {
        trap();
    };
    let mut slots = Vec::new();
    slots.resize(slot_count, None);
    let frame = AsyncFrame {
        function_id,
        state: 0,
        slots,
        caller: None,
        traced: true,
    };
    let state = unsafe { runtime() };
    let index = if let Some(index) = state.free_async_frames.pop() {
        let index = index as usize;
        let Some(slot) = state.async_frames.get_mut(index) else {
            trap();
        };
        if slot.is_some() {
            trap();
        }
        *slot = Some(frame);
        index
    } else {
        let index = state.async_frames.len();
        state.async_frames.push(Some(frame));
        index
    };
    let Some(identifier) = u32::try_from(index)
        .ok()
        .and_then(|index| index.checked_add(1))
    else {
        trap();
    };
    execution(state).set_current_frame(identifier);
    i32::try_from(identifier).unwrap_or_else(|_| trap())
}

/// Allocates one child task frame for a parallel group without changing the active parent task.
pub(crate) fn async_frame_new_parallel(
    group: ValueRef,
    index: i32,
    function_id: i32,
    slot_count: i32,
) -> i32 {
    let Ok(index) = usize::try_from(index) else {
        trap();
    };
    let Ok(function_id) = u32::try_from(function_id) else {
        trap();
    };
    let Ok(slot_count) = usize::try_from(slot_count) else {
        trap();
    };
    let mut slots = Vec::new();
    slots.resize(slot_count, None);
    let state = unsafe { runtime() };
    let index_frame = state.async_frames.len();
    state.async_frames.push(Some(AsyncFrame {
        function_id,
        state: 0,
        slots,
        caller: None,
        traced: false,
    }));
    let identifier = u32::try_from(index_frame)
        .ok()
        .and_then(|value| value.checked_add(1))
        .unwrap_or_else(|| trap());
    execution(state).parallel_spawn(group, index, identifier);
    i32::try_from(identifier).unwrap_or_else(|_| trap())
}

/// Creates one compiler-internal parallel result List and its scheduler group.
pub(crate) fn parallel_new(count: i32) -> ValueRef {
    let Ok(count) = usize::try_from(count) else {
        trap();
    };
    let handle = allocate(RtValue::List(Box::new(RuntimeList {
        elements: Vec::new(),
    })));
    execution(unsafe { runtime() }).parallel_new(handle, count);
    handle
}

/// Suspends the current parent task until its parallel group has completed.
pub(crate) fn parallel_wait(group: ValueRef) -> i32 {
    let execution = execution(unsafe { runtime() });
    execution.parallel_wait(group);
    if execution.has_current() {
        STATUS_READY
    } else {
        STATUS_PENDING
    }
}

/// Replaces one compiler-internal parallel handle List with its source-order child results.
pub(crate) fn parallel_take_results(group: ValueRef) -> ValueRef {
    let results = execution(unsafe { runtime() }).parallel_take_results(group);
    let RtValue::List(list) = value_mut(group) else {
        trap();
    };
    list.elements = results;
    group
}

/// Returns the number of closure values stored in one source List for dynamic `par`.
pub(crate) fn parallel_list_count(list: ValueRef) -> i32 {
    let RtValue::List(list) = value(list) else {
        trap()
    };
    i32::try_from(list.elements.len()).unwrap_or_else(|_| trap())
}

/// Returns one closure candidate from a source List for dynamic `par`.
pub(crate) fn parallel_list_get(list: ValueRef, index: i32) -> ValueRef {
    let Ok(index) = usize::try_from(index) else {
        trap()
    };
    let RtValue::List(list) = value(list) else {
        trap()
    };
    *list.elements.get(index).unwrap_or_else(|| trap())
}

/// Returns the dispatch status after a host call yielded its current task.
pub(crate) fn scheduler_status() -> i32 {
    if execution(unsafe { runtime() }).has_current() {
        STATUS_READY
    } else {
        STATUS_PENDING
    }
}

/// Pops the language error-trace entry only when this frame owns one.
pub(crate) fn async_frame_pop_trace(frame: i32) {
    let frame = async_frame_index(frame);
    let traced = unsafe { runtime() }
        .async_frames
        .get_mut(frame)
        .and_then(Option::as_mut)
        .map(|frame| core::mem::replace(&mut frame.traced, false))
        .unwrap_or_else(|| trap());
    if traced && unsafe { runtime() }.frames.pop().is_none() {
        trap();
    }
}

/// Stores one frame slot that must remain live across suspension.
pub(crate) fn async_frame_set_slot(frame: i32, slot: i32, value: ValueRef) {
    let frame = async_frame_index(frame);
    let slot = async_frame_slot(slot);
    let state = unsafe { runtime() };
    let Some(frame) = state.async_frames.get_mut(frame).and_then(Option::as_mut) else {
        trap();
    };
    let Some(destination) = frame.slots.get_mut(slot) else {
        trap();
    };
    *destination = Some(value);
}

/// Loads one initialized persistent frame slot.
pub(crate) fn async_frame_get_slot(frame: i32, slot: i32) -> ValueRef {
    let frame = async_frame_index(frame);
    let slot = async_frame_slot(slot);
    unsafe { runtime() }
        .async_frames
        .get(frame)
        .and_then(Option::as_ref)
        .and_then(|frame| frame.slots.get(slot))
        .and_then(|value| *value)
        .unwrap_or_else(|| trap())
}

/// Stores the continuation-graph state that runs when the frame is next dispatched.
pub(crate) fn async_frame_set_state(frame: i32, next_state: i32) {
    let frame = async_frame_index(frame);
    let Ok(next_state) = u32::try_from(next_state) else {
        trap();
    };
    let Some(frame) = unsafe { runtime() }
        .async_frames
        .get_mut(frame)
        .and_then(Option::as_mut)
    else {
        trap();
    };
    frame.state = next_state;
}

/// Returns the continuation-graph state stored by one persistent frame.
pub(crate) fn async_frame_state(frame: i32) -> i32 {
    let frame = async_frame_index(frame);
    let Some(frame) = unsafe { runtime() }
        .async_frames
        .get(frame)
        .and_then(Option::as_ref)
    else {
        trap();
    };
    i32::try_from(frame.state).unwrap_or_else(|_| trap())
}

/// Returns the compiler-generated function identifier for one persistent frame.
pub(crate) fn async_frame_function(frame: i32) -> i32 {
    let frame = async_frame_index(frame);
    let Some(frame) = unsafe { runtime() }
        .async_frames
        .get(frame)
        .and_then(Option::as_ref)
    else {
        trap();
    };
    i32::try_from(frame.function_id).unwrap_or_else(|_| trap())
}

/// Returns the current frame identifier, or zero when no resumable execution is active.
pub(crate) fn async_frame_current() -> i32 {
    unsafe { runtime() }
        .execution
        .as_ref()
        .and_then(ExecutionContext::current_frame)
        .map_or(0, |frame| i32::try_from(frame).unwrap_or_else(|_| trap()))
}

/// Selects one existing persistent frame as the generated dispatch target.
pub(crate) fn async_frame_set_current(frame: i32) {
    let frame = async_frame_index(frame);
    if unsafe { runtime() }
        .async_frames
        .get(frame)
        .and_then(Option::as_ref)
        .is_none()
    {
        trap();
    }
    let Some(frame) = u32::try_from(frame)
        .ok()
        .and_then(|index| index.checked_add(1))
    else {
        trap();
    };
    execution(unsafe { runtime() }).set_current_frame(frame);
}

/// Records the parent continuation consumed when a child resumable function completes.
pub(crate) fn async_frame_set_caller(frame: i32, caller: i32, slot: i32) {
    let frame = async_frame_index(frame);
    let caller = async_frame_identifier(caller);
    let Ok(slot) = u32::try_from(slot) else {
        trap();
    };
    let Some(frame) = unsafe { runtime() }
        .async_frames
        .get_mut(frame)
        .and_then(Option::as_mut)
    else {
        trap();
    };
    frame.caller = Some(FrameContinuation {
        frame: caller,
        slot,
    });
}

/// Completes one resumable frame and transfers its result to the parent continuation.
pub(crate) fn async_frame_complete(frame: i32, value: ValueRef) -> i32 {
    let frame_index = async_frame_index(frame);
    let state = unsafe { runtime() };
    let Some(completed) = state
        .async_frames
        .get_mut(frame_index)
        .and_then(Option::take)
    else {
        trap();
    };
    let Ok(free_index) = u32::try_from(frame_index) else {
        trap();
    };
    state.free_async_frames.push(free_index);
    if let Some(continuation) = completed.caller {
        let caller_index = usize::try_from(continuation.frame - 1).unwrap_or_else(|_| trap());
        let Some(caller) = state
            .async_frames
            .get_mut(caller_index)
            .and_then(Option::as_mut)
        else {
            trap();
        };
        let Some(destination) = caller.slots.get_mut(continuation.slot as usize) else {
            trap();
        };
        *destination = Some(value);
        execution(state).set_current_frame(caller_index as u32 + 1);
        0
    } else {
        if execution(state).complete_current_task(value) {
            state.completed_async_result = Some(value);
            1
        } else if execution(state).has_current() {
            0
        } else {
            2
        }
    }
}

/// Takes the completed resumable root result after the generated dispatcher reports completion.
pub(crate) fn async_frame_take_completed() -> ValueRef {
    unsafe { runtime() }
        .completed_async_result
        .take()
        .unwrap_or_else(|| trap())
}

/// Encodes and starts one generic runner-resolved host call.
pub(crate) fn host_call_start(name: ValueRef, arguments: ValueRef) -> i32 {
    let name = match value(name) {
        RtValue::String(name) => name.as_str(),
        _ => {
            return ready_host_error(
                "TypeError",
                "host.call requires a String function name",
                name,
            );
        }
    };
    if !matches!(value(arguments), RtValue::List(_)) {
        return ready_host_error(
            "TypeError",
            "host.call arguments must be represented as a List",
            arguments,
        );
    }
    let request = runtime_to_exs_value(arguments);
    let encoded = request.to_cbor().unwrap_or_else(|_| trap());
    let state = unsafe { runtime() };
    let call_id = state.next_host_call_id;
    let Some(next_call_id) = call_id.checked_add(1) else {
        trap();
    };
    state.next_host_call_id = next_call_id;
    execution(state).begin_host_call(call_id);
    state.host_request_buffer = encoded;
    let name_pointer = pointer(name.as_ptr());
    let name_length = i32::try_from(name.len()).unwrap_or_else(|_| trap());
    let request_pointer = pointer(state.host_request_buffer.as_ptr());
    let request_length = i32::try_from(state.host_request_buffer.len()).unwrap_or_else(|_| trap());
    let source_position = state
        .current_source_position
        .map_or(0, |position| position.0.cast_signed());
    let status = unsafe {
        host_call_start_import(
            call_id.cast_signed(),
            name_pointer,
            name_length,
            request_pointer,
            request_length,
            source_position,
        )
    };
    match status {
        HOST_CALL_READY => status,
        HOST_CALL_PENDING => {
            execution(unsafe { runtime() }).suspend_current_for_host(call_id);
            status
        }
        HOST_CALL_FATAL => status,
        _ => trap(),
    }
}

/// Takes and decodes the response of one synchronously completed generic host call.
pub(crate) fn host_call_take_ready() -> ValueRef {
    if let Some(value) = execution(unsafe { runtime() }).take_current_ready_host_result() {
        execution(unsafe { runtime() }).finish_current_host_call();
        return value;
    }
    let Some(call_id) = execution(unsafe { runtime() }).current_host_call() else {
        trap();
    };
    let length = unsafe { host_call_response_length_import(call_id.cast_signed()) };
    let Ok(length) = usize::try_from(length) else {
        trap();
    };
    let buffer = &mut unsafe { runtime() }.host_response_buffer;
    buffer.clear();
    buffer.resize(length, 0);
    let status = unsafe {
        host_call_response_copy_import(
            call_id.cast_signed(),
            pointer(buffer.as_ptr()),
            i32::try_from(length).unwrap_or_else(|_| trap()),
        )
    };
    if status != 0 {
        trap();
    }
    let checkpoint = gc::temporary_root_checkpoint();
    let value = exs_value_to_runtime(ExsValue::from_cbor(buffer).unwrap_or_else(|_| trap()));
    gc::restore_temporary_roots(checkpoint);
    execution(unsafe { runtime() }).finish_current_host_call();
    value
}

/// Decodes a runner-delivered asynchronous response for its waiting host call.
///
/// The runner must first allocate the runtime-owned input buffer through `__exs_input_alloc` and
/// copy exactly one canonical ExS CBOR value into it. The generated dispatcher then obtains this
/// value through `host_call_take_ready` on its next turn. Returns a nonzero value when the call
/// identifier was invalidated by cancellation.
pub(crate) fn host_call_resume(call_id: i64, pointer_value: i32, length: i32) -> i32 {
    let Ok(call_id) = u64::try_from(call_id) else {
        trap();
    };
    let checkpoint = gc::temporary_root_checkpoint();
    let value = exs_value_to_runtime(decode_input(pointer_value, length));
    gc::restore_temporary_roots(checkpoint);
    match execution(unsafe { runtime() }).resume_host_call(call_id, value) {
        HostResume::Delivered => 0,
        HostResume::Invalidated => 1,
    }
}

/// Stores one locally generated language Error as the ready result of a host call.
fn ready_host_error(kind: &str, message: &str, data: ValueRef) -> i32 {
    let value = recoverable_error(kind, message, data);
    execution(unsafe { runtime() }).set_current_ready_host_result(value);
    HOST_CALL_READY
}

/// Returns the active root scheduler or traps outside resumable root execution.
fn execution(state: &mut RuntimeState) -> &mut ExecutionContext {
    state.execution.as_mut().unwrap_or_else(|| trap())
}

/// Converts a Wasm async-frame identifier into a zero-based state-table index.
fn async_frame_index(frame: i32) -> usize {
    let frame = async_frame_identifier(frame);
    usize::try_from(frame - 1).unwrap_or_else(|_| trap())
}

/// Validates and converts one Wasm async-frame identifier.
fn async_frame_identifier(frame: i32) -> u32 {
    let Ok(frame) = u32::try_from(frame) else {
        trap();
    };
    if frame == 0 {
        trap();
    }
    frame
}

/// Converts a Wasm async-frame slot index into a native index.
fn async_frame_slot(slot: i32) -> usize {
    usize::try_from(slot).unwrap_or_else(|_| trap())
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
            let result = if let Some(enum_data) = &object.enum_data {
                ExsValue::Enum {
                    type_id: enum_data.type_identity.as_ref().into(),
                    variant: enum_data.variant.as_ref().into(),
                    fields: enum_data
                        .fields
                        .iter()
                        .copied()
                        .map(|field| runtime_to_exs_value_inner(field, active_containers))
                        .collect(),
                }
            } else {
                ExsValue::Object(
                    object
                        .entries
                        .iter()
                        .map(|(key, value)| {
                            (
                                key.as_ref().into(),
                                runtime_to_exs_value_inner(*value, active_containers),
                            )
                        })
                        .collect(),
                )
            };
            let _removed = active_containers.pop();
            result
        }
        RtValue::Cell(_) | RtValue::Closure(_) => trap(),
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
            type_id: None,
            entries: entries
                .into_iter()
                .map(|(key, value)| (key.into_boxed_str(), exs_value_to_runtime(value)))
                .collect(),
            enum_data: None,
        })),
        ExsValue::Enum {
            type_id,
            variant,
            fields,
        } => RtValue::Object(Box::new(RuntimeObject::enumeration(
            None,
            RuntimeEnum {
                type_identity: type_id.into_boxed_str(),
                variant: variant.into_boxed_str(),
                fields: fields.into_iter().map(exs_value_to_runtime).collect(),
            },
        ))),
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
