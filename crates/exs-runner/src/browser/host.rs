use super::bindings::{browser_host_elapsed, browser_host_now, browser_host_sleep};
use super::*;

/// Starts one Rust browser host function and converts its result into the JavaScript bridge form.
pub(super) fn start_host_call(
    registry: &BrowserHostFunctionRegistry,
    execution_id: u32,
    name: &str,
    arguments: &[u8],
    source_position: i32,
    execution_started_at: f64,
) -> JsValue {
    let arguments = match decode_arguments(arguments) {
        Ok(arguments) => arguments,
        Err(error) => {
            return rejected_browser_value(&format!("invalid host-call request: {error}"));
        }
    };
    let origin = u32::try_from(source_position).ok().map(SourcePositionId);
    let call = if name == HOST_SLEEP_HOST_NAME {
        Ok(start_host_sleep(arguments, origin))
    } else if name == HOST_NOW_HOST_NAME {
        Ok(start_host_now(arguments, origin))
    } else if name == HOST_ELAPSED_HOST_NAME {
        Ok(start_host_elapsed(arguments, execution_started_at, origin))
    } else if name == HOST_DATETIME_IN_TIMEZONE_HOST_NAME {
        Ok(start_host_datetime_in_timezone(arguments, origin))
    } else if name == HOST_DATETIME_FROM_COMPONENTS_HOST_NAME {
        Ok(start_host_datetime_from_components(arguments, origin))
    } else if name == HOST_STREAM_OPEN_HOST_NAME {
        Ok(registry.open_stream(execution_id, arguments, origin))
    } else if name == HOST_STREAM_NEXT_HOST_NAME {
        Ok(registry.start_stream_next(execution_id, arguments, origin))
    } else {
        registry.start(name, arguments)
    };
    match call {
        Ok(BrowserHostCall::Ready(value)) => match browser_response(&value) {
            Ok(value) => value,
            Err(error) => rejected_browser_value(&error),
        },
        Ok(BrowserHostCall::Pending(future)) => future_to_promise(async move {
            browser_response(&future.await).map_err(|error| JsError::new(&error).into())
        })
        .into(),
        Ok(BrowserHostCall::StreamPending { future, stream_id }) => {
            let registry = registry.clone();
            future_to_promise(async move {
                let item = future.await;
                registry.complete_stream_next(execution_id, stream_id, &item);
                let value = match item {
                    BrowserHostStreamItem::Item(value) => ExsValue::Enum {
                        type_id: STANDARD_ITERATOR_STEP_TYPE_IDENTITY.to_owned(),
                        variant: "Item".to_owned(),
                        fields: vec![value],
                    },
                    BrowserHostStreamItem::End => ExsValue::Enum {
                        type_id: STANDARD_ITERATOR_STEP_TYPE_IDENTITY.to_owned(),
                        variant: "Done".to_owned(),
                        fields: Vec::new(),
                    },
                };
                browser_response(&value).map_err(|error| JsError::new(&error).into())
            })
            .into()
        }
        Err(BrowserRegistryError::UnknownName(name)) => {
            match browser_response(&missing_host_error(name, origin)) {
                Ok(value) => value,
                Err(error) => rejected_browser_value(&error),
            }
        }
        Err(error) => rejected_browser_value(&error.to_string()),
    }
}

/// Starts one browser Host sleep Promise after validating its serialized Duration argument.
fn start_host_sleep(arguments: Vec<ExsValue>, origin: Option<SourcePositionId>) -> BrowserHostCall {
    let (seconds, nanoseconds) = match duration_parts(arguments) {
        Ok(parts) => parts,
        Err(message) => return BrowserHostCall::Ready(sleep_error(message, origin)),
    };
    match browser_host_sleep(seconds, nanoseconds) {
        Ok(promise) => BrowserHostCall::Pending(Box::pin(async move {
            match JsFuture::from(promise).await {
                Ok(_) => ExsValue::None,
                Err(error) => sleep_error(format!("Host sleep failed: {error:?}"), origin),
            }
        })),
        Err(error) => BrowserHostCall::Ready(sleep_error(
            format!("could not start Host sleep: {error:?}"),
            origin,
        )),
    }
}

