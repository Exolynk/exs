//! Shared typed host-registry conversion at the `ExsValue` boundary.

use exs_abi::{ErrorSeverity, ExsError, ExsValue};
use serde::de::DeserializeOwned;

/// Decodes the zero-or-one argument contract accepted by typed host functions.
pub(crate) fn decode_typed_request<Request: DeserializeOwned>(
    arguments: Vec<ExsValue>,
) -> Result<Request, ExsValue> {
    let value = match arguments.as_slice() {
        [] => ExsValue::None,
        [value] => value.clone(),
        values => {
            return Err(typed_decode_error(format!(
                "typed host functions expect zero or one argument, received {}",
                values.len()
            )));
        }
    };
    value
        .into_deserialize()
        .map_err(|error| typed_decode_error(error.to_string()))
}

/// Builds one recoverable language error for typed host request decoding.
pub(crate) fn typed_decode_error(message: String) -> ExsValue {
    typed_error("WireDecodeError", message)
}

/// Builds one recoverable language error for typed host response encoding.
pub(crate) fn typed_encode_error(message: String) -> ExsValue {
    typed_error("WireEncodeError", message)
}

/// Builds one recoverable error emitted by a typed host adapter.
fn typed_error(kind: &str, message: String) -> ExsValue {
    ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: kind.to_owned(),
        message,
        data: Box::new(ExsValue::None),
        origin: None,
        trace: Vec::new(),
        cause: None,
    })
}
