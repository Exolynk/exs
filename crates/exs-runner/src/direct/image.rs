//! Writing values as images in the `exs_layout::image` format.
//!
//! The writer applies the runner's CBOR limits as if it encoded the value as CBOR: nesting,
//! collection entries and the byte size of the equivalent CBOR, so both boundaries accept the
//! same values.

use exs_abi::{CborError, CborLimits, ErrorSeverity, ExsError, ExsValue};
use exs_layout::image::{
    IMAGE_HEADER_SIZE, IMAGE_MAGIC, RECORD_BOOL, RECORD_BYTES, RECORD_ENUM, RECORD_ERROR,
    RECORD_FLOAT, RECORD_INT, RECORD_LIST, RECORD_NONE, RECORD_OBJECT, RECORD_STRING,
};

/// CBOR tag of an ExS Error.
const ERROR_TAG: u64 = 60_001;
/// CBOR tag of an ExS enum.
const ENUM_TAG: u64 = 60_005;

/// Returns `value` as an image, or the CBOR limit it exceeds.
///
/// # Errors
///
/// Returns the exceeded limit, or `CborError::Encode` for an image beyond 4 GiB.
pub(crate) fn write_image(value: &ExsValue, limits: CborLimits) -> Result<Vec<u8>, CborError> {
    let mut writer = Writer {
        bytes: Vec::with_capacity(64),
        cbor_len: 0,
        limits,
    };
    writer.bytes.extend_from_slice(&IMAGE_MAGIC.to_le_bytes());
    writer.bytes.extend_from_slice(&0_u32.to_le_bytes());
    debug_assert_eq!(writer.bytes.len(), IMAGE_HEADER_SIZE as usize);
    let root = writer.value(value, 1)?;
    writer.bytes[4..8].copy_from_slice(&root.to_le_bytes());
    Ok(writer.bytes)
}

/// Returns the byte length of a CBOR head with `argument`.
const fn head_len(argument: u64) -> usize {
    match argument {
        0..=23 => 1,
        24..=0xff => 2,
        0x100..=0xffff => 3,
        0x1_0000..=0xffff_ffff => 5,
        _ => 9,
    }
}

/// Returns the byte length of the CBOR encoding of the Int `value`.
const fn int_len(value: i64) -> usize {
    if value < 0 {
        head_len((-1 - value).cast_unsigned())
    } else {
        head_len(value.cast_unsigned())
    }
}

/// The image being written, with the size of the equivalent CBOR.
struct Writer {
    /// The image so far.
    bytes: Vec<u8>,
    /// Byte length of the equivalent CBOR so far.
    cbor_len: usize,
    /// The limits the value must keep.
    limits: CborLimits,
}

impl Writer {
    /// Accounts for `count` bytes of the equivalent CBOR.
    fn cbor(&mut self, count: usize) -> Result<(), CborError> {
        self.cbor_len = self.cbor_len.saturating_add(count);
        if self.cbor_len > self.limits.max_payload_bytes {
            return Err(CborError::PayloadLimitExceeded);
        }
        Ok(())
    }

    /// Checks one collection's entry count.
    fn collection(&self, count: usize) -> Result<(), CborError> {
        if count > self.limits.max_collection_entries {
            return Err(CborError::CollectionLimitExceeded);
        }
        Ok(())
    }

    /// Returns the offset the next record starts at.
    fn offset(&self) -> Result<u32, CborError> {
        u32::try_from(self.bytes.len()).map_err(|_| CborError::Encode)
    }

    /// Starts a record of `kind` with `argument` and returns its offset.
    fn record(&mut self, kind: u32, argument: usize) -> Result<u32, CborError> {
        let offset = self.offset()?;
        let argument = u32::try_from(argument).map_err(|_| CborError::Encode)?;
        self.bytes.extend_from_slice(&kind.to_le_bytes());
        self.bytes.extend_from_slice(&argument.to_le_bytes());
        Ok(offset)
    }

    /// Appends `bytes` and pads the image to 8 bytes.
    fn payload(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
        self.bytes.resize(self.bytes.len().next_multiple_of(8), 0);
    }

    /// Appends the `u32` words and pads the image to 8 bytes.
    fn words(&mut self, words: &[u32]) {
        for word in words {
            self.bytes.extend_from_slice(&word.to_le_bytes());
        }
        self.bytes.resize(self.bytes.len().next_multiple_of(8), 0);
    }

    /// Writes a String record and returns its offset.
    fn string(&mut self, text: &str) -> Result<u32, CborError> {
        self.cbor(head_len(text.len() as u64) + text.len())?;
        let offset = self.record(RECORD_STRING, text.len())?;
        let chars = u32::try_from(text.chars().count()).map_err(|_| CborError::Encode)?;
        self.bytes.extend_from_slice(&chars.to_le_bytes());
        self.bytes.extend_from_slice(&0_u32.to_le_bytes());
        self.payload(text.as_bytes());
        Ok(offset)
    }

