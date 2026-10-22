//! Guest execution control for the ExS start, resume, and result ABI.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use exs_abi::{
    BoundaryLimits, HOST_CALL_FATAL, HOST_CALL_PENDING, HOST_CALL_READY, STATUS_CANCELLED,
    STATUS_COMPLETE, STATUS_PENDING,
};

use crate::imports::{
    host_call_response_copy, host_call_response_len, host_call_start, task_acquire, task_release,
};
use crate::state::{Execution, buffers_mut, execution_mut, host_state_mut};
use crate::{ErrorSeverity, ExsError, ExsValue, GuestFuture};

/// Allocates one runner-writable input or host-response buffer in guest linear memory.
pub fn input_alloc(length: i32) -> i32 {
    let Ok(length) = usize::try_from(length) else {
        return -1;
    };
    let mut bytes = Vec::new();
    if bytes.try_reserve_exact(length).is_err() {
        return -1;
    }
    bytes.resize(length, 0);
    let Ok(pointer) = i32::try_from(bytes.as_mut_ptr() as usize) else {
        return -1;
    };
    buffers_mut().push(bytes);
    pointer
}

/// Starts one fresh guest execution from the runner-owned input image.
pub fn start<Entry>(pointer: i32, length: i32, entry: Entry) -> i32
where
    Entry: FnOnce(Vec<ExsValue>) -> GuestFuture,
{
    let Some(input) = take_buffer(pointer, length) else {
        return STATUS_CANCELLED;
    };
    let Ok(ExsValue::List(inputs)) = read_image(&input) else {
        return STATUS_CANCELLED;
    };
    start_inputs(inputs, entry)
}

/// Starts one fresh guest execution from decoded runner input values.
pub(crate) fn start_inputs<Entry>(inputs: Vec<ExsValue>, entry: Entry) -> i32
where
    Entry: FnOnce(Vec<ExsValue>) -> GuestFuture,
{
    cancel();
    if task_acquire() != 0 {
        return STATUS_CANCELLED;
    }
    *execution_mut() = Execution::Running(entry(inputs));
    poll_execution()
}

/// Supplies one runner-owned host response and resumes the active guest future.
pub fn resume_host(call_id: i64, pointer: i32, length: i32) -> i32 {
    let Some(response) = take_buffer(pointer, length) else {
        return STATUS_CANCELLED;
    };
    resume_response(call_id, host_response(&response))
}

/// Supplies one decoded host response and resumes the active guest future.
pub(crate) fn resume_response(call_id: i64, response: ExsValue) -> i32 {
    host_state_mut().responses.push((call_id, response));
    poll_execution()
}

/// Cancels the active guest future and releases its runner task permit when needed.
pub fn cancel() {
    let execution = core::mem::replace(execution_mut(), Execution::Idle);
    if matches!(execution, Execution::Running(_)) {
        let _status = task_release();
    }
    let host_state = host_state_mut();
    host_state.responses.clear();
    host_state.next_call_id = 1;
    buffers_mut().clear();
}

/// Returns where the image of the last completed result lies: its linear-memory address in
/// the low 32 bits and its byte length in the high 32 bits, or 0 without a result.
#[must_use]
pub fn result_image() -> i64 {
    match execution_mut() {
        Execution::Completed(result) => {
            let (Ok(address), Ok(length)) = (
                u32::try_from(result.as_ptr() as usize),
                u32::try_from(result.len()),
            ) else {
                return 0;
            };
            (u64::from(length) << 32 | u64::from(address)).cast_signed()
        }
        Execution::Idle | Execution::Running(_) => 0,
    }
}

/// Starts one Host ABI call or reads its immediate response.
pub(crate) fn begin_host_call(
    existing: Option<i64>,
    name: &str,
    arguments: &[ExsValue],
) -> Option<(i64, Option<ExsValue>)> {
    let call_id = existing.unwrap_or_else(next_host_call_id);
    let request = write_image(&ExsValue::List(arguments.to_vec())).ok()?;
    let status = host_call_start(
        call_id,
        name.as_ptr(),
        i32::try_from(name.len()).ok()?,
        request.as_ptr(),
        i32::try_from(request.len()).ok()?,
        -1,
    );
    match status {
        HOST_CALL_READY => Some((call_id, read_ready_host_response(call_id))),
        HOST_CALL_PENDING => Some((call_id, None)),
        HOST_CALL_FATAL => Some((
            call_id,
            Some(guest_error("HostCallFailed", "runner rejected host call")),
        )),
        _ => Some((
            call_id,
            Some(guest_error(
                "HostCallFailed",
                "runner returned an invalid host-call status",
            )),
        )),
    }
}

