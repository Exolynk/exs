//! Runner-owned implementation of the built-in Host sleep capability.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::Duration;

use exs_abi::{ErrorSeverity, ExsError, ExsValue, SourcePositionId};

use crate::HostCall;

/// Starts one built-in Host sleep after validating its serialized Duration argument.
pub(crate) fn start(arguments: Vec<ExsValue>, origin: Option<SourcePositionId>) -> HostCall {
    let (seconds, nanoseconds) = match duration_parts(arguments) {
        Ok(parts) => parts,
        Err(message) => return HostCall::Ready(sleep_error(message, origin)),
    };
    let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let waker = Arc::new(Mutex::new(None::<Waker>));
    let completion = Arc::clone(&completed);
    let wake_target = Arc::clone(&waker);
    let sleep = Duration::new(seconds, nanoseconds);
    let spawned = thread::Builder::new()
        .name("exs-sleep".to_owned())
        .spawn(move || {
            thread::sleep(sleep);
            completion.store(true, std::sync::atomic::Ordering::Release);
            let mut waker = wake_target
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(waker) = waker.take() {
                waker.wake();
            }
        });
    match spawned {
        Ok(_thread) => HostCall::Pending(Box::pin(SleepFuture { completed, waker })),
        Err(error) => HostCall::Ready(sleep_error(
            format!("could not start Host sleep: {error}"),
            origin,
        )),
    }
}

/// Validates one serialized Duration Object and returns normalized duration parts.
fn duration_parts(arguments: Vec<ExsValue>) -> Result<(u64, u32), String> {
    let [ExsValue::Object(entries)] = arguments.as_slice() else {
        return Err("Host::sleep expects exactly one Duration argument".to_owned());
    };
    let mut seconds = None;
    let mut nanoseconds = None;
    for (key, value) in entries {
        let ExsValue::Int(value) = value else {
            return Err("Host::sleep received an invalid Duration value".to_owned());
        };
        match key.as_str() {
            "seconds" if seconds.replace(*value).is_none() => {}
            "nanoseconds" if nanoseconds.replace(*value).is_none() => {}
            _ => return Err("Host::sleep received an invalid Duration value".to_owned()),
        }
    }
    let (Some(seconds), Some(nanoseconds)) = (seconds, nanoseconds) else {
        return Err("Host::sleep received an invalid Duration value".to_owned());
    };
    let seconds = u64::try_from(seconds)
        .map_err(|_| "Host::sleep received a negative Duration value".to_owned())?;
    let nanoseconds = u32::try_from(nanoseconds)
        .map_err(|_| "Host::sleep received an invalid Duration value".to_owned())?;
    if nanoseconds >= 1_000_000_000 {
        return Err("Host::sleep received a non-normalized Duration value".to_owned());
    }
    Ok((seconds, nanoseconds))
}

/// Creates one recoverable Host sleep capability Error.
fn sleep_error(message: String, origin: Option<SourcePositionId>) -> ExsValue {
    ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: "HostSleepError".to_owned(),
        message,
        data: Box::new(ExsValue::None),
        origin,
        trace: Vec::new(),
        cause: None,
    })
}

/// A future that completes after one native thread has slept for the requested duration.
struct SleepFuture {
    /// Completion signal set by the sleep thread.
    completed: Arc<std::sync::atomic::AtomicBool>,
    /// Latest task waker to notify once the sleep expires.
    waker: Arc<Mutex<Option<Waker>>>,
}

impl Future for SleepFuture {
    type Output = ExsValue;

    /// Polls the sleep completion signal without blocking the runner's async executor.
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.completed.load(std::sync::atomic::Ordering::Acquire) {
            return Poll::Ready(ExsValue::None);
        }
        let mut waker = self.waker.lock().unwrap_or_else(|error| error.into_inner());
        *waker = Some(context.waker().clone());
        if self.completed.load(std::sync::atomic::Ordering::Acquire) {
            Poll::Ready(ExsValue::None)
        } else {
            Poll::Pending
        }
    }
}
