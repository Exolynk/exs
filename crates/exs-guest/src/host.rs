//! Asynchronous Rust futures for the ExS Host ABI.

use alloc::string::String;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use exs_abi::{
    HOST_STREAM_NEXT_HOST_NAME, HOST_STREAM_OPEN_HOST_NAME, STANDARD_ITERATOR_STEP_TYPE_IDENTITY,
};

use crate::{ExsValue, begin_host_call, guest_error, take_host_response};

/// The result of advancing one guest-owned Host stream.
#[derive(Clone, Debug, PartialEq)]
pub enum IteratorStep {
    /// One stream value is available.
    Item(ExsValue),
    /// The stream has no remaining values.
    Done,
}

/// One runner-owned Host stream addressed through its opaque integer handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostStream {
    /// Handle returned by the runner's stream-open operation.
    handle: i64,
}

/// Starts one named host call from owned argument values and resolves to its decoded response.
#[must_use]
pub fn call(name: impl Into<String>, arguments: impl IntoIterator<Item = ExsValue>) -> HostCall {
    HostCall {
        name: name.into(),
        arguments: arguments.into_iter().collect(),
        call_id: None,
    }
}

/// Opens one runner-registered Host stream from a name and ordered factory arguments.
pub async fn stream(
    name: impl Into<String>,
    arguments: impl IntoIterator<Item = ExsValue>,
) -> Result<HostStream, ExsValue> {
    let mut request = Vec::new();
    request.push(ExsValue::String(name.into()));
    request.extend(arguments);
    match call(HOST_STREAM_OPEN_HOST_NAME, request).await {
        ExsValue::Int(handle) => Ok(HostStream { handle }),
        error @ ExsValue::Error(_) => Err(error),
        value => Err(stream_protocol_error(
            "stream open returned a non-integer handle",
            value,
        )),
    }
}

impl HostStream {
    /// Advances this stream once and decodes its IteratorStep response.
    pub async fn next(&mut self) -> Result<IteratorStep, ExsValue> {
        match call(HOST_STREAM_NEXT_HOST_NAME, [ExsValue::Int(self.handle)]).await {
            error @ ExsValue::Error(_) => Err(error),
            ExsValue::Enum {
                type_id,
                variant,
                mut fields,
            } if type_id == STANDARD_ITERATOR_STEP_TYPE_IDENTITY && variant == "Item" => {
                if fields.len() == 1 {
                    Ok(IteratorStep::Item(fields.remove(0)))
                } else {
                    Err(stream_protocol_error(
                        "IteratorStep::Item must contain exactly one value",
                        ExsValue::List(fields),
                    ))
                }
            }
            ExsValue::Enum {
                type_id,
                variant,
                fields,
            } if type_id == STANDARD_ITERATOR_STEP_TYPE_IDENTITY && variant == "Done" => {
                if fields.is_empty() {
                    Ok(IteratorStep::Done)
                } else {
                    Err(stream_protocol_error(
                        "IteratorStep::Done must not contain values",
                        ExsValue::List(fields),
                    ))
                }
            }
            value => Err(stream_protocol_error(
                "stream next returned an invalid IteratorStep value",
                value,
            )),
        }
    }
}

/// Builds the fatal guest-boundary Error used for malformed runner stream responses.
fn stream_protocol_error(message: &str, value: ExsValue) -> ExsValue {
    let mut error = guest_error("InvalidHostStreamResponse", message);
    if let ExsValue::Error(ref mut error_value) = error {
        *error_value.data = value;
    }
    error
}

/// One lazily started host call future.
pub struct HostCall {
    /// Name passed through the ExS Host ABI.
    name: String,
    /// Ordered arguments encoded as the Host ABI request list.
    arguments: Vec<ExsValue>,
    /// Runner-assigned continuation identity after the initial poll.
    call_id: Option<i64>,
}

impl Future for HostCall {
    type Output = ExsValue;

    /// Polls the Host ABI once and yields until the runner provides an asynchronous response.
    fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(call_id) = self.call_id
            && let Some(response) = take_host_response(call_id)
        {
            return Poll::Ready(response);
        }
        let Some((call_id, response)) = begin_host_call(self.call_id, &self.name, &self.arguments)
        else {
            return Poll::Ready(guest_error("HostCallFailed", "could not start host call"));
        };
        self.call_id = Some(call_id);
        match response {
            Some(response) => Poll::Ready(response),
            None => Poll::Pending,
        }
    }
}
