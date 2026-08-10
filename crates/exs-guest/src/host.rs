//! Asynchronous Rust futures for the ExS Host ABI.

use alloc::string::String;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use crate::{ExsValue, begin_host_call, guest_error, take_host_response};

/// Starts one named host call from owned argument values and resolves to its decoded response.
#[must_use]
pub fn call(name: impl Into<String>, arguments: impl IntoIterator<Item = ExsValue>) -> HostCall {
    HostCall {
        name: name.into(),
        arguments: arguments.into_iter().collect(),
        call_id: None,
    }
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
