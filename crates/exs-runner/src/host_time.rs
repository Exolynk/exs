//! Runner-owned implementations of the built-in Host wall-clock and monotonic-time capabilities.

use std::time::Duration;

use exs_abi::{ErrorSeverity, ExsError, ExsValue, SourcePositionId};
use jiff::{Timestamp, Zoned, civil::DateTime as CivilDateTime};

use crate::HostCall;

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
    let [value, ExsValue::String(timezone)] = arguments.as_slice() else {
        return HostCall::Ready(time_error(
            "Host::date_time_in_timezone expects a DateTime and IANA time-zone name",
            ExsValue::List(arguments),
            origin,
        ));
    };
    let (seconds, nanoseconds) = match date_time_parts(value) {
        Ok(parts) => parts,
        Err(message) => return HostCall::Ready(time_error(&message, value.clone(), origin)),
    };
    let timestamp = match Timestamp::new(seconds, nanoseconds) {
        Ok(timestamp) => timestamp,
        Err(error) => return HostCall::Ready(date_time_error(error.to_string(), origin)),
    };
    match timestamp.in_tz(timezone) {
        Ok(value) => HostCall::Ready(date_time_value(value)),
        Err(error) => HostCall::Ready(date_time_error(error.to_string(), origin)),
    }
}

/// Resolves civil DateTime components in one IANA time zone using compatible DST disambiguation.
pub(crate) fn from_components(
    arguments: Vec<ExsValue>,
    origin: Option<SourcePositionId>,
) -> HostCall {
    let [
        ExsValue::Int(year),
        ExsValue::Int(month),
        ExsValue::Int(day),
        ExsValue::Int(hour),
        ExsValue::Int(minute),
        ExsValue::Int(second),
        ExsValue::Int(nanosecond),
        ExsValue::String(timezone),
    ] = arguments.as_slice()
    else {
        return HostCall::Ready(time_error(
            "Host::date_time_from_components expects seven Int components and an IANA time-zone name",
            ExsValue::List(arguments),
            origin,
        ));
    };
    let (Ok(year), Ok(month), Ok(day), Ok(hour), Ok(minute), Ok(second), Ok(nanosecond)) = (
        i16::try_from(*year),
        i8::try_from(*month),
        i8::try_from(*day),
        i8::try_from(*hour),
        i8::try_from(*minute),
        i8::try_from(*second),
        i32::try_from(*nanosecond),
    ) else {
        return HostCall::Ready(date_time_error(
            "DateTime component is outside the supported range".to_owned(),
            origin,
        ));
    };
    let civil = match CivilDateTime::new(year, month, day, hour, minute, second, nanosecond) {
        Ok(civil) => civil,
        Err(error) => return HostCall::Ready(date_time_error(error.to_string(), origin)),
    };
    match civil.in_tz(timezone) {
        Ok(value) => HostCall::Ready(date_time_value(value)),
        Err(error) => HostCall::Ready(date_time_error(error.to_string(), origin)),
    }
}

/// Encodes one Jiff Zoned value as the standard prelude DateTime object.
fn date_time_value(value: Zoned) -> ExsValue {
    let timestamp = value.timestamp();
    ExsValue::Object(vec![
        (
            "unix_seconds".to_owned(),
            ExsValue::Int(timestamp.as_second()),
        ),
        (
            "nanoseconds".to_owned(),
            ExsValue::Int(i64::from(timestamp.subsec_nanosecond())),
        ),
        (
            "utc_offset_seconds".to_owned(),
            ExsValue::Int(i64::from(value.offset().seconds())),
        ),
        (
            "timezone".to_owned(),
            value
                .time_zone()
                .iana_name()
                .map_or(ExsValue::None, |name| ExsValue::String(name.to_owned())),
        ),
    ])
}

/// Decodes the instant fields required for one runner DateTime operation.
fn date_time_parts(value: &ExsValue) -> Result<(i64, i32), String> {
    let ExsValue::Object(fields) = value else {
        return Err("DateTime argument must be an Object".to_owned());
    };
    let mut seconds = None;
    let mut nanoseconds = None;
    for (key, value) in fields {
        match (key.as_str(), value) {
            ("unix_seconds", ExsValue::Int(value)) if seconds.replace(*value).is_none() => {}
            ("nanoseconds", ExsValue::Int(value)) if nanoseconds.replace(*value).is_none() => {}
            ("utc_offset_seconds", ExsValue::Int(_))
            | ("timezone", ExsValue::String(_))
            | ("timezone", ExsValue::None) => {}
            _ => return Err("DateTime argument is invalid".to_owned()),
        }
    }
    let (Some(seconds), Some(nanoseconds)) = (seconds, nanoseconds) else {
        return Err("DateTime argument is incomplete".to_owned());
    };
    let nanoseconds =
        i32::try_from(nanoseconds).map_err(|_| "DateTime nanoseconds are invalid".to_owned())?;
    if !(0..1_000_000_000).contains(&nanoseconds) {
        return Err("DateTime nanoseconds are not normalized".to_owned());
    }
    Ok((seconds, nanoseconds))
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

/// Encodes one standard prelude Duration as a host-safe object.
pub(crate) fn duration_value(duration: Duration) -> ExsValue {
    ExsValue::Object(vec![
        (
            "seconds".to_owned(),
            ExsValue::Int(duration.as_secs().min(i64::MAX as u64) as i64),
        ),
        (
            "nanoseconds".to_owned(),
            ExsValue::Int(i64::from(duration.subsec_nanos())),
        ),
    ])
}

/// Builds a recoverable capability Error for invalid built-in time operation input.
fn time_error(message: &str, data: ExsValue, origin: Option<SourcePositionId>) -> ExsValue {
    ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: "TypeError".to_owned(),
        message: message.to_owned(),
        data: Box::new(data),
        origin,
        trace: Vec::new(),
        cause: None,
    })
}

/// Builds a recoverable Error returned when a DateTime operation cannot resolve its request.
fn date_time_error(message: String, origin: Option<SourcePositionId>) -> ExsValue {
    ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: "DateTimeError".to_owned(),
        message,
        data: Box::new(ExsValue::None),
        origin,
        trace: Vec::new(),
        cause: None,
    })
}
