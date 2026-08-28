//! Asynchronous Rust futures for the ExS Host ABI.

use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use exs_abi::{
    HOST_DATETIME_FROM_COMPONENTS_HOST_NAME, HOST_DATETIME_IN_TIMEZONE_HOST_NAME,
    HOST_ELAPSED_HOST_NAME, HOST_NOW_HOST_NAME, HOST_STREAM_NEXT_HOST_NAME,
    HOST_STREAM_OPEN_HOST_NAME, STANDARD_ITERATOR_STEP_TYPE_IDENTITY,
};

use crate::{ExsValue, begin_host_call, guest_error, take_host_response};

/// The result of advancing one guest-owned Host stream.
#[derive(Clone, Debug, PartialEq)]
pub enum IteratorStep {
    /// One stream value is available.
    Item(ExsValue),
    /// The stream has no remaining values.
    Done,
}

/// One runner-owned Host stream addressed through its opaque integer handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostStream {
    /// Handle returned by the runner's stream-open operation.
    handle: i64,
}

/// A wall-clock instant captured by a runner together with its observed local offset.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DateTime {
    /// Signed whole seconds from the Unix epoch.
    pub unix_seconds: i64,
    /// Normalized fractional component in the range `0..1_000_000_000`.
    pub nanoseconds: i32,
    /// UTC offset observed for this instant, in whole seconds.
    pub utc_offset_seconds: i32,
    /// Runner-resolved IANA time-zone identifier when one was available.
    pub timezone: Option<String>,
}

/// Starts one named host call from owned argument values and resolves to its decoded response.
#[must_use]
pub fn call(name: impl Into<String>, arguments: impl IntoIterator<Item = ExsValue>) -> HostCall {
    HostCall {
        name: name.into(),
        arguments: arguments.into_iter().collect(),
        call_id: None,
    }
}

/// Returns a runner-owned wall-clock snapshot with IANA zone metadata when available.
pub async fn now() -> Result<DateTime, ExsValue> {
    match call(HOST_NOW_HOST_NAME, []).await {
        error @ ExsValue::Error(_) => Err(error),
        value => date_time_from_value(value),
    }
}

/// Renders one instant in a runner-resolved IANA time zone.
pub async fn in_timezone(
    value: DateTime,
    timezone: impl Into<String>,
) -> Result<DateTime, ExsValue> {
    match call(
        HOST_DATETIME_IN_TIMEZONE_HOST_NAME,
        [date_time_value(&value), ExsValue::String(timezone.into())],
    )
    .await
    {
        error @ ExsValue::Error(_) => Err(error),
        value => date_time_from_value(value),
    }
}

/// Resolves civil date and time components in one runner-provided IANA zone.
#[allow(clippy::too_many_arguments)] // The Host ABI deliberately receives each civil component.
pub async fn from_components_in_timezone(
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    nanosecond: i64,
    timezone: impl Into<String>,
) -> Result<DateTime, ExsValue> {
    match call(
        HOST_DATETIME_FROM_COMPONENTS_HOST_NAME,
        [
            ExsValue::Int(year),
            ExsValue::Int(month),
            ExsValue::Int(day),
            ExsValue::Int(hour),
            ExsValue::Int(minute),
            ExsValue::Int(second),
            ExsValue::Int(nanosecond),
            ExsValue::String(timezone.into()),
        ],
    )
    .await
    {
        error @ ExsValue::Error(_) => Err(error),
        value => date_time_from_value(value),
    }
}

/// Encodes one typed guest DateTime as the standard runner DateTime object.
fn date_time_value(value: &DateTime) -> ExsValue {
    ExsValue::Object(vec![
        ("unix_seconds".to_owned(), ExsValue::Int(value.unix_seconds)),
        (
            "nanoseconds".to_owned(),
            ExsValue::Int(i64::from(value.nanoseconds)),
        ),
        (
            "utc_offset_seconds".to_owned(),
            ExsValue::Int(i64::from(value.utc_offset_seconds)),
        ),
        (
            "timezone".to_owned(),
            value.timezone.as_ref().map_or(ExsValue::None, |timezone| {
                ExsValue::String(timezone.clone())
            }),
        ),
    ])
}

