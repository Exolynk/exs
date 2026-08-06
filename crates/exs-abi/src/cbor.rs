//! CBOR encoding for values that may cross the ExS Wasm-host ABI boundary.

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use minicbor::{
    Decoder, Encoder,
    data::{Tag, Type},
};

/// Stable CBOR tag for an ExS structured Error value.
const ERROR_TAG: u64 = 60_001;
/// Stable CBOR tag for an ExS nominal enum value.
const ENUM_TAG: u64 = 60_005;
/// Default maximum size of one decoded CBOR payload.
const DEFAULT_MAX_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;
/// Default maximum recursive depth of one decoded CBOR value.
const DEFAULT_MAX_NESTING: usize = 64;
/// Default maximum direct entries in one decoded CBOR collection.
const DEFAULT_MAX_COLLECTION_ENTRIES: usize = 65_536;

/// A host-safe ExS value represented independently of the runtime heap.
///
/// This representation contains only values that can safely cross the Wasm-host boundary. Runtime
/// references, object identities, and cycles are never exposed through this type.
#[derive(Clone, Debug, PartialEq)]
pub enum ExsValue {
    /// The absence variant shared by ExS Options and empty operations.
    None,
    /// A structured ExS language error.
    Error(ExsError),
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
    /// An insertion-ordered mapping from string keys to host-safe ExS values.
    Object(Vec<(String, ExsValue)>),
    /// A nominal enum value with an opaque type identity and ordered variant fields.
    Enum {
        /// Resolver-derived nominal enum identity.
        type_id: String,
        /// Source-visible enum variant name.
        variant: String,
        /// Variant payload fields in declaration order.
        fields: Vec<ExsValue>,
    },
}

/// The severity of one ExS language Error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorSeverity {
    /// A language error from which execution may safely continue.
    Recoverable,
    /// A fault that terminates the current execution context.
    Fatal,
}

/// One source position assigned by the compiler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourcePositionId(pub u32);

/// One language-level call frame stored in an Error trace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExsStackFrame {
    /// Compiler-assigned source function identifier.
    pub function_id: u32,
    /// Compiler-assigned source position for the call site.
    pub call_site: SourcePositionId,
}

/// A host-safe ExS language Error.
#[derive(Clone, Debug, PartialEq)]
pub struct ExsError {
    /// Whether execution may continue after this Error.
    pub severity: ErrorSeverity,
    /// Stable machine-readable error kind.
    pub kind: String,
    /// Human-readable error message.
    pub message: String,
    /// Additional language data associated with the Error.
    pub data: Box<ExsValue>,
    /// Compiler-assigned source position that created the Error.
    pub origin: Option<SourcePositionId>,
    /// Language-level call frames from innermost to outermost.
    pub trace: Vec<ExsStackFrame>,
    /// An optional prior Error or related language value.
    pub cause: Option<Box<ExsValue>>,
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
    /// The value exceeded the configured recursive nesting limit.
    NestingLimitExceeded,
    /// The value exceeded the configured collection-entry limit.
    CollectionLimitExceeded,
    /// The encoded payload exceeded the configured byte limit.
    PayloadLimitExceeded,
    /// The decoder could not reserve memory for a validated collection.
    AllocationFailed,
}

impl fmt::Display for CborError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => formatter.write_str("malformed CBOR value"),
            Self::UnsupportedType => formatter.write_str("unsupported ExS CBOR value type"),
            Self::TrailingData => formatter.write_str("CBOR value has trailing data"),
            Self::Encode => formatter.write_str("could not encode CBOR value"),
            Self::NestingLimitExceeded => formatter.write_str("CBOR nesting limit exceeded"),
            Self::CollectionLimitExceeded => {
                formatter.write_str("CBOR collection-entry limit exceeded")
            }
            Self::PayloadLimitExceeded => formatter.write_str("CBOR payload limit exceeded"),
            Self::AllocationFailed => formatter.write_str("could not allocate decoded CBOR value"),
        }
    }
}

/// Structural limits applied while encoding or decoding, and a payload limit while decoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CborLimits {
    /// Maximum encoded bytes accepted for one CBOR value.
    pub max_payload_bytes: usize,
    /// Maximum recursive value depth, including the root value.
    pub max_nesting: usize,
    /// Maximum direct entries in any List, Object, enum payload, or Error trace.
    pub max_collection_entries: usize,
}

impl CborLimits {
    /// Returns limits that explicitly disable every CBOR resource bound.
    #[must_use]
    pub const fn unrestricted() -> Self {
        Self {
            max_payload_bytes: usize::MAX,
            max_nesting: usize::MAX,
            max_collection_entries: usize::MAX,
        }
    }
}

