//! Reading values from images, which may come from an untrusted module.
//!
//! Every offset, length and record kind is checked, Strings must be valid UTF-8, and the
//! limits apply to the value: nesting, collection entries and its payload size, so records
//! shared by several parents cannot make a value grow without bound.

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
use crate::value::{
    BoundaryError, BoundaryLimits, ErrorSeverity, ExsError, ExsStackFrame, ExsValue,
    SourcePositionId,
};

/// Reads the value of the image `bytes` within `limits`.
///
/// # Errors
///
/// Returns `Malformed` for an invalid image, or the exceeded limit.
pub(super) fn read_image(bytes: &[u8], limits: BoundaryLimits) -> Result<ExsValue, BoundaryError> {
    if bytes.len() < IMAGE_HEADER_SIZE as usize || word(bytes, 0)? != IMAGE_MAGIC {
        return Err(BoundaryError::Malformed);
    }
    let mut reader = Reader {
        bytes,
        payload_len: 0,
        limits,
    };
    reader.value(word(bytes, 4)?, 1)
}

/// Returns the little-endian `u32` at byte `offset`.
fn word(bytes: &[u8], offset: usize) -> Result<u32, BoundaryError> {
    bytes
        .get(offset..offset.checked_add(4).ok_or(BoundaryError::Malformed)?)
        .and_then(|word| word.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or(BoundaryError::Malformed)
}

/// Returns the little-endian `u64` at byte `offset`.
fn double_word(bytes: &[u8], offset: usize) -> Result<u64, BoundaryError> {
    bytes
        .get(offset..offset.checked_add(8).ok_or(BoundaryError::Malformed)?)
        .and_then(|word| word.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or(BoundaryError::Malformed)
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

/// The image being read, with its payload size.
struct Reader<'b> {
    /// The image.
    bytes: &'b [u8],
    /// Payload size so far.
    payload_len: usize,
    /// The limits the value must keep.
    limits: BoundaryLimits,
}

impl Reader<'_> {
    /// Accounts for `count` bytes of payload.
    fn account(&mut self, count: usize) -> Result<(), BoundaryError> {
        self.payload_len = self.payload_len.saturating_add(count);
        if self.payload_len > self.limits.max_payload_bytes {
            return Err(BoundaryError::PayloadLimitExceeded);
        }
        Ok(())
    }

    /// Checks one collection's entry count.
    fn collection(&self, count: u32) -> Result<usize, BoundaryError> {
        let count = count as usize;
        if count > self.limits.max_collection_entries {
            return Err(BoundaryError::CollectionLimitExceeded);
        }
        Ok(count)
    }

    /// Returns the record at `offset`, which must be an aligned record inside the image.
    fn record(&self, offset: u32) -> Result<Record, BoundaryError> {
        let offset = offset as usize;
        if offset < IMAGE_HEADER_SIZE as usize || !offset.is_multiple_of(8) {
            return Err(BoundaryError::Malformed);
        }
        Ok(Record {
            payload: offset + RECORD_HEADER_SIZE as usize,
            kind: word(self.bytes, offset)?,
            argument: word(self.bytes, offset + 4)?,
        })
    }

    /// Returns `len` payload bytes of `record` from byte `start` of its payload.
    fn payload(&self, record: Record, start: usize, len: usize) -> Result<&[u8], BoundaryError> {
        let from = record
            .payload
            .checked_add(start)
            .ok_or(BoundaryError::Malformed)?;
        let to = from.checked_add(len).ok_or(BoundaryError::Malformed)?;
        self.bytes.get(from..to).ok_or(BoundaryError::Malformed)
    }

    /// Returns `u32` word `index` of `record`'s payload.
    fn payload_word(&self, record: Record, index: usize) -> Result<u32, BoundaryError> {
        word(
            self.bytes,
            record
                .payload
                .checked_add(index * 4)
                .ok_or(BoundaryError::Malformed)?,
        )
    }

    /// Reads the String record at `offset`.
    fn string(&mut self, offset: u32) -> Result<String, BoundaryError> {
        let record = self.record(offset)?;
        if record.kind != RECORD_STRING {
            return Err(BoundaryError::Malformed);
        }
        let len = record.argument as usize;
        self.account(head_len(len as u64) + len)?;
        let text = self.payload(record, 8, len)?;
        core::str::from_utf8(text)
            .map(ToOwned::to_owned)
            .map_err(|_| BoundaryError::Malformed)
    }

    /// Reads the record at `offset` at nesting depth `depth`.
    fn value(&mut self, offset: u32, depth: usize) -> Result<ExsValue, BoundaryError> {
        if depth > self.limits.max_nesting {
            return Err(BoundaryError::NestingLimitExceeded);
        }
        let record = self.record(offset)?;
        match record.kind {
            RECORD_NONE => {
                self.account(1)?;
                Ok(ExsValue::None)
            }
            RECORD_BOOL => {
                self.account(1)?;
                match record.argument {
                    0 => Ok(ExsValue::Bool(false)),
                    1 => Ok(ExsValue::Bool(true)),
                    _ => Err(BoundaryError::Malformed),
                }
            }
            RECORD_INT => {
                let value = double_word(self.bytes, record.payload)?.cast_signed();
                self.account(int_len(value))?;
                Ok(ExsValue::Int(value))
            }
            RECORD_FLOAT => {
                self.account(9)?;
                Ok(ExsValue::Float(f64::from_bits(double_word(
                    self.bytes,
                    record.payload,
                )?)))
            }
            RECORD_STRING => self.string(offset).map(ExsValue::String),
            RECORD_BYTES => {
                let len = record.argument as usize;
                self.account(head_len(len as u64) + len)?;
                Ok(ExsValue::Bytes(self.payload(record, 0, len)?.to_vec()))
            }
            RECORD_LIST => {
                let count = self.collection(record.argument)?;
                self.account(head_len(count as u64))?;
                let mut items = Vec::new();
                for index in 0..count {
                    let item = self.payload_word(record, index)?;
                    items.push(self.value(item, depth + 1)?);
                }
                Ok(ExsValue::List(items))
            }
            RECORD_OBJECT => {
                let count = self.collection(record.argument)?;
                self.account(head_len(count as u64))?;
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
                self.account(head_len(ENUM_TAG) + 1 + head_len(count as u64))?;
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
            _ => Err(BoundaryError::Malformed),
        }
    }

    /// Reads an Error record at nesting depth `depth`.
    fn error(&mut self, record: Record, depth: usize) -> Result<ExsValue, BoundaryError> {
        let severity = match record.argument {
            0 => ErrorSeverity::Recoverable,
            1 => ErrorSeverity::Fatal,
            _ => return Err(BoundaryError::Malformed),
        };
        let trace_count = self.collection(self.payload_word(record, 5)?)?;
        self.account(head_len(ERROR_TAG) + 1 + 7 + 1 + head_len(trace_count as u64))?;
        let kind = self.string(self.payload_word(record, 0)?)?;
        let message = self.string(self.payload_word(record, 1)?)?;
        let data = self.value(self.payload_word(record, 2)?, depth + 1)?;
        let origin = match self.payload_word(record, 3)? {
            0 => {
                self.account(1)?;
                None
            }
            origin => {
                let position = origin - 1;
                self.account(head_len(u64::from(position)))?;
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
                    .ok_or(BoundaryError::Malformed)?,
            )?;
            let frame = ExsStackFrame {
                function_id: entry as u32,
                call_site: SourcePositionId((entry >> 32) as u32),
            };
            self.account(
                1 + head_len(u64::from(frame.function_id)) + head_len(u64::from(frame.call_site.0)),
            )?;
            trace.push(frame);
        }
        // A None cause is no cause.
        let cause = match self.payload_word(record, 4)? {
            0 => {
                self.account(1)?;
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