/// Returns and removes the asynchronous response for one suspended host call.
pub(crate) fn take_host_response(call_id: i64) -> Option<ExsValue> {
    let responses = &mut host_state_mut().responses;
    let index = responses.iter().position(|(id, _)| *id == call_id)?;
    Some(responses.swap_remove(index).1)
}

/// Constructs one fatal ExS error for failures at the Rust guest boundary.
pub(crate) fn guest_error(kind: &str, message: &str) -> ExsValue {
    ExsValue::Error(ExsError {
        severity: ErrorSeverity::Fatal,
        kind: String::from(kind),
        message: String::from(message),
        data: Box::new(ExsValue::None),
        origin: None,
        trace: Vec::new(),
        cause: None,
    })
}

fn poll_execution() -> i32 {
    let execution = core::mem::replace(execution_mut(), Execution::Idle);
    let Execution::Running(mut future) = execution else {
        return STATUS_CANCELLED;
    };
    let waker = noop_waker();
    let mut context = Context::from_waker(&waker);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => complete(value),
        Poll::Pending => {
            *execution_mut() = Execution::Running(future);
            STATUS_PENDING
        }
    }
}

/// Encodes and retains one completed guest result for the runner to read.
fn complete(value: ExsValue) -> i32 {
    let result = match write_image(&value) {
        Ok(result) => result,
        Err(_) => write_image(&guest_error(
            "ResultEncodingFailed",
            "guest result could not be encoded as an ExS image",
        ))
        .unwrap_or_default(),
    };
    *execution_mut() = Execution::Completed(result);
    let _status = task_release();
    STATUS_COMPLETE
}

/// Reads and decodes one synchronous runner host-call response.
fn read_ready_host_response(call_id: i64) -> Option<ExsValue> {
    let length = usize::try_from(host_call_response_len(call_id)).ok()?;
    let mut response = Vec::new();
    response.try_reserve_exact(length).ok()?;
    response.resize(length, 0);
    if host_call_response_copy(call_id, response.as_mut_ptr(), i32::try_from(length).ok()?) != 0 {
        return Some(guest_error(
            "HostCallFailed",
            "runner could not copy host response",
        ));
    }
    Some(host_response(&response))
}

/// Decodes one runner host response image, or a fatal Error for an invalid one.
fn host_response(response: &[u8]) -> ExsValue {
    read_image(response).unwrap_or_else(|_| {
        guest_error(
            "InvalidHostResponse",
            "host response is not a valid ExS image",
        )
    })
}

/// Decodes one image the runner wrote; the runner already applied its limits.
fn read_image(bytes: &[u8]) -> Result<ExsValue, exs_abi::BoundaryError> {
    ExsValue::from_image_with_limits(bytes, BoundaryLimits::unrestricted())
}

/// Encodes one value the runner reads as an image; the runner applies its limits.
fn write_image(value: &ExsValue) -> Result<Vec<u8>, exs_abi::BoundaryError> {
    value.to_image_with_limits(BoundaryLimits::unrestricted())
}

/// Allocates the next positive identifier for an in-flight host call.
fn next_host_call_id() -> i64 {
    let state = host_state_mut();
    let call_id = state.next_call_id;
    state.next_call_id = state.next_call_id.checked_add(1).unwrap_or(1);
    call_id
}

/// Takes one runner-writable allocation after verifying its pointer and length.
fn take_buffer(pointer: i32, length: i32) -> Option<Vec<u8>> {
    let pointer = usize::try_from(pointer).ok()?;
    let length = usize::try_from(length).ok()?;
    let buffers = buffers_mut();
    let index = buffers
        .iter()
        .position(|buffer| buffer.as_ptr() as usize == pointer && buffer.len() == length)?;
    Some(buffers.swap_remove(index))
}

/// Creates a no-op waker used to poll guest futures at ABI boundaries.
fn noop_waker() -> Waker {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        |_| RawWaker::new(core::ptr::null(), &VTABLE),
        |_| {},
        |_| {},
        |_| {},
    );
    // The vtable never dereferences its null data pointer.
    unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VTABLE)) }
}
