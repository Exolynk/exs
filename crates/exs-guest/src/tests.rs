//! Internal tests that exercise private guest execution state.

use std::sync::Mutex;

use crate::execution::{resume_response, start_inputs};
use crate::imports::set_test_host_call_status;
use crate::state::execution_mut;
use crate::{ExsValue, boxed_future, cancel};
use exs_abi::{
    HOST_CALL_FATAL, HOST_CALL_PENDING, STANDARD_ITERATOR_STEP_TYPE_IDENTITY, STATUS_COMPLETE,
    STATUS_PENDING,
};

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

/// Opens and advances a guest Host stream through the same resumable ABI as other host calls.
#[test]
fn consumes_a_host_stream() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    cancel();
    set_test_host_call_status(HOST_CALL_PENDING);
    let status = start_inputs(Vec::new(), |_| {
        boxed_future(async move {
            let mut stream = match crate::host::stream("counter", [ExsValue::Int(1)]).await {
                Ok(stream) => stream,
                Err(error) => return error,
            };
            let item = match stream.next().await {
                Ok(crate::host::IteratorStep::Item(item)) => item,
                Ok(crate::host::IteratorStep::Done) => return ExsValue::None,
                Err(error) => return error,
            };
            match stream.next().await {
                Ok(crate::host::IteratorStep::Done) => item,
                Ok(crate::host::IteratorStep::Item(_)) => ExsValue::None,
                Err(error) => error,
            }
        })
    });
    assert_eq!(status, STATUS_PENDING);
    assert_eq!(resume_response(1, ExsValue::Int(7)), STATUS_PENDING);
    assert_eq!(
        resume_response(
            2,
            ExsValue::Enum {
                type_id: STANDARD_ITERATOR_STEP_TYPE_IDENTITY.to_owned(),
                variant: "Item".to_owned(),
                fields: vec![ExsValue::Int(42)],
            },
        ),
        STATUS_PENDING
    );
    assert_eq!(
        resume_response(
            3,
            ExsValue::Enum {
                type_id: STANDARD_ITERATOR_STEP_TYPE_IDENTITY.to_owned(),
                variant: "Done".to_owned(),
                fields: Vec::new(),
            },
        ),
        STATUS_COMPLETE
    );
    assert_eq!(read_result(), ExsValue::Int(42));
    set_test_host_call_status(HOST_CALL_FATAL);
}

/// Decodes runner-provided wall-clock and monotonic-duration responses through typed guest APIs.
#[test]
fn consumes_host_time() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    cancel();
    set_test_host_call_status(HOST_CALL_PENDING);
    let status = start_inputs(Vec::new(), |_| {
        boxed_future(async move {
            let now = match crate::host::now().await {
                Ok(now) => now,
                Err(error) => return error,
            };
            let elapsed = match crate::host::elapsed().await {
                Ok(elapsed) => elapsed,
                Err(error) => return error,
            };
            ExsValue::Int(now.unix_seconds + elapsed.as_secs() as i64)
        })
    });
    assert_eq!(status, STATUS_PENDING);
    assert_eq!(
        resume_response(
            1,
            ExsValue::Object(vec![
                ("unix_seconds".to_owned(), ExsValue::Int(42)),
                ("nanoseconds".to_owned(), ExsValue::Int(123)),
                ("utc_offset_seconds".to_owned(), ExsValue::Int(3_600)),
                (
                    "timezone".to_owned(),
                    ExsValue::String("Europe/Zurich".to_owned()),
                ),
            ]),
        ),
        STATUS_PENDING
    );
    assert_eq!(
        resume_response(
            2,
            ExsValue::Object(vec![
                ("seconds".to_owned(), ExsValue::Int(1)),
                ("nanoseconds".to_owned(), ExsValue::Int(2)),
            ]),
        ),
        STATUS_COMPLETE
    );
    assert_eq!(read_result(), ExsValue::Int(43));
    set_test_host_call_status(HOST_CALL_FATAL);
}

/// Resolves typed guest DateTime zone construction and conversion host calls.
#[test]
fn resolves_guest_datetime_time_zones() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    cancel();
    set_test_host_call_status(HOST_CALL_PENDING);
    let status = start_inputs(Vec::new(), |_| {
        boxed_future(async move {
            let constructed = match crate::host::from_components_in_timezone(
                2024,
                1,
                1,
                12,
                0,
                0,
                0,
                "Europe/Zurich",
            )
            .await
            {
                Ok(value) => value,
                Err(error) => return error,
            };
            let converted = match crate::host::in_timezone(constructed, "UTC").await {
                Ok(value) => value,
                Err(error) => return error,
            };
            ExsValue::Int(i64::from(converted.utc_offset_seconds))
        })
    });
    assert_eq!(status, STATUS_PENDING);
    assert_eq!(
        resume_response(
            1,
            ExsValue::Object(vec![
                ("unix_seconds".to_owned(), ExsValue::Int(1_704_106_800)),
                ("nanoseconds".to_owned(), ExsValue::Int(0)),
                ("utc_offset_seconds".to_owned(), ExsValue::Int(3_600)),
                (
                    "timezone".to_owned(),
                    ExsValue::String("Europe/Zurich".to_owned()),
                ),
            ]),
        ),
        STATUS_PENDING
    );
    assert_eq!(
        resume_response(
            2,
            ExsValue::Object(vec![
                ("unix_seconds".to_owned(), ExsValue::Int(1_704_106_800)),
                ("nanoseconds".to_owned(), ExsValue::Int(0)),
                ("utc_offset_seconds".to_owned(), ExsValue::Int(0)),
                ("timezone".to_owned(), ExsValue::String("UTC".to_owned())),
            ]),
        ),
        STATUS_COMPLETE
    );
    assert_eq!(read_result(), ExsValue::Int(0));
    set_test_host_call_status(HOST_CALL_FATAL);
}
