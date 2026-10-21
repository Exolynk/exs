//! Host calls of the task: started through the runner, finished at once or resumed later.

use exs_abi::{HOST_CALL_READY, HOST_CALL_UNSUPPORTED};

use super::state::{Field, load, store};
use crate::errors::recoverable;
use crate::execution::reserve_input;
use crate::image::adopt_at;
use crate::lists::list_object;
use crate::memory::Memory;
use crate::strings::{byte_len, string_object};

/// The runner's host-call imports.
pub trait Host {
    /// Starts the host call `call_id` of the function named by the `name_len` bytes at `name`
    /// with the List `arguments`, which the runner reads from the memory of the VM at `vm`,
    /// reported at `position`. Returns `HOST_CALL_READY`, `HOST_CALL_PENDING`,
    /// `HOST_CALL_FATAL`, or `HOST_CALL_UNSUPPORTED` when the arguments cannot cross the
    /// boundary.
    fn start(
        &mut self,
        call_id: u64,
        name: u32,
        name_len: u32,
        arguments: u64,
        vm: u32,
        position: u32,
    ) -> i32;

    /// Returns the byte length of the ready response of `call_id`.
    fn response_len(&mut self, call_id: u64) -> i32;

    /// Copies the ready response of `call_id` into the `len` bytes at `address`, returning 0.
    fn response_copy(&mut self, call_id: u64, address: u32, len: u32) -> i32;

    /// Acquires the runner's permit for one more task, returning `false` when it refuses.
    fn acquire(&mut self) -> bool;

    /// Releases the permit of one task.
    fn release(&mut self);
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
    let call_id = load(memory, vm, Field::NextHostCall).max(1);
    store(memory, vm, Field::NextHostCall, call_id.checked_add(1)?);
    store(memory, vm, Field::HostCall, call_id);
    let position = crate::vm::Vm(vm).source_position(memory);
    let status = host.start(
        call_id,
        name_object + crate::strings::DATA,
        byte_len(memory, name_object),
        arguments,
        vm,
        position,
    );
    if status == HOST_CALL_UNSUPPORTED {
        // Like v1, arguments that cannot cross the boundary are a recoverable Error value.
        let error = crate::boundary::serialization_error(memory, vm, arguments, false)?;
        return ready(memory, vm, error);
    }
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
    let value = adopt_at(memory, vm, buffer, len).ok()?;
    store(memory, vm, Field::HostCall, 0);
    Some(value)
}

/// Delivers the runner's response of `length` bytes at `address` to the task waiting for the
/// host call `call_id`, which becomes runnable. Returns 0, or 1 when no task waits for the
/// call because it was cancelled.
pub fn host_call_resume(
    memory: &mut impl Memory,
    vm: u32,
    call_id: u64,
    address: u32,
    length: u32,
) -> Option<i32> {
    if call_id == 0 || !super::scheduler::waits_for(memory, vm, call_id) {
        return Some(1);
    }
    let value = adopt_at(memory, vm, address, length).ok()?;
    let delivered = super::scheduler::deliver(memory, vm, call_id, value)?;
    Some(i32::from(!delivered))
}

/// Cancels the execution: every task ends, the permits of `par` children are released and
/// their pending host calls are abandoned.
pub fn task_cancel(memory: &mut impl Memory, host: &mut impl Host, vm: u32) {
    super::scheduler::end_tasks(memory, host, vm);
}
