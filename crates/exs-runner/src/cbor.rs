//! Canonical CBOR conversion at the runner-host function boundary.

use std::fmt;

use exs_abi::{CborError, ExsValue};

/// A CBOR failure at the host-function boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostCborError {
    /// The payload does not contain a valid ExS CBOR value.
    Invalid(CborError),
    /// A host-call request was valid CBOR but was not an ordered argument List.
    ArgumentsMustBeList,
}

impl fmt::Display for HostCborError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(error) => write!(formatter, "invalid ExS CBOR: {error}"),
            Self::ArgumentsMustBeList => {
                formatter.write_str("host-call arguments must be a CBOR List")
            }
        }
    }
}

impl std::error::Error for HostCborError {}

/// Decodes one canonical CBOR host request into ordered ExS arguments.
///
/// # Errors
///
/// Returns an error when the payload is not a valid ExS CBOR List.
pub fn decode_arguments(bytes: &[u8]) -> Result<Vec<ExsValue>, HostCborError> {
    match ExsValue::from_cbor(bytes).map_err(HostCborError::Invalid)? {
        ExsValue::List(arguments) => Ok(arguments),
        _ => Err(HostCborError::ArgumentsMustBeList),
    }
}

/// Encodes one host-function result for delivery back into the runtime.
///
/// # Errors
///
/// Returns an error when the ExS value cannot be represented by the current CBOR ABI.
pub fn encode_result(result: &ExsValue) -> Result<Vec<u8>, HostCborError> {
    result.to_cbor().map_err(HostCborError::Invalid)
}