impl Default for CborLimits {
    /// Creates conservative limits suitable for one untrusted host-boundary value.
    fn default() -> Self {
        Self {
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
            max_nesting: DEFAULT_MAX_NESTING,
            max_collection_entries: DEFAULT_MAX_COLLECTION_ENTRIES,
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
        self.to_cbor_with_limits(CborLimits::unrestricted())
    }

    /// Encodes this value as one CBOR item with structural resource limits.
    ///
    /// # Errors
    ///
    /// Returns an error when the value exceeds a structural limit or encoding fails.
    pub fn to_cbor_with_limits(&self, limits: CborLimits) -> Result<Vec<u8>, CborError> {
        let mut encoder = Encoder::new(Vec::new());
        encode_value(self, &mut encoder, limits, 1)?;
        Ok(encoder.into_writer())
    }

    /// Decodes exactly one Phase-1 ExS CBOR value with conservative resource limits.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is malformed, unsupported, contains trailing data, or
    /// exceeds a default limit.
    pub fn from_cbor(bytes: &[u8]) -> Result<Self, CborError> {
        Self::from_cbor_with_limits(bytes, CborLimits::default())
    }

    /// Decodes exactly one Phase-1 ExS CBOR value with structural resource limits.
    ///
    /// # Errors
    ///
    /// Returns an error when the input is malformed, unsupported, trailing, or exceeds a limit.
    pub fn from_cbor_with_limits(bytes: &[u8], limits: CborLimits) -> Result<Self, CborError> {
        if bytes.len() > limits.max_payload_bytes {
            return Err(CborError::PayloadLimitExceeded);
        }
        let mut decoder = Decoder::new(bytes);
        let value = decode_value(&mut decoder, bytes.len(), limits, 1)?;
        if decoder.position() != bytes.len() {
            return Err(CborError::TrailingData);
        }
        Ok(value)
    }
}

/// Encodes one host-safe ExS value into the current CBOR item position.
fn encode_value(
    value: &ExsValue,
    encoder: &mut Encoder<Vec<u8>>,
    limits: CborLimits,
    depth: usize,
) -> Result<(), CborError> {
    check_nesting(limits, depth)?;
    match value {
        ExsValue::None => encoder.null().map(|_| ()).map_err(|_| CborError::Encode),
        ExsValue::Error(error) => encode_error(error, encoder, limits, depth),
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
            check_collection_length(limits, values.len())?;
            let length = u64::try_from(values.len()).map_err(|_| CborError::Encode)?;
            encoder.array(length).map_err(|_| CborError::Encode)?;
            for value in values {
                encode_value(value, encoder, limits, next_depth(depth)?)?;
            }
            Ok(())
        }
        ExsValue::Object(entries) => {
            check_collection_length(limits, entries.len())?;
            let length = u64::try_from(entries.len()).map_err(|_| CborError::Encode)?;
            encoder.map(length).map_err(|_| CborError::Encode)?;
            for (key, value) in entries {
                encoder.str(key).map_err(|_| CborError::Encode)?;
                encode_value(value, encoder, limits, next_depth(depth)?)?;
            }
            Ok(())
        }
        ExsValue::Enum {
            type_id,
            variant,
            fields,
        } => {
            check_collection_length(limits, fields.len())?;
            encoder
                .tag(Tag::new(ENUM_TAG))
                .and_then(|encoder| encoder.array(3))
                .and_then(|encoder| encoder.str(type_id))
                .and_then(|encoder| encoder.str(variant))
                .and_then(|encoder| encoder.array(fields.len() as u64))
                .map_err(|_| CborError::Encode)?;
            for field in fields {
                encode_value(field, encoder, limits, next_depth(depth)?)?;
            }
            Ok(())
        }
    }
}

