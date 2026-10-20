//! String values: creation, rendering of scalars, concatenation, equality and ordering.
//!
//! A String's header word holds its character count, so the length needs no scan.

use core::fmt::Write;
use core::mem::{offset_of, size_of};

use exs_abi::TYPE_STRING;
use exs_layout::{ObjectKind, StringObject};
use exs_value::layout::{KIND_SHIFT, type_kind};
use exs_value::{Immediate, ValueRef};

use crate::allocator::allocate;
use crate::memory::Memory;
use crate::numbers::{Arithmetic, Number, NumericError, arithmetic, number, reference};

/// Byte offset of [`StringObject::len`].
const LEN: u32 = offset_of!(StringObject, len) as u32;
/// Byte offset of the first text byte.
pub(crate) const DATA: u32 = size_of::<StringObject>() as u32;
/// Byte offset of the character count in the header word.
const CHARS: u32 = 4;

/// Why a String operation produced no value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StringError {
    /// An operand of `+` is neither a String nor a Bool, Int or Float in a String context.
    NotRenderable,
    /// A numeric step of the operation failed.
    Numeric(NumericError),
    /// The heap could not hold the result.
    OutOfMemory,
}

/// Returns the address of the String behind one raw reference, or `None` for other values.
#[must_use]
pub fn string_object(raw: u64) -> Option<u32> {
    // Strings are never immediates, so their kind bits alone identify them.
    (raw >> KIND_SHIFT == u64::from(type_kind(TYPE_STRING))).then_some(raw as u32)
}

/// Returns the byte length of the String at `object`.
pub(crate) fn byte_len(memory: &impl Memory, object: u32) -> u32 {
    memory.load_u32(object + LEN)
}

/// Returns the character count of the String at `object`.
pub(crate) fn char_count(memory: &impl Memory, object: u32) -> u32 {
    memory.load_u32(object + CHARS)
}

/// Allocates an uninitialized String of `len` bytes and `chars` characters.
fn allocate_string(memory: &mut impl Memory, vm: u32, len: u32, chars: u32) -> Option<u32> {
    let object = allocate(memory, vm, DATA.checked_add(len)?, ObjectKind::String)?;
    memory.store_u32(object + CHARS, chars);
    memory.store_u32(object + LEN, len);
    Some(object)
}

/// Returns the raw reference of the String at `object`.
pub(crate) fn string_reference(object: u32) -> Option<u64> {
    ValueRef::from_parts(type_kind(TYPE_STRING), 0, object).map(ValueRef::raw)
}

/// Creates one String from `len` bytes of validated UTF-8 at `source`.
pub(crate) fn copy_string(
    memory: &mut impl Memory,
    vm: u32,
    source: u32,
    len: u32,
    chars: u32,
) -> Option<u64> {
    let object = allocate_string(memory, vm, len, chars)?;
    memory.copy(object + DATA, source, len);
    string_reference(object)
}

/// Counts the bytes written through `core::fmt`.
#[derive(Default)]
struct Counter {
    /// Bytes written.
    bytes: u32,
}

impl Write for Counter {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        self.bytes += text.len() as u32;
        Ok(())
    }
}

/// Writes text produced by `core::fmt` into linear memory.
struct MemoryWriter<'memory, M: Memory> {
    /// The target memory.
    memory: &'memory mut M,
    /// Address of the next byte.
    position: u32,
}

impl<M: Memory> Write for MemoryWriter<'_, M> {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        for byte in text.bytes() {
            self.memory.store_u8(self.position, byte);
            self.position += 1;
        }
        Ok(())
    }
}

/// One `+` operand rendered into a String: an existing String or a scalar's text.
#[derive(Clone, Copy)]
enum Part {
    /// The String at this address.
    String(u32),
    /// A Bool, Int or Float rendered with `core::fmt`.
    Scalar(Scalar),
}

/// A scalar value that renders as text.
#[derive(Clone, Copy)]
enum Scalar {
    /// A Bool.
    Bool(bool),
    /// An Int.
    Int(i64),
    /// A Float, rendered like Rust's `{}`.
    Float(f64),
}

impl Scalar {
    /// Returns the byte length of the rendered text, which is ASCII and so also its character
    /// count.
    fn len(self) -> u32 {
        match self {
            Self::Bool(true) => 4,
            Self::Bool(false) => 5,
            Self::Int(value) => int_len(value),
            Self::Float(value) => {
                let mut counter = Counter::default();
                let _ = write!(counter, "{value}");
                counter.bytes
            }
        }
    }

    /// Writes the rendered text at `position` and returns the address after it.
    fn write(self, memory: &mut impl Memory, position: u32) -> u32 {
        match self {
            Self::Bool(flag) => {
                write_bytes(memory, position, if flag { b"true" } else { b"false" })
            }
            Self::Int(value) => {
                let mut digits = [0_u8; 20];
                write_bytes(memory, position, int_text(value, &mut digits))
            }
            Self::Float(value) => {
                let mut writer = MemoryWriter { memory, position };
                let _ = write!(writer, "{value}");
                writer.position
            }
        }
    }
}

/// Writes `bytes` at `position` and returns the address after them.
fn write_bytes(memory: &mut impl Memory, position: u32, bytes: &[u8]) -> u32 {
    for (offset, byte) in (0_u32..).zip(bytes) {
        memory.store_u8(position + offset, *byte);
    }
    position + bytes.len() as u32
}