/// Returns one browser wall-clock snapshot encoded as a standard DateTime object.
fn start_host_now(arguments: Vec<ExsValue>, origin: Option<SourcePositionId>) -> BrowserHostCall {
    if !arguments.is_empty() {
        return BrowserHostCall::Ready(time_error(
            "Host::now expects no arguments",
            ExsValue::List(arguments),
            origin,
        ));
    }
    let values = Array::from(&browser_host_now());
    if values.length() != 4 {
        return BrowserHostCall::Ready(invalid_time_response_error(origin));
    }
    let seconds = values.get(0);
    let nanoseconds = values.get(1);
    let offset = values.get(2);
    let timezone = values.get(3);
    let Some(seconds) = javascript_integer(&seconds) else {
        return BrowserHostCall::Ready(invalid_time_response_error(origin));
    };
    let Some(nanoseconds) = javascript_integer(&nanoseconds) else {
        return BrowserHostCall::Ready(invalid_time_response_error(origin));
    };
    let Some(offset) = javascript_integer(&offset) else {
        return BrowserHostCall::Ready(invalid_time_response_error(origin));
    };
    let Ok(nanoseconds) = i32::try_from(nanoseconds) else {
        return BrowserHostCall::Ready(invalid_time_response_error(origin));
    };
    let Ok(offset) = i32::try_from(offset) else {
        return BrowserHostCall::Ready(invalid_time_response_error(origin));
    };
    if !(0..1_000_000_000).contains(&nanoseconds) {
        return BrowserHostCall::Ready(invalid_time_response_error(origin));
    }
    let timezone = if timezone.is_null() || timezone.is_undefined() {
        ExsValue::None
    } else if let Some(timezone) = timezone.as_string() {
        ExsValue::String(timezone)
    } else {
        return BrowserHostCall::Ready(invalid_time_response_error(origin));
    };
    let response = ExsValue::Object(vec![
        ("unix_seconds".to_owned(), ExsValue::Int(seconds)),
        (
            "nanoseconds".to_owned(),
            ExsValue::Int(i64::from(nanoseconds)),
        ),
        (
            "utc_offset_seconds".to_owned(),
            ExsValue::Int(i64::from(offset)),
        ),
        ("timezone".to_owned(), timezone),
    ]);
    BrowserHostCall::Ready(response)
}

/// Renders one serialized DateTime instant in the requested bundled IANA time zone.
fn start_host_datetime_in_timezone(
    arguments: Vec<ExsValue>,
    origin: Option<SourcePositionId>,
) -> BrowserHostCall {
    let [value, ExsValue::String(timezone)] = arguments.as_slice() else {
        return BrowserHostCall::Ready(time_error(
            "Host::date_time_in_timezone expects a DateTime and IANA time-zone name",
            ExsValue::List(arguments),
            origin,
        ));
    };
    let (seconds, nanoseconds) = match date_time_parts(value) {
        Ok(parts) => parts,
        Err(message) => {
            return BrowserHostCall::Ready(time_error(&message, value.clone(), origin));
        }
    };
    let timestamp = match Timestamp::new(seconds, nanoseconds) {
        Ok(timestamp) => timestamp,
        Err(error) => return BrowserHostCall::Ready(date_time_error(error.to_string(), origin)),
    };
    match timestamp.in_tz(timezone) {
        Ok(value) => BrowserHostCall::Ready(date_time_value(value)),
        Err(error) => BrowserHostCall::Ready(date_time_error(error.to_string(), origin)),
    }
}

/// Resolves civil components in the requested bundled IANA time zone with compatible DST rules.
fn start_host_datetime_from_components(
    arguments: Vec<ExsValue>,
    origin: Option<SourcePositionId>,
) -> BrowserHostCall {
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
        return BrowserHostCall::Ready(time_error(
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
        return BrowserHostCall::Ready(date_time_error(
            "DateTime component is outside the supported range".to_owned(),
            origin,
        ));
    };
    let civil = match CivilDateTime::new(year, month, day, hour, minute, second, nanosecond) {
        Ok(civil) => civil,
        Err(error) => return BrowserHostCall::Ready(date_time_error(error.to_string(), origin)),
    };
    match civil.in_tz(timezone) {
        Ok(value) => BrowserHostCall::Ready(date_time_value(value)),
        Err(error) => BrowserHostCall::Ready(date_time_error(error.to_string(), origin)),
    }
}

/// Encodes one Jiff zoned value as the standard DateTime object returned to ExS.
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

/// Decodes the normalized instant fields needed by a browser DateTime zone conversion.
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

/// Returns one browser monotonic elapsed duration encoded as a standard Duration object.
fn start_host_elapsed(
    arguments: Vec<ExsValue>,
    execution_started_at: f64,
    origin: Option<SourcePositionId>,
) -> BrowserHostCall {
    if !arguments.is_empty() {
        return BrowserHostCall::Ready(time_error(
            "Host::elapsed expects no arguments",
            ExsValue::List(arguments),
            origin,
        ));
    }
    let values = Array::from(&browser_host_elapsed(execution_started_at));
    if values.length() != 2 {
        return BrowserHostCall::Ready(invalid_time_response_error(origin));
    }
    let seconds = values.get(0);
    let nanoseconds = values.get(1);
    let (Some(seconds), Some(nanoseconds)) = (
        javascript_integer(&seconds),
        javascript_integer(&nanoseconds),
    ) else {
        return BrowserHostCall::Ready(invalid_time_response_error(origin));
    };
    let Ok(nanoseconds) = i32::try_from(nanoseconds) else {
        return BrowserHostCall::Ready(invalid_time_response_error(origin));
    };
    if seconds < 0 || !(0..1_000_000_000).contains(&nanoseconds) {
        return BrowserHostCall::Ready(invalid_time_response_error(origin));
    }
    BrowserHostCall::Ready(ExsValue::Object(vec![
        ("seconds".to_owned(), ExsValue::Int(seconds)),
        (
            "nanoseconds".to_owned(),
            ExsValue::Int(i64::from(nanoseconds)),
        ),
    ]))
}

/// Converts one JavaScript finite integer into the Host ABI signed integer domain.
fn javascript_integer(value: &JsValue) -> Option<i64> {
    let value = value.as_f64()?;
    if value.is_finite()
        && value.fract() == 0.0
        && value >= i64::MIN as f64
        && value <= i64::MAX as f64
    {
        Some(value as i64)
    } else {
        None
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

/// Builds one recoverable invalid-input Error for a built-in time operation.
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

/// Builds one recoverable Error for a malformed browser time-provider response.
fn invalid_time_response_error(origin: Option<SourcePositionId>) -> ExsValue {
    ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: "HostTimeError".to_owned(),
        message: "browser time provider returned an invalid response".to_owned(),
        data: Box::new(ExsValue::None),
        origin,
        trace: Vec::new(),
        cause: None,
    })
}

/// Builds one recoverable Error returned when a browser zone operation cannot be resolved.
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

/// Builds one recoverable browser Host stream error.
fn stream_error(
    kind: &str,
    message: String,
    data: ExsValue,
    origin: Option<SourcePositionId>,
) -> ExsValue {
    ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: kind.to_owned(),
        message,
        data: Box::new(data),
        origin,
        trace: Vec::new(),
        cause: None,
    })
}

