//! Runner-owned implementation of the built-in Host sleep capability.

use std::sync::Arc;
use std::time::{Duration, Instant};

use exs_abi::{ExsValue, SourcePositionId};

use crate::host_values::{duration_parts, sleep_error};
use crate::{HostCall, RunnerTimer};

/// Starts one built-in Host sleep after validating its serialized Duration argument. The sleep
/// ends at the execution deadline at the latest.
pub(crate) fn start(
    arguments: Vec<ExsValue>,
    deadline_at: Instant,
    timer: &Arc<dyn RunnerTimer>,
    origin: Option<SourcePositionId>,
) -> HostCall {
    let (seconds, nanoseconds) = match duration_parts(arguments) {
        Ok(parts) => parts,
        Err(message) => return HostCall::Ready(sleep_error(message, origin)),
    };
    let requested = Duration::new(seconds, nanoseconds);
    let wake_at = Instant::now()
        .checked_add(requested)
        .map_or(deadline_at, |wake_at| wake_at.min(deadline_at));
    let sleep = timer.sleep_until(wake_at);
    HostCall::Pending(Box::pin(async move {
        sleep.await;
        ExsValue::Unit
    }))
}
