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
use crate::bytes::{bytes_len, bytes_object};
use crate::enums::{enum_object, names};
use crate::errors::recoverable;
use crate::lists::{list_add, list_object};
use crate::memory::Memory;
use crate::numbers::{Arithmetic, Number, arithmetic, number, reference};
use crate::render::default_text;
use crate::vm::Vm;

/// Byte offset of [`StringObject::len`].
pub(crate) const LEN: u32 = offset_of!(StringObject, len) as u32;
/// Byte offset of the first text byte.
pub(crate) const DATA: u32 = size_of::<StringObject>() as u32;
/// Byte offset of the character count in the header word.
const CHARS: u32 = 4;

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

/// Borrows the text of the String at `object`, which is always valid UTF-8.
pub(crate) fn text(memory: &impl Memory, object: u32) -> &str {
    core::str::from_utf8(memory.bytes(object + DATA, byte_len(memory, object))).unwrap_or_default()
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

/// Creates one String holding `text`.
pub(crate) fn text_string(memory: &mut impl Memory, vm: u32, text: &str) -> Option<u64> {
    let len = u32::try_from(text.len()).ok()?;
    let chars = u32::try_from(text.chars().count()).ok()?;
    let object = allocate_string(memory, vm, len, chars)?;
    write_bytes(memory, object + DATA, text.as_bytes());
    string_reference(object)
}

/// Creates one String of `len` bytes and `chars` characters whose text `write` stores at the
/// address it receives.
///
/// Every value `write` reads must be rooted, because the String is allocated first.
pub(crate) fn copy_string_object<M: Memory>(
    memory: &mut M,
    vm: u32,
    len: u32,
    chars: u32,
    write: impl FnOnce(&mut M, u32),
) -> Option<u64> {
    let object = allocate_string(memory, vm, len, chars)?;
    write(memory, object + DATA);
    string_reference(object)
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
    /// The fixed ASCII default rendering of another value.
    Text(&'static str),
    /// The default rendering `Bytes(n)` of Bytes with `n` octets.
    Bytes(u32),
    /// The default rendering `identity::Variant` of an enum, from these two Strings.
    Enum {
        /// The enum's identity String.
        identity: u32,
        /// The variant-name String.
        name: u32,
    },
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
    #[inline(always)]
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
#[inline(always)]
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

/// Returns how one value renders in a formatted string: like a `+` operand where possible,
/// otherwise with its default text.
#[inline(always)]
fn default_part(memory: &impl Memory, vm: u32, raw: u64) -> Option<Part> {
    if let Some(part) = part(memory, raw) {
        return Some(part);
    }
    if let Some(object) = bytes_object(raw) {
        return Some(Part::Bytes(bytes_len(memory, object)));
    }
    if let Some((identity, name)) =
        enum_object(memory, raw).and_then(|object| names(memory, vm, object))
    {
        return Some(Part::Enum { identity, name });
    }
    Some(Part::Text(default_text(raw)))
}

/// Returns the byte and character length of one part.
fn part_size(memory: &impl Memory, part: Part) -> (u32, u32) {
    match part {
        Part::String(object) => (byte_len(memory, object), char_count(memory, object)),
        Part::Scalar(scalar) => {
            let len = scalar.len();
            (len, len)
        }
        Part::Text(text) => (text.len() as u32, text.len() as u32),
        Part::Bytes(len) => {
            let len = 7 + int_len(i64::from(len));
            (len, len)
        }
        Part::Enum { identity, name } => (
            byte_len(memory, identity) + 2 + byte_len(memory, name),
            char_count(memory, identity) + 2 + char_count(memory, name),
        ),
    }
}

/// Renders `slots[0..count]` into one String like v1's formatted strings: Strings and scalars
/// as with `+`, every other value with its default text, so formatting never fails.
///
/// The slots must be rooted. Returns `None` when memory is exhausted.
pub fn format(memory: &mut impl Memory, vm: u32, slots: u32, count: u32) -> Option<u64> {
    // Usually every part is a String or a scalar, which renders exactly like a `+` operand.
    if let Some((len, chars)) = measure(memory, slots, count, part) {
        return join(memory, vm, slots, count, len, chars, part);
    }
    let render = |memory: &_, raw| default_part(memory, vm, raw);
    let (len, chars) = measure(memory, slots, count, render)?;
    join(memory, vm, slots, count, len, chars, render)
}

/// Returns the byte and character length of `slots[0..count]` rendered by `render`, or `None`
/// when a part does not render or the text would not fit.
#[inline(always)]
fn measure<M: Memory>(
    memory: &M,
    slots: u32,
    count: u32,
    render: impl Fn(&M, u64) -> Option<Part>,
) -> Option<(u32, u32)> {
    let (mut len, mut chars) = (0_u32, 0_u32);
    for index in 0..count {
        let part = render(memory, memory.load_u64(slots + index * 8))?;
        let (part_len, part_chars) = part_size(memory, part);
        len = len.checked_add(part_len)?;
        chars = chars.checked_add(part_chars)?;
    }
    Some((len, chars))
}

/// Message of the TypeError for a `+` operand a String cannot render.
const NOT_RENDERABLE: &str = "String addition requires a String, Bool, Int, or Float right operand";

/// Evaluates `slots[0] + slots[1] + ... + slots[count - 1]` with the language's `+`, one
/// step per right operand like v1.
///
/// While the accumulated value is a String, every following renderable operand is appended and
/// the whole run is allocated once. A List accumulator builds a new List per operand; other
/// operands fall back to one numeric `+` at a time. A failing step yields an Error value that
/// the following steps add to, as in v1. When `positions` is not 0, it addresses one `u32`
/// source position per step, which becomes the origin of an Error that step creates;
/// otherwise Errors use the VM's current position.
///
/// The slots must be rooted; `slots[0]` is overwritten with intermediate results. Returns
/// `None` when memory is exhausted.
pub fn concat(
    memory: &mut impl Memory,
    vm: u32,
    slots: u32,
    count: u32,
    positions: u32,
) -> Option<u64> {
    let step_position = |memory: &mut _, step: u32| {
        if positions != 0 {
            let position = Memory::load_u32(memory, positions + (step - 1) * 4);
            Vm(vm).set_source_position(memory, position);
        }
    };
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
                len = len.checked_add(part_len)?;
                chars = chars.checked_add(part_chars)?;
                end += 1;
            }
            if end == index {
                let right = memory.load_u64(slots + index * 8);
                step_position(memory, index);
                index += 1;
                recoverable(memory, vm, "TypeError", NOT_RENDERABLE, right)?
            } else {
                let joined = join(memory, vm, slots, end, len, chars, part)?;
                index = end;
                joined
            }
        } else if list_object(accumulated).is_some() {
            let right = memory.load_u64(slots + index * 8);
            index += 1;
            list_add(memory, vm, accumulated, right)?
        } else {
            let right = memory.load_u64(slots + index * 8);
            step_position(memory, index);
            index += 1;
            arithmetic(memory, vm, Arithmetic::Add, accumulated, right)?
        };
        memory.store_u64(slots, result);
    }
    Some(memory.load_u64(slots))
}

