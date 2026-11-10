//! Writing values as images.
//!
//! The writer applies the runner's limits to the value: nesting, collection entries and its
//! payload size. A parent record comes before its parts, whose offsets it receives as they are
//! written.

use alloc::string::String;
use alloc::vec::Vec;

use super::format::{
    IMAGE_HEADER_SIZE, IMAGE_MAGIC, RECORD_BOOL, RECORD_BYTES, RECORD_ENUM, RECORD_ERROR,
    RECORD_FLOAT, RECORD_HEADER_SIZE, RECORD_INT, RECORD_LIST, RECORD_NONE, RECORD_OBJECT,
    RECORD_STRING, RECORD_TYPED, RECORD_UNIT,
};
use crate::value::{BoundaryError, BoundaryLimits, ErrorSeverity, ExsError, ExsValue};

/// Tag number of an Error in the compact encoding the payload size measures.
pub(super) const ERROR_TAG: u64 = 60_001;
/// Tag number of an enum in the compact encoding the payload size measures.
pub(super) const ENUM_TAG: u64 = 60_005;
/// Tag number of a nominal Object value in the compact encoding the payload size measures.
pub(super) const TYPED_TAG: u64 = 60_006;

/// Returns `value` as an image, or the limit it exceeds.
///
/// # Errors
///
/// Returns the exceeded limit, or `BoundaryError::TooLarge` for an image beyond 4 GiB.
pub(super) fn write_image(
    value: &ExsValue,
    limits: BoundaryLimits,
) -> Result<Vec<u8>, BoundaryError> {
    let mut writer = Writer::new(limits);
    let root = writer.value(value, 1)?;
    Ok(writer.finish(root))
}

/// Returns the image of a List holding `items`, as [`write_image`] would write
/// `ExsValue::List(items.to_vec())`, without copying the items.
///
/// # Errors
///
/// Returns the exceeded limit, or `BoundaryError::TooLarge` for an image beyond 4 GiB.
pub(super) fn write_list_image(
    items: &[ExsValue],
    limits: BoundaryLimits,
) -> Result<Vec<u8>, BoundaryError> {
    let mut writer = Writer::new(limits);
    let root = writer.list(items, 1)?;
    Ok(writer.finish(root))
}

/// Returns the byte length of a head with `argument` in the compact encoding.
pub(super) const fn head_len(argument: u64) -> usize {
    match argument {
        0..=23 => 1,
        24..=0xff => 2,
        0x100..=0xffff => 3,
        0x1_0000..=0xffff_ffff => 5,
        _ => 9,
    }
}

/// Returns the byte length of the Int `value` in the compact encoding.
pub(super) const fn int_len(value: i64) -> usize {
    if value < 0 {
        head_len((-1 - value).cast_unsigned())
    } else {
        head_len(value.cast_unsigned())
    }
}

/// The image being written, with its payload size.
struct Writer {
    /// The image so far.
    bytes: Vec<u8>,
    /// Payload size so far.
    payload_len: usize,
    /// The limits the value must keep.
    limits: BoundaryLimits,
}