/// Decodes one host-safe ExS value from the current CBOR item position.
fn decode_value(
    decoder: &mut Decoder<'_>,
    input_length: usize,
    limits: CborLimits,
    depth: usize,
) -> Result<ExsValue, CborError> {
    check_nesting(limits, depth)?;
    match decoder.datatype().map_err(|_| CborError::Malformed)? {
        Type::Null => {
            decoder.null().map_err(|_| CborError::Malformed)?;
            Ok(ExsValue::None)
        }
        Type::Tag => {
            let tag = decoder.tag().map_err(|_| CborError::Malformed)?;
            match tag.as_u64() {
                ERROR_TAG => Ok(ExsValue::Error(decode_error(
                    decoder,
                    input_length,
                    limits,
                    depth,
                )?)),
                ENUM_TAG => decode_enum(decoder, input_length, limits, depth),
                _ => Err(CborError::UnsupportedType),
            }
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
            check_collection_length(limits, capacity)?;
            check_declared_entries(decoder, input_length, length, 1)?;
            let mut values = Vec::new();
            values
                .try_reserve_exact(capacity)
                .map_err(|_| CborError::AllocationFailed)?;
            for _ in 0..length {
                values.push(decode_value(
                    decoder,
                    input_length,
                    limits,
                    next_depth(depth)?,
                )?);
            }
            Ok(ExsValue::List(values))
        }
        Type::Map => {
            let length = decoder
                .map()
                .map_err(|_| CborError::Malformed)?
                .ok_or(CborError::UnsupportedType)?;
            let capacity = usize::try_from(length).map_err(|_| CborError::Malformed)?;
            check_collection_length(limits, capacity)?;
            check_declared_entries(decoder, input_length, length, 2)?;
            let mut entries = Vec::new();
            entries
                .try_reserve_exact(capacity)
                .map_err(|_| CborError::AllocationFailed)?;
            for _ in 0..length {
                let key = match decoder.datatype().map_err(|_| CborError::Malformed)? {
                    Type::String => decoder.str().map_err(|_| CborError::Malformed)?.into(),
                    _ => return Err(CborError::UnsupportedType),
                };
                let value = decode_value(decoder, input_length, limits, next_depth(depth)?)?;
                if entries.iter().any(|(entry_key, _)| entry_key == &key) {
                    return Err(CborError::Malformed);
                }
                entries.push((key, value));
            }
            Ok(ExsValue::Object(entries))
        }
        _ => Err(CborError::UnsupportedType),
    }
}

/// Decodes the fixed three-field CBOR representation of one enum value.
fn decode_enum(
    decoder: &mut Decoder<'_>,
    input_length: usize,
    limits: CborLimits,
    depth: usize,
) -> Result<ExsValue, CborError> {
    if decoder.array().map_err(|_| CborError::Malformed)? != Some(3) {
        return Err(CborError::Malformed);
    }
    let type_id = decoder
        .str()
        .map(str::to_owned)
        .map_err(|_| CborError::Malformed)?;
    let variant = decoder
        .str()
        .map(str::to_owned)
        .map_err(|_| CborError::Malformed)?;
    let length = decoder
        .array()
        .map_err(|_| CborError::Malformed)?
        .ok_or(CborError::UnsupportedType)?;
    let capacity = usize::try_from(length).map_err(|_| CborError::Malformed)?;
    check_collection_length(limits, capacity)?;
    check_declared_entries(decoder, input_length, length, 1)?;
    let mut fields = Vec::new();
    fields
        .try_reserve_exact(capacity)
        .map_err(|_| CborError::AllocationFailed)?;
    for _ in 0..length {
        fields.push(decode_value(
            decoder,
            input_length,
            limits,
            next_depth(depth)?,
        )?);
    }
    Ok(ExsValue::Enum {
        type_id,
        variant,
        fields,
    })
}

/// Encodes the stable seven-field CBOR map used for one ExS Error.
fn encode_error(
    error: &ExsError,
    encoder: &mut Encoder<Vec<u8>>,
    limits: CborLimits,
    depth: usize,
) -> Result<(), CborError> {
    check_collection_length(limits, error.trace.len())?;
    encoder
        .tag(Tag::new(ERROR_TAG))
        .and_then(|encoder| encoder.map(7))
        .map_err(|_| CborError::Encode)?;
    encoder
        .u8(0)
        .and_then(|encoder| {
            encoder.u8(match error.severity {
                ErrorSeverity::Recoverable => 0,
                ErrorSeverity::Fatal => 1,
            })
        })
        .and_then(|encoder| encoder.u8(1))
        .and_then(|encoder| encoder.str(&error.kind))
        .and_then(|encoder| encoder.u8(2))
        .and_then(|encoder| encoder.str(&error.message))
        .and_then(|encoder| encoder.u8(3))
        .map_err(|_| CborError::Encode)?;
    encode_value(&error.data, encoder, limits, next_depth(depth)?)?;
    encoder.u8(4).map_err(|_| CborError::Encode)?;
    match error.origin {
        Some(position) => encoder.u32(position.0),
        None => encoder.null(),
    }
    .map_err(|_| CborError::Encode)?;
    encoder
        .u8(5)
        .and_then(|encoder| encoder.array(error.trace.len() as u64))
        .map_err(|_| CborError::Encode)?;
    for frame in &error.trace {
        encoder
            .array(2)
            .and_then(|encoder| encoder.u32(frame.function_id))
            .and_then(|encoder| encoder.u32(frame.call_site.0))
            .map_err(|_| CborError::Encode)?;
    }
    encoder.u8(6).map_err(|_| CborError::Encode)?;
    match &error.cause {
        Some(cause) => encode_value(cause, encoder, limits, next_depth(depth)?),
        None => encoder.null().map(|_| ()).map_err(|_| CborError::Encode),
    }
}

