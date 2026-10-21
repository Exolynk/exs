//! Reading values from images, which may come from an untrusted module.
//!
//! Every offset, length and record kind is checked, Strings must be valid UTF-8, and the
//! limits apply as if the value were CBOR: nesting, collection entries and the byte size of
//! the equivalent CBOR, so records shared by several parents cannot make a value grow without
//! bound.

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use super::format::{
    IMAGE_HEADER_SIZE, IMAGE_MAGIC, RECORD_BOOL, RECORD_BYTES, RECORD_ENUM, RECORD_ERROR,
    RECORD_FLOAT, RECORD_HEADER_SIZE, RECORD_INT, RECORD_LIST, RECORD_NONE, RECORD_OBJECT,
    RECORD_STRING,
};
use super::writer::{ENUM_TAG, ERROR_TAG, head_len, int_len};
use crate::cbor::{
    CborError, CborLimits, ErrorSeverity, ExsError, ExsStackFrame, ExsValue, SourcePositionId,
};

/// Reads the value of the image `bytes` within `limits`.
///
/// # Errors
///
/// Returns `Malformed` for an invalid image, or the exceeded limit.
pub(super) fn read_image(bytes: &[u8], limits: CborLimits) -> Result<ExsValue, CborError> {
    if bytes.len() < IMAGE_HEADER_SIZE as usize || word(bytes, 0)? != IMAGE_MAGIC {
        return Err(CborError::Malformed);
    }
    let mut reader = Reader {
        bytes,
        cbor_len: 0,
        limits,
    };
    reader.value(word(bytes, 4)?, 1)
}

