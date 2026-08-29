//! Target-neutral ExS Host value validation, conversion, and language errors.

use std::time::Duration;

use exs_abi::{ErrorSeverity, ExsError, ExsValue, SourcePositionId};
use jiff::{Timestamp, Zoned, civil::DateTime as CivilDateTime};

/// Resolves one serialized DateTime in the requested IANA time zone.
pub(crate) fn date_time_in_timezone(
    arguments: Vec<ExsValue>,
    origin: Option<SourcePositionId>,
) -> ExsValue {
    let [value, ExsValue::String(timezone)] = arguments.as_slice() else {
        return time_error(
            "Host::date_time_in_timezone expects a DateTime and IANA time-zone name",
            ExsValue::List(arguments),
            origin,
        );
    };
    let (seconds, nanoseconds) = match date_time_parts(value) {
        Ok(parts) => parts,
        Err(message) => return time_error(&message, value.clone(), origin),
    };
    let timestamp = match Timestamp::new(seconds, nanoseconds) {
        Ok(timestamp) => timestamp,
        Err(error) => return date_time_error(error.to_string(), origin),
    };
    match timestamp.in_tz(timezone) {
        Ok(value) => date_time_value(value),
        Err(error) => date_time_error(error.to_string(), origin),
    }
}

/// Resolves civil DateTime components in the requested IANA time zone.
pub(crate) fn date_time_from_components(
    arguments: Vec<ExsValue>,
    origin: Option<SourcePositionId>,
) -> ExsValue {
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
        return time_error(
            "Host::date_time_from_components expects seven Int components and an IANA time-zone name",
            ExsValue::List(arguments),
            origin,
        );
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
        return date_time_error(
            "DateTime component is outside the supported range".to_owned(),
            origin,
        );
    };
    let civil = match CivilDateTime::new(year, month, day, hour, minute, second, nanosecond) {
        Ok(civil) => civil,
        Err(error) => return date_time_error(error.to_string(), origin),
    };
    match civil.in_tz(timezone) {
        Ok(value) => date_time_value(value),
        Err(error) => date_time_error(error.to_string(), origin),
    }
}

/// Encodes one Jiff Zoned value as the standard prelude DateTime object.
pub(crate) fn date_time_value(value: Zoned) -> ExsValue {
    let timestamp = value.timestamp();
    date_time_value_from_parts(
        timestamp.as_second(),
        timestamp.subsec_nanosecond(),
        value.offset().seconds(),
        value
            .time_zone()
            .iana_name()
            .map_or(ExsValue::None, |name| ExsValue::String(name.to_owned())),
    )
}

/// Encodes normalized DateTime fields as the standard prelude DateTime object.
pub(crate) fn date_time_value_from_parts(
    seconds: i64,
    nanoseconds: i32,
    offset_seconds: i32,
    timezone: ExsValue,
) -> ExsValue {
    ExsValue::Object(vec![
        ("unix_seconds".to_owned(), ExsValue::Int(seconds)),
        (
            "nanoseconds".to_owned(),
            ExsValue::Int(i64::from(nanoseconds)),
        ),
        (
            "utc_offset_seconds".to_owned(),
            ExsValue::Int(i64::from(offset_seconds)),
        ),
        ("timezone".to_owned(), timezone),
    ])
}

/// Decodes the normalized instant fields required for one DateTime operation.
pub(crate) fn date_time_parts(value: &ExsValue) -> Result<(i64, i32), String> {
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

/// Validates one serialized Duration Object and returns normalized duration parts.
pub(crate) fn duration_parts(arguments: Vec<ExsValue>) -> Result<(u64, u32), String> {
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

/// Builds a recoverable Host sleep capability Error.
pub(crate) fn sleep_error(message: String, origin: Option<SourcePositionId>) -> ExsValue {
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

/// Builds a recoverable invalid-input Error for one built-in time operation.
pub(crate) fn time_error(
    message: &str,
    data: ExsValue,
    origin: Option<SourcePositionId>,
) -> ExsValue {
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

/// Builds a recoverable Error returned when one DateTime operation cannot resolve its request.
pub(crate) fn date_time_error(message: String, origin: Option<SourcePositionId>) -> ExsValue {
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
