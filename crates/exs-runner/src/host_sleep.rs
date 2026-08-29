//! Runner-owned implementation of the built-in Host sleep capability.

use std::time::Duration;

use exs_abi::{ExsValue, SourcePositionId};

use crate::host_values::{duration_parts, sleep_error};
use crate::{HostCall, timer};

/// Starts one built-in Host sleep after validating its serialized Duration argument.
pub(crate) fn start(
    arguments: Vec<ExsValue>,
    remaining_until_deadline: Duration,
    origin: Option<SourcePositionId>,
) -> HostCall {
    let (seconds, nanoseconds) = match duration_parts(arguments) {
        Ok(parts) => parts,
        Err(message) => return HostCall::Ready(sleep_error(message, origin)),
    };
    let requested = Duration::new(seconds, nanoseconds);
    let duration = requested.min(remaining_until_deadline);
    match timer::sleep(duration) {
        Ok(sleep) => HostCall::Pending(Box::pin(async move {
            sleep.await;
            ExsValue::None
        })),
        Err(error) => HostCall::Ready(sleep_error(
            format!("could not start Host sleep timer: {error}"),
            origin,
        )),
    }
}
