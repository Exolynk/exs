use super::bindings::{browser_host_elapsed, browser_host_now, browser_host_sleep};
use super::*;
use crate::host_values::{
    date_time_from_components, date_time_in_timezone, date_time_value_from_parts, duration_parts,
    duration_value, sleep_error, time_error,
};
use exs_abi::{BuiltinHostOperation, builtin_host_operation};

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
    let call = match builtin_host_operation(name) {
        Some(BuiltinHostOperation::Sleep) => Ok(start_host_sleep(arguments, origin)),
        Some(BuiltinHostOperation::Now) => Ok(start_host_now(arguments, origin)),
        Some(BuiltinHostOperation::Elapsed) => {
            Ok(start_host_elapsed(arguments, execution_started_at, origin))
        }
        Some(BuiltinHostOperation::DateTimeInTimezone) => Ok(BrowserHostCall::Ready(
            date_time_in_timezone(arguments, origin),
        )),
        Some(BuiltinHostOperation::DateTimeFromComponents) => Ok(BrowserHostCall::Ready(
            date_time_from_components(arguments, origin),
        )),
        Some(BuiltinHostOperation::StreamOpen) => {
            Ok(registry.open_stream(execution_id, arguments, origin))
        }
        Some(BuiltinHostOperation::StreamNext) => {
            Ok(registry.start_stream_next(execution_id, arguments, origin))
        }
        None => registry.start(name, arguments),
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
    BrowserHostCall::Ready(date_time_value_from_parts(
        seconds,
        nanoseconds,
        offset,
        timezone,
    ))
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
    BrowserHostCall::Ready(duration_value(std::time::Duration::new(
        seconds as u64,
        nanoseconds as u32,
    )))
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
