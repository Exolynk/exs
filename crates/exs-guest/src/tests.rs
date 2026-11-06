//! Internal tests that exercise private guest execution state.

use std::sync::Mutex;

use core::future::poll_fn;
use core::task::Poll;

use crate::__private::{boxed_future, cancel};
use crate::execution::{resume_response, start_inputs};
use crate::imports::{set_test_host_call_status, test_host_call_start_count};
use crate::state::execution_mut;
use crate::{ExsError, ExsValue};
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
    match ExsValue::from_image_with_limits(bytes, exs_abi::BoundaryLimits::unrestricted()) {
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

/// Keeps a pending HostCall from starting again when another call completes first.
#[test]
fn preserves_pending_host_calls_when_responses_complete_out_of_order() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    cancel();
    set_test_host_call_status(HOST_CALL_PENDING);
    let status = start_inputs(Vec::new(), |_| {
        boxed_future(async move {
            let mut first = Box::pin(crate::host::call("first", []));
            let mut second = Box::pin(crate::host::call("second", []));
            let mut first_response = None;
            let mut second_response = None;
            poll_fn(move |context| {
                if first_response.is_none()
                    && let Poll::Ready(response) = first.as_mut().poll(context)
                {
                    first_response = Some(response);
                }
                if second_response.is_none()
                    && let Poll::Ready(response) = second.as_mut().poll(context)
                {
                    second_response = Some(response);
                }
                match (first_response.take(), second_response.take()) {
                    (Some(first), Some(second)) => Poll::Ready(ExsValue::List(vec![first, second])),
                    (first, second) => {
                        first_response = first;
                        second_response = second;
                        Poll::Pending
                    }
                }
            })
            .await
        })
    });
    assert_eq!(status, STATUS_PENDING);
    assert_eq!(test_host_call_start_count(), 2);
    assert_eq!(resume_response(2, ExsValue::Int(20)), STATUS_PENDING);
    assert_eq!(test_host_call_start_count(), 2);
    assert_eq!(resume_response(1, ExsValue::Int(10)), STATUS_COMPLETE);
    assert_eq!(
        read_result(),
        ExsValue::List(vec![ExsValue::Int(10), ExsValue::Int(20)])
    );
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
            let mut stream = crate::host::stream("counter", [ExsValue::Int(1)]).await?;
            let item = match stream.next().await? {
                crate::host::IteratorStep::Item(item) => item,
                crate::host::IteratorStep::Done => ExsValue::None,
            };
            match stream.next().await? {
                crate::host::IteratorStep::Done => Ok::<_, ExsError>(item),
                crate::host::IteratorStep::Item(_) => Ok(ExsValue::None),
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
            let now = crate::host::now().await?;
            let elapsed = crate::host::elapsed().await?;
            Ok::<_, ExsError>(ExsValue::Int(now.unix_seconds + elapsed.as_secs() as i64))
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
            let components = crate::host::DateTimeComponents {
                year: 2024,
                month: 1,
                day: 1,
                hour: 12,
                ..crate::host::DateTimeComponents::default()
            };
            let constructed =
                crate::host::from_components_in_timezone(components, "Europe/Zurich").await?;
            let converted = crate::host::in_timezone(&constructed, "UTC").await?;
            Ok::<_, ExsError>(ExsValue::Int(i64::from(converted.utc_offset_seconds)))
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

/// Completes a sleep when the runner responds with Unit.
#[test]
fn completes_a_sleep() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    cancel();
    set_test_host_call_status(HOST_CALL_PENDING);
    let status = start_inputs(Vec::new(), |_| {
        boxed_future(async move {
            crate::host::sleep(core::time::Duration::from_millis(5)).await?;
            Ok::<_, ExsError>(ExsValue::Int(1))
        })
    });
    assert_eq!(status, STATUS_PENDING);
    assert_eq!(resume_response(1, ExsValue::Unit), STATUS_COMPLETE);
    assert_eq!(read_result(), ExsValue::Int(1));
    set_test_host_call_status(HOST_CALL_FATAL);
}

/// Reports a response to a sleep that is neither Unit nor an Error as a fatal protocol Error.
#[test]
fn rejects_a_malformed_sleep_response() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    cancel();
    set_test_host_call_status(HOST_CALL_PENDING);
    let status = start_inputs(Vec::new(), |_| {
        boxed_future(async move {
            crate::host::sleep(core::time::Duration::ZERO).await?;
            Ok::<_, ExsError>(ExsValue::Int(1))
        })
    });
    assert_eq!(status, STATUS_PENDING);
    assert_eq!(resume_response(1, ExsValue::Int(2)), STATUS_COMPLETE);
    let ExsValue::Error(error) = read_result() else {
        panic!("a malformed response must fail the entry");
    };
    assert_eq!(error.kind, "InvalidHostTimeResponse");
    set_test_host_call_status(HOST_CALL_FATAL);
}

/// Converts between Rust types and values, and keeps an Error as that Error.
#[cfg(feature = "serde")]
#[test]
fn decodes_and_encodes_typed_values() {
    use std::collections::BTreeMap;

    let list = crate::encode(&vec![1_i64, 2]).expect("a list encodes");
    assert_eq!(
        list,
        ExsValue::List(vec![ExsValue::Int(1), ExsValue::Int(2)])
    );
    assert_eq!(crate::decode::<Vec<i64>>(list.clone()), Ok(vec![1, 2]));

    let mismatch = crate::decode::<String>(list).expect_err("a list is no String");
    assert_eq!(mismatch.kind, "WireDecodeError");

    let error = ExsError::recoverable("Denied", "no access");
    assert_eq!(
        crate::decode::<i64>(ExsValue::Error(error.clone())),
        Err(error)
    );

    let keys = BTreeMap::from([(1_i64, 2_i64)]);
    let unsupported = crate::encode(&keys).expect_err("Object keys are strings");
    assert_eq!(unsupported.kind, "WireEncodeError");
}

/// Completes an entry whose `?` met an Error with that Error as its result.
#[test]
fn completes_an_entry_with_the_error_of_a_question_mark() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    cancel();
    set_test_host_call_status(HOST_CALL_PENDING);
    let status = start_inputs(Vec::new(), |_| {
        boxed_future(async move {
            crate::host::now().await?;
            Ok::<_, ExsError>(ExsValue::Int(1))
        })
    });
    assert_eq!(status, STATUS_PENDING);
    let error = ExsError::recoverable("Denied", "no clock");
    assert_eq!(
        resume_response(1, ExsValue::Error(error.clone())),
        STATUS_COMPLETE
    );
    assert_eq!(read_result(), ExsValue::Error(error));
}
