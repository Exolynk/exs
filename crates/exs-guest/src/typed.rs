//! Conversions between Rust types and [`ExsValue`] for guests with the `serde` feature.

use alloc::string::ToString;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{ExsError, ExsValue};

/// Converts an input or a host response into `T`. A value that is an Error stays that Error, so a
/// failed host call reports its own failure:
///
/// ```no_run
/// use exs_guest::{ExsError, ExsValue, decode, encode, host};
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Deserialize)]
/// struct Order {
///     id: String,
/// }
///
/// #[derive(Deserialize, Serialize)]
/// struct Price {
///     cents: i64,
/// }
///
/// async fn price(inputs: Vec<ExsValue>) -> Result<ExsValue, ExsError> {
///     let input = inputs.into_iter().next().unwrap_or(ExsValue::None);
///     let order: Order = decode(input)?;
///     let id = ExsValue::String(order.id);
///     let price: Price = decode(host::call("price", [id]).await)?;
///     encode(&price)
/// }
/// ```
///
/// # Errors
///
/// Returns the Error `value` holds, or a recoverable `WireDecodeError` when it does not fit `T`.
pub fn decode<T: DeserializeOwned>(value: ExsValue) -> Result<T, ExsError> {
    if let ExsValue::Error(error) = value {
        return Err(error);
    }
    value
        .into_deserialize()
        .map_err(|error| ExsError::recoverable("WireDecodeError", error.to_string()))
}

/// Converts `value` into the [`ExsValue`] it crosses the boundary as.
///
/// # Errors
///
/// Returns a recoverable `WireEncodeError` when `value` has no ExS form, such as a map with keys
/// that are not strings.
pub fn encode<T: Serialize>(value: &T) -> Result<ExsValue, ExsError> {
    ExsValue::from_serialize(value)
        .map_err(|error| ExsError::recoverable("WireEncodeError", error.to_string()))
}
