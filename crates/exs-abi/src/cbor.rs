//! CBOR encoding for values that may cross the ExS Wasm-host ABI boundary.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use minicbor::{Decoder, Encoder, data::Type};

/// A host-safe ExS value represented independently of the runtime heap.
///
/// Phase 1 supports only primitive values. Future language values are added here when their CBOR
/// representation and host-facing semantics are defined.
#[derive(Clone, Debug, PartialEq)]
pub enum ExsValue {
    /// The singular ExS null value.
    Null,
    /// An ExS boolean.
    Bool(bool),
    /// An ExS signed integer.
    Int(i64),
    /// An ExS IEEE 754 binary64 floating-point number.
    Float(f64),
    /// An immutable UTF-8 ExS string.
    String(String),
}

/// A malformed or unsupported ExS CBOR value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CborError {
    /// The input does not contain a well-formed CBOR item.
    Malformed,
    /// The CBOR item type is not supported by the current ExS ABI.
    UnsupportedType,
    /// The input contains bytes after the one expected CBOR item.
    TrailingData,
    /// Encoding into the destination buffer unexpectedly failed.
    Encode,
}

impl fmt::Display for CborError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => formatter.write_str("malformed CBOR value"),
            Self::UnsupportedType => formatter.write_str("unsupported ExS CBOR value type"),
            Self::TrailingData => formatter.write_str("CBOR value has trailing data"),
            Self::Encode => formatter.write_str("could not encode CBOR value"),
        }
    }
}

impl ExsValue {
    /// Encodes this value as one CBOR item.
    ///
    /// # Errors
    ///
    /// Returns an error only if writing to the owned output buffer unexpectedly fails.
    pub fn to_cbor(&self) -> Result<Vec<u8>, CborError> {
        let mut encoder = Encoder::new(Vec::new());
        let encoded = match self {
            Self::Null => encoder.null(),
            Self::Bool(value) => encoder.bool(*value),
            Self::Int(value) => encoder.i64(*value),
            Self::Float(value) => encoder.f64(*value),
            Self::String(value) => encoder.str(value),
        };
        encoded.map_err(|_| CborError::Encode)?;
        Ok(encoder.into_writer())
    }

    /// Decodes exactly one Phase-1 ExS CBOR value.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is malformed, unsupported, or contains trailing data.
    pub fn from_cbor(bytes: &[u8]) -> Result<Self, CborError> {
        let mut decoder = Decoder::new(bytes);
        let value = match decoder.datatype().map_err(|_| CborError::Malformed)? {
            Type::Null => {
                decoder.null().map_err(|_| CborError::Malformed)?;
                Self::Null
            }
            Type::Bool => Self::Bool(decoder.bool().map_err(|_| CborError::Malformed)?),
            Type::U8
            | Type::U16
            | Type::U32
            | Type::U64
            | Type::I8
            | Type::I16
            | Type::I32
            | Type::I64
            | Type::Int => Self::Int(decoder.i64().map_err(|_| CborError::Malformed)?),
            Type::F16 | Type::F32 | Type::F64 => {
                Self::Float(decoder.f64().map_err(|_| CborError::Malformed)?)
            }
            Type::String => Self::String(decoder.str().map_err(|_| CborError::Malformed)?.into()),
            _ => return Err(CborError::UnsupportedType),
        };
        if decoder.position() != bytes.len() {
            return Err(CborError::TrailingData);
        }
        Ok(value)
    }
}