/// Returns monotonic time elapsed since this root guest execution began.
pub async fn elapsed() -> Result<core::time::Duration, ExsValue> {
    match call(HOST_ELAPSED_HOST_NAME, []).await {
        error @ ExsValue::Error(_) => Err(error),
        value => duration_from_value(value),
    }
}

/// Decodes one runner-provided DateTime object into the typed guest representation.
fn date_time_from_value(value: ExsValue) -> Result<DateTime, ExsValue> {
    let ExsValue::Object(entries) = value else {
        return Err(time_protocol_error(
            "Host::now returned a non-DateTime response",
            value,
        ));
    };
    let mut unix_seconds = None;
    let mut nanoseconds = None;
    let mut utc_offset_seconds = None;
    let mut timezone = None;
    for (key, value) in entries {
        match (key.as_str(), value) {
            ("unix_seconds", ExsValue::Int(value)) if unix_seconds.replace(value).is_none() => {}
            ("nanoseconds", ExsValue::Int(value)) if nanoseconds.replace(value).is_none() => {}
            ("utc_offset_seconds", ExsValue::Int(value))
                if utc_offset_seconds.replace(value).is_none() => {}
            ("timezone", ExsValue::String(value)) => {
                if timezone.is_some() {
                    return Err(time_protocol_error(
                        "Host::now returned an invalid DateTime response",
                        ExsValue::String(value),
                    ));
                }
                timezone = Some(Some(value));
            }
            ("timezone", ExsValue::None) => {
                if timezone.is_some() {
                    return Err(time_protocol_error(
                        "Host::now returned an invalid DateTime response",
                        ExsValue::None,
                    ));
                }
                timezone = Some(None);
            }
            (_, value) => {
                return Err(time_protocol_error(
                    "Host::now returned an invalid DateTime response",
                    value,
                ));
            }
        }
    }
    let (Some(unix_seconds), Some(nanoseconds), Some(utc_offset_seconds), Some(timezone)) =
        (unix_seconds, nanoseconds, utc_offset_seconds, timezone)
    else {
        return Err(time_protocol_error(
            "Host::now returned an incomplete DateTime response",
            ExsValue::None,
        ));
    };
    let Ok(nanoseconds) = i32::try_from(nanoseconds) else {
        return Err(time_protocol_error(
            "Host::now returned an invalid nanosecond value",
            ExsValue::Int(nanoseconds),
        ));
    };
    let Ok(utc_offset_seconds) = i32::try_from(utc_offset_seconds) else {
        return Err(time_protocol_error(
            "Host::now returned an invalid UTC offset",
            ExsValue::Int(utc_offset_seconds),
        ));
    };
    if !(0..1_000_000_000).contains(&nanoseconds) {
        return Err(time_protocol_error(
            "Host::now returned a non-normalized nanosecond value",
            ExsValue::Int(i64::from(nanoseconds)),
        ));
    }
    Ok(DateTime {
        unix_seconds,
        nanoseconds,
        utc_offset_seconds,
        timezone,
    })
}