/// Decodes the stable seven-field CBOR map used for one ExS Error.
fn decode_error(
    decoder: &mut Decoder<'_>,
    input_length: usize,
    limits: CborLimits,
    depth: usize,
) -> Result<ExsError, CborError> {
    let length = decoder
        .map()
        .map_err(|_| CborError::Malformed)?
        .ok_or(CborError::UnsupportedType)?;
    if length != 7 {
        return Err(CborError::Malformed);
    }
    let severity = decode_error_key(decoder, 0).and_then(|_| {
        match decoder.u8().map_err(|_| CborError::Malformed)? {
            0 => Ok(ErrorSeverity::Recoverable),
            1 => Ok(ErrorSeverity::Fatal),
            _ => Err(CborError::Malformed),
        }
    })?;
    let kind = decode_error_key(decoder, 1).and_then(|_| {
        decoder
            .str()
            .map(str::to_owned)
            .map_err(|_| CborError::Malformed)
    })?;
    let message = decode_error_key(decoder, 2).and_then(|_| {
        decoder
            .str()
            .map(str::to_owned)
            .map_err(|_| CborError::Malformed)
    })?;
    decode_error_key(decoder, 3)?;
    let data = Box::new(decode_value(
        decoder,
        input_length,
        limits,
        next_depth(depth)?,
    )?);
    decode_error_key(decoder, 4)?;
    let origin = if decoder.datatype().map_err(|_| CborError::Malformed)? == Type::Null {
        decoder.null().map_err(|_| CborError::Malformed)?;
        None
    } else {
        Some(SourcePositionId(
            decoder.u32().map_err(|_| CborError::Malformed)?,
        ))
    };
    decode_error_key(decoder, 5)?;
    let trace_length = decoder
        .array()
        .map_err(|_| CborError::Malformed)?
        .ok_or(CborError::UnsupportedType)?;
    let trace_capacity = usize::try_from(trace_length).map_err(|_| CborError::Malformed)?;
    check_collection_length(limits, trace_capacity)?;
    check_declared_entries(decoder, input_length, trace_length, 1)?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(trace_capacity)
        .map_err(|_| CborError::AllocationFailed)?;
    for _ in 0..trace_length {
        if decoder.array().map_err(|_| CborError::Malformed)? != Some(2) {
            return Err(CborError::Malformed);
        }
        trace.push(ExsStackFrame {
            function_id: decoder.u32().map_err(|_| CborError::Malformed)?,
            call_site: SourcePositionId(decoder.u32().map_err(|_| CborError::Malformed)?),
        });
    }
    decode_error_key(decoder, 6)?;
    let cause = if decoder.datatype().map_err(|_| CborError::Malformed)? == Type::Null {
        decoder.null().map_err(|_| CborError::Malformed)?;
        None
    } else {
        Some(Box::new(decode_value(
            decoder,
            input_length,
            limits,
            next_depth(depth)?,
        )?))
    };
    Ok(ExsError {
        severity,
        kind,
        message,
        data,
        origin,
        trace,
        cause,
    })
}

/// Reads and validates the next fixed Error-map field identifier.
fn decode_error_key(decoder: &mut Decoder<'_>, expected: u8) -> Result<(), CborError> {
    if decoder.u8().map_err(|_| CborError::Malformed)? == expected {
        Ok(())
    } else {
        Err(CborError::Malformed)
    }
}

/// Checks one recursive value depth before allocating or descending into its payload.
fn check_nesting(limits: CborLimits, depth: usize) -> Result<(), CborError> {
    if depth > limits.max_nesting {
        Err(CborError::NestingLimitExceeded)
    } else {
        Ok(())
    }
}

/// Produces one checked child-value depth.
fn next_depth(depth: usize) -> Result<usize, CborError> {
    depth.checked_add(1).ok_or(CborError::NestingLimitExceeded)
}

/// Checks a declared or constructed collection size before allocating its backing storage.
fn check_collection_length(limits: CborLimits, length: usize) -> Result<(), CborError> {
    if length > limits.max_collection_entries {
        Err(CborError::CollectionLimitExceeded)
    } else {
        Ok(())
    }
}

/// Rejects a collection header whose declared entries cannot fit in the remaining input.
fn check_declared_entries(
    decoder: &Decoder<'_>,
    input_length: usize,
    entries: u64,
    minimum_bytes_per_entry: usize,
) -> Result<(), CborError> {
    let remaining = input_length
        .checked_sub(decoder.position())
        .ok_or(CborError::Malformed)?;
    let maximum_entries = remaining / minimum_bytes_per_entry;
    if entries > maximum_entries as u64 {
        Err(CborError::Malformed)
    } else {
        Ok(())
    }
}