/// Returns the little-endian `u32` at byte `offset`.
fn word(bytes: &[u8], offset: usize) -> Result<u32, CborError> {
    bytes
        .get(offset..offset.checked_add(4).ok_or(CborError::Malformed)?)
        .and_then(|word| word.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or(CborError::Malformed)
}

/// Returns the little-endian `u64` at byte `offset`.
fn double_word(bytes: &[u8], offset: usize) -> Result<u64, CborError> {
    bytes
        .get(offset..offset.checked_add(8).ok_or(CborError::Malformed)?)
        .and_then(|word| word.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or(CborError::Malformed)
}

/// One record: its payload offset, kind and argument.
#[derive(Clone, Copy)]
struct Record {
    /// Byte offset of the payload.
    payload: usize,
    /// The record kind.
    kind: u32,
    /// The record argument.
    argument: u32,
}

/// The image being read, with the size of the equivalent CBOR.
struct Reader<'b> {
    /// The image.
    bytes: &'b [u8],
    /// Byte length of the equivalent CBOR so far.
    cbor_len: usize,
    /// The limits the value must keep.
    limits: CborLimits,
}

impl Reader<'_> {
    /// Accounts for `count` bytes of the equivalent CBOR.
    fn cbor(&mut self, count: usize) -> Result<(), CborError> {
        self.cbor_len = self.cbor_len.saturating_add(count);
        if self.cbor_len > self.limits.max_payload_bytes {
            return Err(CborError::PayloadLimitExceeded);
        }
        Ok(())
    }

    /// Checks one collection's entry count.
    fn collection(&self, count: u32) -> Result<usize, CborError> {
        let count = count as usize;
        if count > self.limits.max_collection_entries {
            return Err(CborError::CollectionLimitExceeded);
        }
        Ok(count)
    }

    /// Returns the record at `offset`, which must be an aligned record inside the image.
    fn record(&self, offset: u32) -> Result<Record, CborError> {
        let offset = offset as usize;
        if offset < IMAGE_HEADER_SIZE as usize || !offset.is_multiple_of(8) {
            return Err(CborError::Malformed);
        }
        Ok(Record {
            payload: offset + RECORD_HEADER_SIZE as usize,
            kind: word(self.bytes, offset)?,
            argument: word(self.bytes, offset + 4)?,
        })
    }

    /// Returns `len` payload bytes of `record` from byte `start` of its payload.
    fn payload(&self, record: Record, start: usize, len: usize) -> Result<&[u8], CborError> {
        let from = record
            .payload
            .checked_add(start)
            .ok_or(CborError::Malformed)?;
        let to = from.checked_add(len).ok_or(CborError::Malformed)?;
        self.bytes.get(from..to).ok_or(CborError::Malformed)
    }

    /// Returns `u32` word `index` of `record`'s payload.
    fn payload_word(&self, record: Record, index: usize) -> Result<u32, CborError> {
        word(
            self.bytes,
            record
                .payload
                .checked_add(index * 4)
                .ok_or(CborError::Malformed)?,
        )
    }

    /// Reads the String record at `offset`.
    fn string(&mut self, offset: u32) -> Result<String, CborError> {
        let record = self.record(offset)?;
        if record.kind != RECORD_STRING {
            return Err(CborError::Malformed);
        }
        let len = record.argument as usize;
        self.cbor(head_len(len as u64) + len)?;
        let text = self.payload(record, 8, len)?;
        core::str::from_utf8(text)
            .map(ToOwned::to_owned)
            .map_err(|_| CborError::Malformed)
    }

    /// Reads the record at `offset` at nesting depth `depth`.
    fn value(&mut self, offset: u32, depth: usize) -> Result<ExsValue, CborError> {
        if depth > self.limits.max_nesting {
            return Err(CborError::NestingLimitExceeded);
        }
        let record = self.record(offset)?;
        match record.kind {
            RECORD_NONE => {
                self.cbor(1)?;
                Ok(ExsValue::None)
            }
            RECORD_BOOL => {
                self.cbor(1)?;
                match record.argument {
                    0 => Ok(ExsValue::Bool(false)),
                    1 => Ok(ExsValue::Bool(true)),
                    _ => Err(CborError::Malformed),
                }
            }
            RECORD_INT => {
                let value = double_word(self.bytes, record.payload)?.cast_signed();
                self.cbor(int_len(value))?;
                Ok(ExsValue::Int(value))
            }
            RECORD_FLOAT => {
                self.cbor(9)?;
                Ok(ExsValue::Float(f64::from_bits(double_word(
                    self.bytes,
                    record.payload,
                )?)))
            }
            RECORD_STRING => self.string(offset).map(ExsValue::String),
            RECORD_BYTES => {
                let len = record.argument as usize;
                self.cbor(head_len(len as u64) + len)?;
                Ok(ExsValue::Bytes(self.payload(record, 0, len)?.to_vec()))
            }
            RECORD_LIST => {
                let count = self.collection(record.argument)?;
                self.cbor(head_len(count as u64))?;
                let mut items = Vec::new();
                for index in 0..count {
                    let item = self.payload_word(record, index)?;
                    items.push(self.value(item, depth + 1)?);
                }
                Ok(ExsValue::List(items))
            }
            RECORD_OBJECT => {
                let count = self.collection(record.argument)?;
                self.cbor(head_len(count as u64))?;
                let mut entries = Vec::new();
                for index in 0..count {
                    let key = self.string(self.payload_word(record, index * 2)?)?;
                    let value = self.value(self.payload_word(record, index * 2 + 1)?, depth + 1)?;
                    entries.push((key, value));
                }
                Ok(ExsValue::Object(entries))
            }
            RECORD_ENUM => {
                let count = self.collection(record.argument)?;
                self.cbor(head_len(ENUM_TAG) + 1 + head_len(count as u64))?;
                let type_id = self.string(self.payload_word(record, 0)?)?;
                let variant = self.string(self.payload_word(record, 1)?)?;
                let mut fields = Vec::new();
                for index in 0..count {
                    let field = self.payload_word(record, index + 2)?;
                    fields.push(self.value(field, depth + 1)?);
                }
                Ok(ExsValue::Enum {
                    type_id,
                    variant,
                    fields,
                })
            }
            RECORD_ERROR => self.error(record, depth),
            _ => Err(CborError::Malformed),
        }
    }

    /// Reads an Error record at nesting depth `depth`.
    fn error(&mut self, record: Record, depth: usize) -> Result<ExsValue, CborError> {
        let severity = match record.argument {
            0 => ErrorSeverity::Recoverable,
            1 => ErrorSeverity::Fatal,
            _ => return Err(CborError::Malformed),
        };
        let trace_count = self.collection(self.payload_word(record, 5)?)?;
        self.cbor(head_len(ERROR_TAG) + 1 + 7 + 1 + head_len(trace_count as u64))?;
        let kind = self.string(self.payload_word(record, 0)?)?;
        let message = self.string(self.payload_word(record, 1)?)?;
        let data = self.value(self.payload_word(record, 2)?, depth + 1)?;
        let origin = match self.payload_word(record, 3)? {
            0 => {
                self.cbor(1)?;
                None
            }
            origin => {
                let position = origin - 1;
                self.cbor(head_len(u64::from(position)))?;
                Some(SourcePositionId(position))
            }
        };
        let mut trace = Vec::new();
        for index in 0..trace_count {
            let entry = double_word(
                self.bytes,
                record
                    .payload
                    .checked_add(24 + index * 8)
                    .ok_or(CborError::Malformed)?,
            )?;
            let frame = ExsStackFrame {
                function_id: entry as u32,
                call_site: SourcePositionId((entry >> 32) as u32),
            };
            self.cbor(
                1 + head_len(u64::from(frame.function_id)) + head_len(u64::from(frame.call_site.0)),
            )?;
            trace.push(frame);
        }
        // Like CBOR, whose `null` cause decodes as no cause, a None cause is no cause.
        let cause = match self.payload_word(record, 4)? {
            0 => {
                self.cbor(1)?;
                None
            }
            offset => match self.value(offset, depth + 1)? {
                ExsValue::None => None,
                cause => Some(Box::new(cause)),
            },
        };
        Ok(ExsValue::Error(ExsError {
            severity,
            kind,
            message,
            data: Box::new(data),
            origin,
            trace,
            cause,
        }))
    }
}