/// Decodes one runner-provided Duration object into core's standard Duration.
fn duration_from_value(value: ExsValue) -> Result<core::time::Duration, ExsValue> {
    let ExsValue::Object(entries) = value else {
        return Err(time_protocol_error(
            "Host::elapsed returned a non-Duration response",
            value,
        ));
    };
    let mut seconds = None;
    let mut nanoseconds = None;
    for (key, value) in entries {
        match (key.as_str(), value) {
            ("seconds", ExsValue::Int(value)) if seconds.replace(value).is_none() => {}
            ("nanoseconds", ExsValue::Int(value)) if nanoseconds.replace(value).is_none() => {}
            (_, value) => {
                return Err(time_protocol_error(
                    "Host::elapsed returned an invalid Duration response",
                    value,
                ));
            }
        }
    }
    let (Some(seconds), Some(nanoseconds)) = (seconds, nanoseconds) else {
        return Err(time_protocol_error(
            "Host::elapsed returned an incomplete Duration response",
            ExsValue::None,
        ));
    };
    let Ok(seconds) = u64::try_from(seconds) else {
        return Err(time_protocol_error(
            "Host::elapsed returned a negative Duration",
            ExsValue::Int(seconds),
        ));
    };
    let Ok(nanoseconds) = u32::try_from(nanoseconds) else {
        return Err(time_protocol_error(
            "Host::elapsed returned an invalid nanosecond value",
            ExsValue::Int(nanoseconds),
        ));
    };
    if nanoseconds >= 1_000_000_000 {
        return Err(time_protocol_error(
            "Host::elapsed returned a non-normalized Duration",
            ExsValue::Int(i64::from(nanoseconds)),
        ));
    }
    Ok(core::time::Duration::new(seconds, nanoseconds))
}

/// Builds the fatal guest-boundary Error used for malformed runner time responses.
fn time_protocol_error(message: &str, value: ExsValue) -> ExsValue {
    let mut error = guest_error("InvalidHostTimeResponse", message);
    if let ExsValue::Error(ref mut error_value) = error {
        *error_value.data = value;
    }
    error
}

/// Opens one runner-registered Host stream from a name and ordered factory arguments.
pub async fn stream(
    name: impl Into<String>,
    arguments: impl IntoIterator<Item = ExsValue>,
) -> Result<HostStream, ExsValue> {
    let mut request = Vec::new();
    request.push(ExsValue::String(name.into()));
    request.extend(arguments);
    match call(HOST_STREAM_OPEN_HOST_NAME, request).await {
        ExsValue::Int(handle) => Ok(HostStream { handle }),
        error @ ExsValue::Error(_) => Err(error),
        value => Err(stream_protocol_error(
            "stream open returned a non-integer handle",
            value,
        )),
    }
}

impl HostStream {
    /// Advances this stream once and decodes its IteratorStep response.
    pub async fn next(&mut self) -> Result<IteratorStep, ExsValue> {
        match call(HOST_STREAM_NEXT_HOST_NAME, [ExsValue::Int(self.handle)]).await {
            error @ ExsValue::Error(_) => Err(error),
            ExsValue::Enum {
                type_id,
                variant,
                mut fields,
            } if type_id == STANDARD_ITERATOR_STEP_TYPE_IDENTITY && variant == "Item" => {
                if fields.len() == 1 {
                    Ok(IteratorStep::Item(fields.remove(0)))
                } else {
                    Err(stream_protocol_error(
                        "IteratorStep::Item must contain exactly one value",
                        ExsValue::List(fields),
                    ))
                }
            }
            ExsValue::Enum {
                type_id,
                variant,
                fields,
            } if type_id == STANDARD_ITERATOR_STEP_TYPE_IDENTITY && variant == "Done" => {
                if fields.is_empty() {
                    Ok(IteratorStep::Done)
                } else {
                    Err(stream_protocol_error(
                        "IteratorStep::Done must not contain values",
                        ExsValue::List(fields),
                    ))
                }
            }
            value => Err(stream_protocol_error(
                "stream next returned an invalid IteratorStep value",
                value,
            )),
        }
    }
}

/// Builds the fatal guest-boundary Error used for malformed runner stream responses.
fn stream_protocol_error(message: &str, value: ExsValue) -> ExsValue {
    let mut error = guest_error("InvalidHostStreamResponse", message);
    if let ExsValue::Error(ref mut error_value) = error {
        *error_value.data = value;
    }
    error
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
