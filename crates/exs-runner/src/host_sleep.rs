//! Runner-owned implementation of the built-in Host sleep capability.

use std::time::Duration;

use exs_abi::{ErrorSeverity, ExsError, ExsValue, SourcePositionId};

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
