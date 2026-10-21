//! Host calls of the task: started through the runner, finished at once or resumed later.

use exs_abi::HOST_CALL_READY;

use super::state::{Field, load, store};
use crate::cbor::{decode_at, encode};
use crate::errors::recoverable;
use crate::execution::{reserve_input, result};
use crate::lists::list_object;
use crate::memory::Memory;
use crate::strings::{byte_len, string_object};

/// The runner's host-call imports.
pub trait Host {
    /// Starts the host call `call_id` of the function named by the `name_len` bytes at `name`
    /// with the CBOR request of `request_len` bytes at `request`, reported at `position`.
    /// Returns `HOST_CALL_READY`, `HOST_CALL_PENDING` or `HOST_CALL_FATAL`.
    fn start(
        &mut self,
        call_id: u64,
        name: u32,
        name_len: u32,
        request: u32,
        request_len: u32,
        position: u32,
    ) -> i32;

    /// Returns the byte length of the ready response of `call_id`.
    fn response_len(&mut self, call_id: u64) -> i32;

    /// Copies the ready response of `call_id` into the `len` bytes at `address`, returning 0.
    fn response_copy(&mut self, call_id: u64, address: u32, len: u32) -> i32;
}

/// Starts the host call of the function named `name` with the List `arguments`, like v1.
///
/// Returns `HOST_CALL_READY` when its result can be taken at once, also for v1's Errors of an
/// invalid name, arguments or an argument that cannot cross the boundary; `HOST_CALL_PENDING`
/// when the task waits for the runner; or the runner's `HOST_CALL_FATAL`. Returns `None` when
/// memory is exhausted.
pub fn host_call_start(
    memory: &mut impl Memory,
    host: &mut impl Host,
    vm: u32,
    name: u64,
    arguments: u64,
) -> Option<i32> {
    let Some(name_object) = string_object(name) else {
        let error = recoverable(
            memory,
            vm,
            "TypeError",
            "Host::call requires a String function name",
            name,
        )?;
        return ready(memory, vm, error);
    };
    if list_object(arguments).is_none() {
        let error = recoverable(
            memory,
            vm,
            "TypeError",
            "Host::call arguments must be represented as a List",
            arguments,
        )?;
        return ready(memory, vm, error);
    }
    if encode(memory, vm, arguments).is_err() {
        let error = crate::boundary::serialization_error(memory, vm, arguments, false)?;
        return ready(memory, vm, error);
    }
    let call_id = load(memory, vm, Field::NextHostCall).max(1);
    store(memory, vm, Field::NextHostCall, call_id.checked_add(1)?);
    store(memory, vm, Field::HostCall, call_id);
    let (request, request_len) = result(memory, vm);
    let position = crate::vm::Vm(vm).source_position(memory);
    let status = host.start(
        call_id,
        name_object + crate::strings::DATA,
        byte_len(memory, name_object),
        request,
        request_len,
        position,
    );
    Some(status)
}

/// Makes `value` the ready result of the current host call.
fn ready(memory: &mut impl Memory, vm: u32, value: u64) -> Option<i32> {
    store(memory, vm, Field::Ready, value);
    Some(HOST_CALL_READY)
}

/// Takes the result of the current host call: a locally created Error, a resumed result, or
/// the runner's ready response, which is decoded. Returns `None` when there is no call or its
/// response is malformed.
pub fn host_call_take(memory: &mut impl Memory, host: &mut impl Host, vm: u32) -> Option<u64> {
    let ready = load(memory, vm, Field::Ready);
    if ready != 0 {
        store(memory, vm, Field::Ready, 0);
        store(memory, vm, Field::HostCall, 0);
        return Some(ready);
    }
    let call_id = load(memory, vm, Field::HostCall);
    if call_id == 0 {
        return None;
    }
    let len = u32::try_from(host.response_len(call_id)).ok()?;
    let buffer = reserve_input(memory, vm, len)?;
    if host.response_copy(call_id, buffer, len) != 0 {
        return None;
    }
    let value = decode_at(memory, vm, buffer, len).ok()?;
    store(memory, vm, Field::HostCall, 0);
    Some(value)
}

/// Delivers the runner's response of `length` bytes at `address` to the host call
/// `call_id`, which the task waits for. Returns 0, or 1 when the call was cancelled.
pub fn host_call_resume(
    memory: &mut impl Memory,
    vm: u32,
    call_id: u64,
    address: u32,
    length: u32,
) -> Option<i32> {
    if call_id != 0 && load(memory, vm, Field::CancelledCall) == call_id {
        return Some(1);
    }
    if load(memory, vm, Field::HostCall) != call_id {
        return None;
    }
    let value = decode_at(memory, vm, address, length).ok()?;
    store(memory, vm, Field::Ready, value);
    Some(0)
}

/// Cancels the task: a pending host call is remembered as cancelled and no frame stays active.
pub fn task_cancel(memory: &mut impl Memory, vm: u32) {
    let pending = load(memory, vm, Field::HostCall);
    if pending != 0 {
        store(memory, vm, Field::CancelledCall, pending);
    }
    for field in [
        Field::Frame,
        Field::Ready,
        Field::Completed,
        Field::HostCall,
    ] {
        store(memory, vm, field, 0);
    }
    let state = crate::vm::Vm(vm);
    state.set_trace_top(memory, state.trace_base(memory));
}