/// Returns the length of the decimal text of one Int.
const fn int_len(value: i64) -> u32 {
    let mut remaining = value.unsigned_abs();
    let mut len = if value < 0 { 2 } else { 1 };
    while remaining >= 10 {
        remaining /= 10;
        len += 1;
    }
    len
}

/// Writes the decimal text of one Int into the end of `digits` and returns it.
fn int_text(value: i64, digits: &mut [u8; 20]) -> &[u8] {
    let mut position = digits.len();
    let mut remaining = value.unsigned_abs();
    loop {
        position -= 1;
        // The remainder is always below ten.
        digits[position] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    if value < 0 {
        position -= 1;
        digits[position] = b'-';
    }
    &digits[position..]
}

/// Returns how one value renders as the right operand of String `+`, or `None` when it cannot.
fn part(memory: &impl Memory, raw: u64) -> Option<Part> {
    if let Some(object) = string_object(raw) {
        return Some(Part::String(object));
    }
    match reference(raw)?.immediate() {
        Some(Immediate::Bool(flag)) => return Some(Part::Scalar(Scalar::Bool(flag))),
        Some(Immediate::None) => return None,
        _ => {}
    }
    Some(Part::Scalar(match number(memory, raw)? {
        Number::Int(value) => Scalar::Int(value),
        Number::Float(value) => Scalar::Float(value),
    }))
}

/// Returns the byte and character length of one part.
fn part_size(memory: &impl Memory, part: Part) -> (u32, u32) {
    match part {
        Part::String(object) => (byte_len(memory, object), char_count(memory, object)),
        Part::Scalar(scalar) => {
            let len = scalar.len();
            (len, len)
        }
    }
}

/// Evaluates `slots[0] + slots[1] + ... + slots[count - 1]` with the language's `+`.
///
/// While the accumulated value is a String, every following renderable operand is appended and
/// the whole run is allocated once. Other operands fall back to one `+` at a time. The slots
/// must be rooted; `slots[0]` is overwritten with intermediate results.
///
/// # Errors
///
/// Returns why a `+` produced no value.
pub fn concat(
    memory: &mut impl Memory,
    vm: u32,
    slots: u32,
    count: u32,
) -> Result<u64, StringError> {
    let mut index = 1;
    while index < count {
        let accumulated = memory.load_u64(slots);
        let result = if let Some(first) = string_object(accumulated) {
            // Measure the run of renderable operands that follows the String.
            let (mut len, mut chars) = (byte_len(memory, first), char_count(memory, first));
            let mut end = index;
            while end < count {
                let Some(part) = part(memory, memory.load_u64(slots + end * 8)) else {
                    break;
                };
                let (part_len, part_chars) = part_size(memory, part);
                len = len.checked_add(part_len).ok_or(StringError::OutOfMemory)?;
                chars = chars
                    .checked_add(part_chars)
                    .ok_or(StringError::OutOfMemory)?;
                end += 1;
            }
            if end == index {
                return Err(StringError::NotRenderable);
            }
            let joined = join(memory, vm, slots, end, len, chars)?;
            index = end;
            joined
        } else {
            let right = memory.load_u64(slots + index * 8);
            index += 1;
            if string_object(right).is_some() {
                // A number cannot be added to a String.
                return Err(StringError::Numeric(NumericError::NotNumeric));
            }
            arithmetic(memory, vm, Arithmetic::Add, accumulated, right)
                .map_err(StringError::Numeric)?
        };
        memory.store_u64(slots, result);
    }
    Ok(memory.load_u64(slots))
}

/// Joins the String in `slots[0]` with the renderable operands `slots[1..end]`, measured as
/// `len` bytes and `chars` characters, into one String.
fn join(
    memory: &mut impl Memory,
    vm: u32,
    slots: u32,
    end: u32,
    len: u32,
    chars: u32,
) -> Result<u64, StringError> {
    let object = allocate_string(memory, vm, len, chars).ok_or(StringError::OutOfMemory)?;
    let mut position = object + DATA;
    for index in 0..end {
        // Allocation never moves objects, so the parts read here are the ones measured above.
        match part(memory, memory.load_u64(slots + index * 8)).ok_or(StringError::NotRenderable)? {
            Part::String(source) => {
                let source_len = byte_len(memory, source);
                memory.copy(position, source + DATA, source_len);
                position += source_len;
            }
            Part::Scalar(scalar) => position = scalar.write(memory, position),
        }
    }
    string_reference(object).ok_or(StringError::OutOfMemory)
}

/// Returns whether two Strings hold the same text.
pub(crate) fn strings_equal(memory: &impl Memory, left: u32, right: u32) -> bool {
    if left == right {
        return true;
    }
    let len = byte_len(memory, left);
    len == byte_len(memory, right)
        && (0..len).all(|offset| {
            memory.load_u8(left + DATA + offset) == memory.load_u8(right + DATA + offset)
        })
}

/// Compares two Strings byte by byte.
pub(crate) fn compare_strings(memory: &impl Memory, left: u32, right: u32) -> core::cmp::Ordering {
    let (left_len, right_len) = (byte_len(memory, left), byte_len(memory, right));
    for offset in 0..left_len.min(right_len) {
        let ordering = memory
            .load_u8(left + DATA + offset)
            .cmp(&memory.load_u8(right + DATA + offset));
        if ordering.is_ne() {
            return ordering;
        }
    }
    left_len.cmp(&right_len)
}