impl Writer {
    /// Returns a writer holding the image header, with the root offset still unset.
    fn new(limits: BoundaryLimits) -> Self {
        let mut bytes = Vec::with_capacity(64);
        bytes.extend_from_slice(&IMAGE_MAGIC.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        debug_assert_eq!(bytes.len(), IMAGE_HEADER_SIZE as usize);
        Self {
            bytes,
            payload_len: 0,
            limits,
        }
    }

    /// Stores the offset of the root record in the header and returns the image.
    fn finish(mut self, root: u32) -> Vec<u8> {
        self.bytes[4..8].copy_from_slice(&root.to_le_bytes());
        self.bytes
    }

    /// Accounts for `count` bytes of payload.
    fn account(&mut self, count: usize) -> Result<(), BoundaryError> {
        self.payload_len = self.payload_len.saturating_add(count);
        if self.payload_len > self.limits.max_payload_bytes {
            return Err(BoundaryError::PayloadLimitExceeded);
        }
        Ok(())
    }

    /// Checks one collection's entry count.
    fn collection(&self, count: usize) -> Result<(), BoundaryError> {
        if count > self.limits.max_collection_entries {
            return Err(BoundaryError::CollectionLimitExceeded);
        }
        Ok(())
    }

    /// Returns the offset the next record starts at.
    fn offset(&self) -> Result<u32, BoundaryError> {
        u32::try_from(self.bytes.len()).map_err(|_| BoundaryError::TooLarge)
    }

    /// Starts a record of `kind` with `argument` and returns its offset.
    fn record(&mut self, kind: u32, argument: usize) -> Result<u32, BoundaryError> {
        let offset = self.offset()?;
        let argument = u32::try_from(argument).map_err(|_| BoundaryError::TooLarge)?;
        self.bytes.extend_from_slice(&kind.to_le_bytes());
        self.bytes.extend_from_slice(&argument.to_le_bytes());
        Ok(offset)
    }

    /// Appends `bytes` and pads the image to 8 bytes.
    fn payload(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
        self.bytes.resize(self.bytes.len().next_multiple_of(8), 0);
    }

    /// Starts a record of `kind` with `argument` and `payload` zeroed payload bytes, padded to
    /// 8 bytes, and returns its offset. The parts written later fill in the payload.
    fn reserved(
        &mut self,
        kind: u32,
        argument: usize,
        payload: usize,
    ) -> Result<u32, BoundaryError> {
        let offset = self.record(kind, argument)?;
        let end = self
            .bytes
            .len()
            .checked_add(payload)
            .ok_or(BoundaryError::TooLarge)?
            .next_multiple_of(8);
        self.bytes.resize(end, 0);
        Ok(offset)
    }

    /// Stores `value` in payload word `index` of the record at `record`.
    fn set_word(&mut self, record: u32, index: usize, value: u32) {
        let at = record as usize + RECORD_HEADER_SIZE as usize + index * 4;
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Writes a List record of `items` at nesting depth `depth` and returns its offset.
    fn list(&mut self, items: &[ExsValue], depth: usize) -> Result<u32, BoundaryError> {
        if depth > self.limits.max_nesting {
            return Err(BoundaryError::NestingLimitExceeded);
        }
        self.collection(items.len())?;
        self.account(head_len(items.len() as u64))?;
        let offset = self.reserved(RECORD_LIST, items.len(), items.len() * 4)?;
        for (index, item) in items.iter().enumerate() {
            let part = self.value(item, depth + 1)?;
            self.set_word(offset, index, part);
        }
        Ok(offset)
    }

    /// Writes the key and value of each of `entries` into the words of the record at `record`,
    /// from word `first`, at nesting depth `depth`.
    fn entries(
        &mut self,
        record: u32,
        first: usize,
        entries: &[(String, ExsValue)],
        depth: usize,
    ) -> Result<(), BoundaryError> {
        for (index, (key, item)) in entries.iter().enumerate() {
            let part = self.string(key)?;
            self.set_word(record, first + index * 2, part);
            let part = self.value(item, depth + 1)?;
            self.set_word(record, first + index * 2 + 1, part);
        }
        Ok(())
    }

    /// Writes a String record and returns its offset.
    fn string(&mut self, text: &str) -> Result<u32, BoundaryError> {
        self.account(head_len(text.len() as u64) + text.len())?;
        let offset = self.record(RECORD_STRING, text.len())?;
        let chars = u32::try_from(text.chars().count()).map_err(|_| BoundaryError::TooLarge)?;
        self.bytes.extend_from_slice(&chars.to_le_bytes());
        self.bytes.extend_from_slice(&0_u32.to_le_bytes());
        self.payload(text.as_bytes());
        Ok(offset)
    }

    /// Writes the records of `value` at nesting depth `depth` and returns its offset.
    fn value(&mut self, value: &ExsValue, depth: usize) -> Result<u32, BoundaryError> {
        if depth > self.limits.max_nesting {
            return Err(BoundaryError::NestingLimitExceeded);
        }
        match value {
            ExsValue::None => {
                self.account(1)?;
                self.record(RECORD_NONE, 0)
            }
            ExsValue::Unit => {
                self.account(1)?;
                self.record(RECORD_UNIT, 0)
            }
            ExsValue::Bool(flag) => {
                self.account(1)?;
                self.record(RECORD_BOOL, usize::from(*flag))
            }
            ExsValue::Int(number) => {
                self.account(int_len(*number))?;
                let offset = self.record(RECORD_INT, 0)?;
                self.payload(&number.to_le_bytes());
                Ok(offset)
            }
            ExsValue::Float(number) => {
                self.account(9)?;
                let offset = self.record(RECORD_FLOAT, 0)?;
                self.payload(&number.to_bits().to_le_bytes());
                Ok(offset)
            }
            ExsValue::String(text) => self.string(text),
            ExsValue::Bytes(octets) => {
                self.account(head_len(octets.len() as u64) + octets.len())?;
                let offset = self.record(RECORD_BYTES, octets.len())?;
                self.payload(octets);
                Ok(offset)
            }
            ExsValue::List(items) => self.list(items, depth),
            ExsValue::Object(entries) => {
                self.collection(entries.len())?;
                self.account(head_len(entries.len() as u64))?;
                let offset = self.reserved(RECORD_OBJECT, entries.len(), entries.len() * 8)?;
                self.entries(offset, 0, entries, depth)?;
                Ok(offset)
            }
            ExsValue::Typed { type_id, fields } => {
                self.collection(fields.len())?;
                self.account(head_len(TYPED_TAG) + 1 + head_len(fields.len() as u64))?;
                let offset = self.reserved(RECORD_TYPED, fields.len(), fields.len() * 8 + 4)?;
                let part = self.string(type_id)?;
                self.set_word(offset, 0, part);
                self.entries(offset, 1, fields, depth)?;
                Ok(offset)
            }
            ExsValue::Enum {
                type_id,
                variant,
                fields,
            } => {
                self.collection(fields.len())?;
                self.account(head_len(ENUM_TAG) + 1 + head_len(fields.len() as u64))?;
                let offset = self.reserved(RECORD_ENUM, fields.len(), fields.len() * 4 + 8)?;
                let part = self.string(type_id)?;
                self.set_word(offset, 0, part);
                let part = self.string(variant)?;
                self.set_word(offset, 1, part);
                for (index, field) in fields.iter().enumerate() {
                    let part = self.value(field, depth + 1)?;
                    self.set_word(offset, index + 2, part);
                }
                Ok(offset)
            }
            ExsValue::Error(error) => self.error(error, depth),
        }
    }

    /// Writes an Error record at nesting depth `depth` and returns its offset.
    fn error(&mut self, error: &ExsError, depth: usize) -> Result<u32, BoundaryError> {
        self.collection(error.trace.len())?;
        // Tag, map head and the seven keys, the severity and the trace array head.
        self.account(head_len(ERROR_TAG) + 1 + 7 + 1 + head_len(error.trace.len() as u64))?;
        let fatal = matches!(error.severity, ErrorSeverity::Fatal);
        let trace = u32::try_from(error.trace.len()).map_err(|_| BoundaryError::TooLarge)?;
        let trace_bytes = error
            .trace
            .len()
            .checked_mul(8)
            .ok_or(BoundaryError::TooLarge)?;
        let offset = self.reserved(RECORD_ERROR, usize::from(fatal), 24 + trace_bytes)?;
        let entries = offset as usize + RECORD_HEADER_SIZE as usize + 24;
        for (index, frame) in error.trace.iter().enumerate() {
            let entry = u64::from(frame.function_id) | (u64::from(frame.call_site.0) << 32);
            let at = entries + index * 8;
            self.bytes[at..at + 8].copy_from_slice(&entry.to_le_bytes());
        }
        let part = self.string(&error.kind)?;
        self.set_word(offset, 0, part);
        let part = self.string(&error.message)?;
        self.set_word(offset, 1, part);
        let part = self.value(&error.data, depth + 1)?;
        self.set_word(offset, 2, part);
        let origin = match error.origin {
            Some(position) => {
                self.account(head_len(u64::from(position.0)))?;
                position.0.checked_add(1).ok_or(BoundaryError::TooLarge)?
            }
            None => {
                self.account(1)?;
                0
            }
        };
        self.set_word(offset, 3, origin);
        for frame in &error.trace {
            self.account(
                1 + head_len(u64::from(frame.function_id)) + head_len(u64::from(frame.call_site.0)),
            )?;
        }
        let cause = match &error.cause {
            Some(cause) => self.value(cause, depth + 1)?,
            None => {
                self.account(1)?;
                0
            }
        };
        self.set_word(offset, 4, cause);
        self.set_word(offset, 5, trace);
        Ok(offset)
    }
}
