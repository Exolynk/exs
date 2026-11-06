//! Writing the images in which a module passes results and host-call arguments to the runner.
//!
//! The writer walks a value in the module's memory and lays it out as records of the
//! `exs_abi::image` format in the VM's output buffer, so the runner never reads the heap. A
//! parent record comes before its parts and receives their offsets as they are written. Typed
//! Objects carry their boundary identity followed by their present declared fields in entry
//! order and their extra keys, as do untyped Objects that keep the identity of a type the
//! program does not use. Closures, Cells and nesting beyond [`MAX_DEPTH`] cannot cross the
//! boundary.

use exs_abi::image::{
    IMAGE_HEADER_SIZE, IMAGE_MAGIC, RECORD_BOOL, RECORD_BYTES, RECORD_ENUM, RECORD_ERROR,
    RECORD_FLOAT, RECORD_HEADER_SIZE, RECORD_INT, RECORD_LIST, RECORD_NONE, RECORD_OBJECT,
    RECORD_STRING, RECORD_TYPED, RECORD_UNIT,
};
use exs_layout::{Immediate, ValueRef};

use crate::boundary::serialization_error;
use crate::bytes::{bytes_len, bytes_object};
use crate::enums::{enum_object, field_address as enum_field, names, payload_count};
use crate::errors::{error_object, error_parts};
use crate::execution::{grow_output, output, result_value};
use crate::lists::{item_address, len, list_object};
use crate::memory::Memory;
use crate::numbers::{Number, number, reference};
use crate::objects::{foreign_identity, shape, untyped_object, value_address};
use crate::shapes::{key, key_count};
use crate::strings::{DATA, byte_len, char_count, string_object};
use crate::typed::{
    extra, ordered_slot, slot_address, slot_count, type_entry, type_id, typed_object,
};

/// Deepest nesting of Lists, Objects, enums and Errors the writer follows; deeper values,
/// including cyclic ones, cannot cross the boundary.
const MAX_DEPTH: u32 = 128;
/// Payload words of an Error record before its trace entries: the offsets of its kind, message
/// and data, its origin, the offset of its cause and its trace count.
const ERROR_WORDS: u32 = 6;
/// Byte size of one trace entry.
const TRACE_ENTRY_SIZE: u32 = 8;
/// Bytes of a String record's payload before its text: the character count and padding.
const STRING_PREFIX: u32 = 8;

/// Why a value could not be written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WriteError {
    /// The value cannot cross the boundary, such as a closure or a cyclic graph.
    Unsupported,
    /// Memory cannot hold the image.
    OutOfMemory,
}

/// Writes the image of `raw`, a value of the VM at `vm`, and returns its address and length.
///
/// The image stays valid until the next one is written. Writing allocates no heap objects, and
/// when memory runs short it collects with `raw` as a root.
///
/// # Errors
///
/// Returns the reason the value could not be written.
pub(crate) fn write_image(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
) -> Result<(u32, u32), WriteError> {
    let (base, capacity) = output(memory, vm);
    let mut writer = Writer {
        memory,
        vm,
        root: raw,
        base,
        capacity,
        length: 0,
    };
    writer.reserve(IMAGE_HEADER_SIZE)?;
    let root = writer.value(raw, 0)?;
    writer.memory.store_u32(writer.base, IMAGE_MAGIC);
    writer.memory.store_u32(writer.base + 4, root);
    Ok((writer.base, writer.length))
}

/// Writes the image of the completed result and returns its location, its address in the low
/// and its length in the high 32 bits. A result that cannot cross the boundary becomes a fatal
/// SerializationError without origin and trace. Returns `None` when memory is exhausted.
pub fn result_image(memory: &mut impl Memory, vm: u32) -> Option<u64> {
    let raw = result_value(memory, vm);
    let (address, length) = match write_image(memory, vm, raw) {
        Ok(location) => location,
        Err(WriteError::OutOfMemory) => return None,
        Err(WriteError::Unsupported) => {
            let error = serialization_error(memory, vm, raw, true)?;
            write_image(memory, vm, error).ok()?
        }
    };
    Some(u64::from(address) | (u64::from(length) << 32))
}

