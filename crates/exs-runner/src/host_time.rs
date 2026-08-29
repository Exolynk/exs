//! Runner-owned implementations of the built-in Host wall-clock and monotonic-time capabilities.

use std::time::Duration;

use exs_abi::{ExsValue, SourcePositionId};
use jiff::Zoned;

use crate::HostCall;
use crate::host_values::{
    date_time_from_components, date_time_in_timezone, date_time_value, duration_value, time_error,
};

/// Returns the current wall-clock instant together with the system zone snapshot.
pub(crate) fn now(arguments: Vec<ExsValue>, origin: Option<SourcePositionId>) -> HostCall {
    if !arguments.is_empty() {
        return HostCall::Ready(time_error(
            "Host::now expects no arguments",
            ExsValue::List(arguments),
            origin,
        ));
    }
    HostCall::Ready(date_time_value(Zoned::now()))
}

/// Renders one runner-provided DateTime instant in the requested IANA time zone.
pub(crate) fn in_timezone(arguments: Vec<ExsValue>, origin: Option<SourcePositionId>) -> HostCall {
    HostCall::Ready(date_time_in_timezone(arguments, origin))
}

/// Resolves civil DateTime components in one IANA time zone using compatible DST disambiguation.
pub(crate) fn from_components(
    arguments: Vec<ExsValue>,
    origin: Option<SourcePositionId>,
) -> HostCall {
    HostCall::Ready(date_time_from_components(arguments, origin))
}

/// Returns the monotonic duration elapsed since the current root execution started.
pub(crate) fn elapsed(
    arguments: Vec<ExsValue>,
    elapsed: Duration,
    origin: Option<SourcePositionId>,
) -> HostCall {
    if !arguments.is_empty() {
        return HostCall::Ready(time_error(
            "Host::elapsed expects no arguments",
            ExsValue::List(arguments),
            origin,
        ));
    }
    HostCall::Ready(duration_value(elapsed))
}