/// Joins the parts `slots[0..end]`, rendered by `render` and measured as `len` bytes and
/// `chars` characters, into one String.
#[inline(always)]
fn join<M: Memory>(
    memory: &mut M,
    vm: u32,
    slots: u32,
    end: u32,
    len: u32,
    chars: u32,
    render: impl Fn(&M, u64) -> Option<Part>,
) -> Option<u64> {
    let object = allocate_string(memory, vm, len, chars)?;
    let mut position = object + DATA;
    for index in 0..end {
        // Allocation never moves objects, so the parts read here are the ones measured above.
        match render(memory, memory.load_u64(slots + index * 8))? {
            Part::String(source) => {
                let source_len = byte_len(memory, source);
                memory.copy(position, source + DATA, source_len);
                position += source_len;
            }
            Part::Scalar(scalar) => position = scalar.write(memory, position),
            Part::Text(text) => position = write_bytes(memory, position, text.as_bytes()),
            Part::Enum { identity, name } => {
                for (index, text) in [identity, name].into_iter().enumerate() {
                    if index > 0 {
                        position = write_bytes(memory, position, b"::");
                    }
                    let text_len = byte_len(memory, text);
                    memory.copy(position, text + DATA, text_len);
                    position += text_len;
                }
            }
            Part::Bytes(len) => {
                let mut digits = [0_u8; 20];
                position = write_bytes(memory, position, b"Bytes(");
                position = write_bytes(memory, position, int_text(i64::from(len), &mut digits));
                position = write_bytes(memory, position, b")");
            }
        }
    }
    string_reference(object)
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