    /// Writes the records of `value` at nesting depth `depth` and returns its offset.
    fn value(&mut self, value: &ExsValue, depth: usize) -> Result<u32, CborError> {
        if depth > self.limits.max_nesting {
            return Err(CborError::NestingLimitExceeded);
        }
        match value {
            ExsValue::None => {
                self.cbor(1)?;
                self.record(RECORD_NONE, 0)
            }
            ExsValue::Bool(flag) => {
                self.cbor(1)?;
                self.record(RECORD_BOOL, usize::from(*flag))
            }
            ExsValue::Int(number) => {
                self.cbor(int_len(*number))?;
                let offset = self.record(RECORD_INT, 0)?;
                self.payload(&number.to_le_bytes());
                Ok(offset)
            }
            ExsValue::Float(number) => {
                self.cbor(9)?;
                let offset = self.record(RECORD_FLOAT, 0)?;
                self.payload(&number.to_bits().to_le_bytes());
                Ok(offset)
            }
            ExsValue::String(text) => self.string(text),
            ExsValue::Bytes(octets) => {
                self.cbor(head_len(octets.len() as u64) + octets.len())?;
                let offset = self.record(RECORD_BYTES, octets.len())?;
                self.payload(octets);
                Ok(offset)
            }
            ExsValue::List(items) => {
                self.collection(items.len())?;
                self.cbor(head_len(items.len() as u64))?;
                let words = items
                    .iter()
                    .map(|item| self.value(item, depth + 1))
                    .collect::<Result<Vec<_>, _>>()?;
                let offset = self.record(RECORD_LIST, items.len())?;
                self.words(&words);
                Ok(offset)
            }
            ExsValue::Object(entries) => {
                self.collection(entries.len())?;
                self.cbor(head_len(entries.len() as u64))?;
                let mut words = Vec::with_capacity(entries.len() * 2);
                for (key, item) in entries {
                    words.push(self.string(key)?);
                    words.push(self.value(item, depth + 1)?);
                }
                let offset = self.record(RECORD_OBJECT, entries.len())?;
                self.words(&words);
                Ok(offset)
            }
            ExsValue::Enum {
                type_id,
                variant,
                fields,
            } => {
                self.collection(fields.len())?;
                self.cbor(head_len(ENUM_TAG) + 1 + head_len(fields.len() as u64))?;
                let mut words = Vec::with_capacity(fields.len() + 2);
                words.push(self.string(type_id)?);
                words.push(self.string(variant)?);
                for field in fields {
                    words.push(self.value(field, depth + 1)?);
                }
                let offset = self.record(RECORD_ENUM, fields.len())?;
                self.words(&words);
                Ok(offset)
            }
            ExsValue::Error(error) => self.error(error, depth),
        }
    }

    /// Writes an Error record at nesting depth `depth` and returns its offset.
    fn error(&mut self, error: &ExsError, depth: usize) -> Result<u32, CborError> {
        self.collection(error.trace.len())?;
        // Tag, map head and the seven keys, the severity and the trace array head.
        self.cbor(head_len(ERROR_TAG) + 1 + 7 + 1 + head_len(error.trace.len() as u64))?;
        let kind = self.string(&error.kind)?;
        let message = self.string(&error.message)?;
        let data = self.value(&error.data, depth + 1)?;
        let origin = match error.origin {
            Some(position) => {
                self.cbor(head_len(u64::from(position.0)))?;
                position.0.checked_add(1).ok_or(CborError::Encode)?
            }
            None => {
                self.cbor(1)?;
                0
            }
        };
        for frame in &error.trace {
            self.cbor(
                1 + head_len(u64::from(frame.function_id)) + head_len(u64::from(frame.call_site.0)),
            )?;
        }
        let cause = match &error.cause {
            Some(cause) => self.value(cause, depth + 1)?,
            None => {
                self.cbor(1)?;
                0
            }
        };
        let fatal = matches!(error.severity, ErrorSeverity::Fatal);
        let trace = u32::try_from(error.trace.len()).map_err(|_| CborError::Encode)?;
        let offset = self.record(RECORD_ERROR, usize::from(fatal))?;
        self.words(&[kind, message, data, origin, cause, trace]);
        for frame in &error.trace {
            let entry = u64::from(frame.function_id) | (u64::from(frame.call_site.0) << 32);
            self.bytes.extend_from_slice(&entry.to_le_bytes());
        }
        Ok(offset)
    }
}