/// One image being written.
struct Writer<'w, M> {
    /// The module's memory.
    memory: &'w mut M,
    /// Address of the VM state block.
    vm: u32,
    /// The value being written, which a collection must keep.
    root: u64,
    /// Address of the output buffer.
    base: u32,
    /// Byte capacity of the output buffer.
    capacity: u32,
    /// Byte length of the image so far.
    length: u32,
}

impl<M: Memory> Writer<'_, M> {
    /// Reserves `size` bytes, rounded up to the alignment of records, and returns their offset.
    fn reserve(&mut self, size: u32) -> Result<u32, WriteError> {
        let offset = self.length;
        let end = size
            .checked_next_multiple_of(8)
            .and_then(|size| offset.checked_add(size))
            .ok_or(WriteError::OutOfMemory)?;
        if end > self.capacity {
            (self.base, self.capacity) = grow_output(self.memory, self.vm, offset, end, self.root)
                .ok_or(WriteError::OutOfMemory)?;
        }
        self.length = end;
        Ok(offset)
    }

    /// Starts a record of `kind` and `argument` with `payload` bytes and returns its offset.
    fn record(&mut self, kind: u32, argument: u32, payload: u32) -> Result<u32, WriteError> {
        let size = payload
            .checked_add(RECORD_HEADER_SIZE)
            .ok_or(WriteError::OutOfMemory)?;
        let offset = self.reserve(size)?;
        let address = self.base + offset;
        self.memory.store_u32(address, kind);
        self.memory.store_u32(address + 4, argument);
        Ok(offset)
    }

    /// Starts a record of `kind` and `argument` with `words` payload words and returns its
    /// offset.
    fn words(&mut self, kind: u32, argument: u32, words: u32) -> Result<u32, WriteError> {
        let payload = words.checked_mul(4).ok_or(WriteError::OutOfMemory)?;
        self.record(kind, argument, payload)
    }

    /// Stores `value` in payload word `index` of the record at `record`.
    fn set_word(&mut self, record: u32, index: u32, value: u32) {
        let address = self.base + record + RECORD_HEADER_SIZE + index * 4;
        self.memory.store_u32(address, value);
    }

    /// Writes the String at `object` and returns its offset.
    fn string(&mut self, object: u32) -> Result<u32, WriteError> {
        let len = byte_len(self.memory, object);
        let payload = len
            .checked_add(STRING_PREFIX)
            .ok_or(WriteError::OutOfMemory)?;
        let offset = self.record(RECORD_STRING, len, payload)?;
        let chars = char_count(self.memory, object);
        self.set_word(offset, 0, chars);
        self.set_word(offset, 1, 0);
        let target = self.base + offset + RECORD_HEADER_SIZE + STRING_PREFIX;
        self.memory.copy(target, object + DATA, len);
        Ok(offset)
    }

    /// Writes the String value `raw` and returns its offset.
    fn string_value(&mut self, raw: u64) -> Result<u32, WriteError> {
        self.string(string_object(raw).ok_or(WriteError::Unsupported)?)
    }

    /// Writes the value `raw` at nesting depth `depth` and returns its offset.
    fn value(&mut self, raw: u64, depth: u32) -> Result<u32, WriteError> {
        if let Some(object) = string_object(raw) {
            return self.string(object);
        }
        if let Some(object) = bytes_object(raw) {
            let len = bytes_len(self.memory, object);
            let offset = self.record(RECORD_BYTES, len, len)?;
            let target = self.base + offset + RECORD_HEADER_SIZE;
            self.memory.copy(target, object + DATA, len);
            return Ok(offset);
        }
        if let Some(list) = list_object(raw) {
            return self.list(list, depth);
        }
        if let Some(object) = typed_object(self.memory, raw) {
            return self.typed_object(object, depth);
        }
        if let Some(object) = error_object(raw) {
            return self.error(object, depth);
        }
        if let Some(object) = enum_object(self.memory, raw) {
            return self.enumeration(object, depth);
        }
        if let Some(object) = untyped_object(self.memory, raw) {
            return self.untyped_object(object, depth);
        }
        match reference(raw).and_then(ValueRef::immediate) {
            Some(Immediate::None) => self.record(RECORD_NONE, 0, 0),
            Some(Immediate::Unit) => self.record(RECORD_UNIT, 0, 0),
            Some(Immediate::Bool(flag)) => self.record(RECORD_BOOL, u32::from(flag), 0),
            _ => match number(self.memory, raw).ok_or(WriteError::Unsupported)? {
                Number::Int(value) => self.number(RECORD_INT, value.cast_unsigned()),
                Number::Float(value) => self.number(RECORD_FLOAT, value.to_bits()),
            },
        }
    }

    /// Writes a number record of `kind` holding `bits` and returns its offset.
    fn number(&mut self, kind: u32, bits: u64) -> Result<u32, WriteError> {
        let offset = self.record(kind, 0, 8)?;
        let address = self.base + offset + RECORD_HEADER_SIZE;
        self.memory.store_u64(address, bits);
        Ok(offset)
    }

    /// Writes the List at `list` and returns its offset.
    fn list(&mut self, list: u32, depth: u32) -> Result<u32, WriteError> {
        if depth >= MAX_DEPTH {
            return Err(WriteError::Unsupported);
        }
        let count = len(self.memory, list);
        let offset = self.words(RECORD_LIST, count, count)?;
        for index in 0..count {
            let item = self.memory.load_u64(item_address(self.memory, list, index));
            let part = self.value(item, depth + 1)?;
            self.set_word(offset, index, part);
        }
        Ok(offset)
    }

    /// Writes the untyped Object at `object`, as a Typed record when it keeps an identity, and
    /// returns its offset.
    fn untyped_object(&mut self, object: u32, depth: u32) -> Result<u32, WriteError> {
        if depth >= MAX_DEPTH {
            return Err(WriteError::Unsupported);
        }
        let count = key_count(self.memory, shape(self.memory, object));
        let words = count.checked_mul(2).ok_or(WriteError::OutOfMemory)?;
        match foreign_identity(self.memory, object) {
            0 => {
                let offset = self.words(RECORD_OBJECT, count, words)?;
                self.entries(offset, 0, object, depth)?;
                Ok(offset)
            }
            identity => {
                let words = words.checked_add(1).ok_or(WriteError::OutOfMemory)?;
                let offset = self.words(RECORD_TYPED, count, words)?;
                let part = self.string(identity)?;
                self.set_word(offset, 0, part);
                self.entries(offset, 1, object, depth)?;
                Ok(offset)
            }
        }
    }

    /// Writes the keys and values of the untyped Object at `object` in key order into the words
    /// of the record at `record`, from word `first`, and returns the word after them.
    fn entries(
        &mut self,
        record: u32,
        first: u32,
        object: u32,
        depth: u32,
    ) -> Result<u32, WriteError> {
        let shape = shape(self.memory, object);
        let mut word = first;
        for index in 0..key_count(self.memory, shape) {
            let name = key(self.memory, shape, index);
            let part = self.string_value(name)?;
            self.set_word(record, word, part);
            let value = self
                .memory
                .load_u64(value_address(self.memory, object, index));
            let part = self.value(value, depth + 1)?;
            self.set_word(record, word + 1, part);
            word += 2;
        }
        Ok(word)
    }

    /// Writes the typed Object at `object` as its boundary identity, followed by its present
    /// declared fields in entry order and its extra keys, and returns its offset.
    fn typed_object(&mut self, object: u32, depth: u32) -> Result<u32, WriteError> {
        if depth >= MAX_DEPTH {
            return Err(WriteError::Unsupported);
        }
        let entry = type_entry(self.memory, self.vm, type_id(self.memory, object))
            .filter(|entry| entry.field_count <= slot_count(self.memory, object))
            .ok_or(WriteError::Unsupported)?;
        let extra = extra(self.memory, object);
        let extra_count = if extra == 0 {
            0
        } else {
            key_count(self.memory, shape(self.memory, extra))
        };
        // Declared fields removed by `delete` or `clear` hold an empty slot and are skipped.
        let present = (0..entry.field_count)
            .filter(|index| self.memory.load_u64(slot_address(object, *index)) != 0)
            .count() as u32;
        let count = present
            .checked_add(extra_count)
            .ok_or(WriteError::Unsupported)?;
        let words = count
            .checked_mul(2)
            .and_then(|words| words.checked_add(1))
            .ok_or(WriteError::OutOfMemory)?;
        let offset = self.words(RECORD_TYPED, count, words)?;
        let part = self.string_value(entry.boundary)?;
        self.set_word(offset, 0, part);
        let mut word = 1;
        for position in 0..entry.field_count {
            let index = ordered_slot(self.memory, object, position);
            let value = self.memory.load_u64(slot_address(object, index));
            if value == 0 {
                continue;
            }
            let name = self.memory.load_u64(entry.field_names + index * 8);
            let part = self.string_value(name)?;
            self.set_word(offset, word, part);
            let part = self.value(value, depth + 1)?;
            self.set_word(offset, word + 1, part);
            word += 2;
        }
        if extra != 0 {
            self.entries(offset, word, extra, depth)?;
        }
        Ok(offset)
    }

    /// Writes the enum at `object` and returns its offset.
    fn enumeration(&mut self, object: u32, depth: u32) -> Result<u32, WriteError> {
        if depth >= MAX_DEPTH {
            return Err(WriteError::Unsupported);
        }
        let (identity, name) =
            names(self.memory, self.vm, object).ok_or(WriteError::Unsupported)?;
        let count = payload_count(self.memory, object);
        let words = count.checked_add(2).ok_or(WriteError::OutOfMemory)?;
        let offset = self.words(RECORD_ENUM, count, words)?;
        let part = self.string(identity)?;
        self.set_word(offset, 0, part);
        let part = self.string(name)?;
        self.set_word(offset, 1, part);
        for index in 0..count {
            let value = self.memory.load_u64(enum_field(object, index));
            let part = self.value(value, depth + 1)?;
            self.set_word(offset, index + 2, part);
        }
        Ok(offset)
    }

    /// Writes the Error at `object` and returns its offset.
    fn error(&mut self, object: u32, depth: u32) -> Result<u32, WriteError> {
        if depth >= MAX_DEPTH {
            return Err(WriteError::Unsupported);
        }
        let parts = error_parts(self.memory, object);
        let (trace, count) = parts.trace;
        let trace_bytes = count
            .checked_mul(TRACE_ENTRY_SIZE)
            .ok_or(WriteError::OutOfMemory)?;
        let payload = trace_bytes
            .checked_add(ERROR_WORDS * 4)
            .ok_or(WriteError::OutOfMemory)?;
        let offset = self.record(RECORD_ERROR, u32::from(parts.fatal), payload)?;
        let target = self.base + offset + RECORD_HEADER_SIZE + ERROR_WORDS * 4;
        self.memory.copy(target, trace, trace_bytes);
        let part = self.string_value(parts.kind)?;
        self.set_word(offset, 0, part);
        let part = self.string_value(parts.message)?;
        self.set_word(offset, 1, part);
        let part = self.value(parts.data, depth + 1)?;
        self.set_word(offset, 2, part);
        // An image stores the origin plus one, so that 0 can mean none.
        let origin = match parts.origin {
            0 => 0,
            origin => origin.checked_add(1).ok_or(WriteError::Unsupported)?,
        };
        self.set_word(offset, 3, origin);
        let cause = if parts.cause == ValueRef::NONE.raw() {
            0
        } else {
            self.value(parts.cause, depth + 1)?
        };
        self.set_word(offset, 4, cause);
        self.set_word(offset, 5, count);
        Ok(offset)
    }
}