/// Builds the Error returned when Host::stream omits its registered stream name.
pub(super) fn stream_name_missing_error(origin: Option<SourcePositionId>) -> ExsValue {
    stream_error(
        "TypeError",
        "Host::stream expects a stream name as its first argument".to_owned(),
        ExsValue::None,
        origin,
    )
}

/// Builds the Error returned when Host::stream receives a non-String stream name.
pub(super) fn stream_name_error(value: ExsValue, origin: Option<SourcePositionId>) -> ExsValue {
    stream_error(
        "TypeError",
        "Host::stream expects a String stream name".to_owned(),
        value,
        origin,
    )
}

/// Builds the Error returned when a stream advance omits its handle.
pub(super) fn stream_handle_missing_error(origin: Option<SourcePositionId>) -> ExsValue {
    stream_error(
        "TypeError",
        "stream next expects a stream handle".to_owned(),
        ExsValue::None,
        origin,
    )
}

/// Builds the Error returned when a stream advance receives a non-Int handle.
pub(super) fn stream_handle_error(value: ExsValue, origin: Option<SourcePositionId>) -> ExsValue {
    stream_error(
        "TypeError",
        "stream handle must be an Int".to_owned(),
        value,
        origin,
    )
}

/// Builds the Error returned when a stream already has a pending advance.
pub(super) fn stream_busy_error(stream_id: i64, origin: Option<SourcePositionId>) -> ExsValue {
    stream_error(
        "StreamBusy",
        format!("stream handle `{stream_id}` already has a pending next call"),
        ExsValue::Int(stream_id),
        origin,
    )
}

/// Builds the Error returned when a stream handle is no longer active.
pub(super) fn invalid_stream_handle_error(
    stream_id: i64,
    origin: Option<SourcePositionId>,
) -> ExsValue {
    stream_error(
        "InvalidStreamHandle",
        format!("stream handle `{stream_id}` is not open"),
        ExsValue::Int(stream_id),
        origin,
    )
}

/// Builds the Error returned for an unregistered stream factory.
pub(super) fn missing_stream_error(name: &str, origin: Option<SourcePositionId>) -> ExsValue {
    stream_error(
        "HostFunctionNotFound",
        format!("unknown host stream `{name}`"),
        ExsValue::None,
        origin,
    )
}

/// Encodes one host response into the byte-array value expected by the JavaScript bridge.
fn browser_response(value: &ExsValue) -> Result<JsValue, String> {
    let bytes = encode_result(value).map_err(host_cbor_error)?;
    Ok(Uint8Array::from(bytes.as_slice()).into())
}

/// Returns one rejected Promise used to surface a technical browser bridge failure.
pub(super) fn rejected_browser_value(message: &str) -> JsValue {
    Promise::reject(&JsError::new(message)).into()
}

/// Converts one host-boundary CBOR error into a browser bridge error message.
fn host_cbor_error(error: HostCborError) -> String {
    format!("could not encode host response: {error}")
}

/// Builds the recoverable language value used for an unregistered browser host name.
fn missing_host_error(name: String, origin: Option<SourcePositionId>) -> ExsValue {
    ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: "HostFunctionNotFound".to_owned(),
        message: format!("Host function `{name}` is not registered."),
        data: Box::new(ExsValue::None),
        origin,
        trace: Vec::new(),
        cause: None,
    })
}
