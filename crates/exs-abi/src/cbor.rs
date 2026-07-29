//! CBOR encoding for values that may cross the ExS Wasm-host ABI boundary.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use minicbor::{Decoder, Encoder, data::Type};

/// A host-safe ExS value represented independently of the runtime heap.
///
/// This representation contains only values that can safely cross the Wasm-host boundary. Runtime
/// references, object identities, and cycles are never exposed through this type.
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
    /// An ordered sequence of host-safe ExS values.
    List(Vec<ExsValue>),
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
        encode_value(self, &mut encoder)?;
        Ok(encoder.into_writer())
    }

    /// Decodes exactly one Phase-1 ExS CBOR value.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is malformed, unsupported, or contains trailing data.
    pub fn from_cbor(bytes: &[u8]) -> Result<Self, CborError> {
        let mut decoder = Decoder::new(bytes);
        let value = decode_value(&mut decoder)?;
        if decoder.position() != bytes.len() {
            return Err(CborError::TrailingData);
        }
        Ok(value)
    }
}

/// Encodes one host-safe ExS value into the current CBOR item position.
fn encode_value(value: &ExsValue, encoder: &mut Encoder<Vec<u8>>) -> Result<(), CborError> {
    match value {
        ExsValue::Null => encoder.null().map(|_| ()).map_err(|_| CborError::Encode),
        ExsValue::Bool(value) => encoder
            .bool(*value)
            .map(|_| ())
            .map_err(|_| CborError::Encode),
        ExsValue::Int(value) => encoder
            .i64(*value)
            .map(|_| ())
            .map_err(|_| CborError::Encode),
        ExsValue::Float(value) => encoder
            .f64(*value)
            .map(|_| ())
            .map_err(|_| CborError::Encode),
        ExsValue::String(value) => encoder
            .str(value)
            .map(|_| ())
            .map_err(|_| CborError::Encode),
        ExsValue::List(values) => {
            let length = u64::try_from(values.len()).map_err(|_| CborError::Encode)?;
            encoder.array(length).map_err(|_| CborError::Encode)?;
            for value in values {
                encode_value(value, encoder)?;
            }
            Ok(())
        }
    }
}

/// Decodes one host-safe ExS value from the current CBOR item position.
fn decode_value(decoder: &mut Decoder<'_>) -> Result<ExsValue, CborError> {
    match decoder.datatype().map_err(|_| CborError::Malformed)? {
        Type::Null => {
            decoder.null().map_err(|_| CborError::Malformed)?;
            Ok(ExsValue::Null)
        }
        Type::Bool => Ok(ExsValue::Bool(
            decoder.bool().map_err(|_| CborError::Malformed)?,
        )),
        Type::U8
        | Type::U16
        | Type::U32
        | Type::U64
        | Type::I8
        | Type::I16
        | Type::I32
        | Type::I64
        | Type::Int => Ok(ExsValue::Int(
            decoder.i64().map_err(|_| CborError::Malformed)?,
        )),
        Type::F16 | Type::F32 | Type::F64 => Ok(ExsValue::Float(
            decoder.f64().map_err(|_| CborError::Malformed)?,
        )),
        Type::String => Ok(ExsValue::String(
            decoder.str().map_err(|_| CborError::Malformed)?.into(),
        )),
        Type::Array => {
            let length = decoder
                .array()
                .map_err(|_| CborError::Malformed)?
                .ok_or(CborError::UnsupportedType)?;
            let capacity = usize::try_from(length).map_err(|_| CborError::Malformed)?;
            let mut values = Vec::with_capacity(capacity);
            for _ in 0..length {
                values.push(decode_value(decoder)?);
            }
            Ok(ExsValue::List(values))
        }
        _ => Err(CborError::UnsupportedType),
    }
}
