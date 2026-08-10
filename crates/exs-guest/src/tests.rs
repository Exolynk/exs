//! Internal tests that exercise private guest execution state.

use std::sync::Mutex;

use crate::execution::{resume_response, start_inputs};
use crate::imports::set_test_host_call_status;
use crate::state::execution_mut;
use crate::{ExsValue, boxed_future, cancel};
use exs_abi::{HOST_CALL_FATAL, HOST_CALL_PENDING, STATUS_COMPLETE, STATUS_PENDING};

/// Serializes tests over the singleton guest-instance ABI state.
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Decodes the completed result retained by private execution state.
fn read_result() -> ExsValue {
    let crate::state::Execution::Completed(bytes) = execution_mut() else {
        panic!("test execution did not complete");
    };
    match ExsValue::from_cbor(bytes) {
        Ok(value) => value,
        Err(error) => panic!("test result decoding failed: {error}"),
    }
}

/// Executes a Rust async entry that completes during its initial poll.
#[test]
fn completes_an_async_entry() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    cancel();
    let status = start_inputs(vec![ExsValue::Int(42)], |inputs| {
        boxed_future(async move { inputs.into_iter().next().unwrap_or(ExsValue::None) })
    });
    assert_eq!(status, STATUS_COMPLETE);
    assert_eq!(read_result(), ExsValue::Int(42));
}

/// Suspends an async host call and completes it after a runner-style ABI resume.
#[test]
fn resumes_an_async_host_call() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    cancel();
    set_test_host_call_status(HOST_CALL_PENDING);
    let status = start_inputs(vec![ExsValue::Int(7)], |inputs| {
        boxed_future(async move { crate::host::call("echo", inputs).await })
    });
    assert_eq!(status, STATUS_PENDING);
    assert_eq!(resume_response(1, ExsValue::Int(7)), STATUS_COMPLETE);
    assert_eq!(read_result(), ExsValue::Int(7));
    set_test_host_call_status(HOST_CALL_FATAL);
}
