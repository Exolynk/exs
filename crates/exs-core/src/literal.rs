//! The ExS literal spelling of values, rendered natively.
//!
//! A value renders in two passes over the same walk: the first measures the text, the second
//! writes it into a String of that size. The walk gives up, and leaves the value to the
//! prelude's renderer, when it reaches a value whose type implements `Debug` or `ToString`,
//! or nests too deeply, which also bounds cycles.

use exs_layout::{TYPE_ENTRY_CUSTOM_RENDERING, ValueRef};

use crate::bytes::{bytes_len, bytes_object};
use crate::enums::{enum_object, field_address, names, payload_count};
use crate::errors::{error_object, error_parts};
use crate::lists::{item_address, len, list_object};
use crate::memory::Memory;
use crate::objects::{shape, untyped_object, value_address};
use crate::parts::{custom_kind, present};
use crate::render::{escaped_extra, write_escaped};
use crate::shapes::{key, key_count};
use crate::strings::{
    DATA, Scalar, byte_len, char_count, copy_string_object, scalar, string_object,
};
use crate::typed::{slot_address, slot_count, type_entry, type_id, typed_object};

/// Deepest nesting the native rendering follows before it leaves the value to the prelude.
const NATIVE_DEPTH: u32 = 32;

/// Returns the ExS literal spelling of `raw` as a new String, or None when the prelude must
/// render it. Returns `None` when memory is exhausted. `raw` must be rooted.
pub(crate) fn literal(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<u64> {
    let mut measure = Out {
        position: 0,
        bytes: 0,
        chars: 0,
        write: false,
    };
    if value(memory, vm, raw, 0, &mut measure).is_err() {
        return Some(ValueRef::NONE.raw());
    }
    copy_string_object(
        memory,
        vm,
        measure.bytes,
        measure.chars,
        |memory, target| {
            let mut out = Out {
                position: target,
                bytes: 0,
                chars: 0,
                write: true,
            };
            // The second pass walks the same unchanged values, so it cannot give up.
            let _ = value(memory, vm, raw, 0, &mut out);
        },
    )
}

/// The walk gave up: the prelude renders the value.
struct Fallback;

/// Where the walk's text goes: counted only, or also written at `position`.
struct Out {
    /// Address of the next byte when writing.
    position: u32,
    /// Bytes of text so far.
    bytes: u32,
    /// Characters of text so far.
    chars: u32,
    /// Whether the text is written.
    write: bool,
}

impl Out {
    /// Appends ASCII `text`.
    fn ascii(&mut self, memory: &mut impl Memory, text: &[u8]) {
        if self.write {
            for byte in text {
                memory.store_u8(self.position, *byte);
                self.position += 1;
            }
        }
        self.bytes += text.len() as u32;
        self.chars += text.len() as u32;
    }

    /// Appends the `len` bytes and `chars` characters of UTF-8 text at `source`.
    fn copy(&mut self, memory: &mut impl Memory, source: u32, len: u32, chars: u32) {
        if self.write {
            memory.copy(self.position, source, len);
            self.position += len;
        }
        self.bytes += len;
        self.chars += chars;
    }

    /// Appends the text at `source` with the escapes of a quoted literal.
    fn escaped(&mut self, memory: &mut impl Memory, source: u32, len: u32, chars: u32) {
        let (extra_bytes, extra_chars) = escaped_extra(memory, source, len);
        if self.write {
            self.position = write_escaped(memory, source, len, self.position);
        }
        self.bytes += len + extra_bytes;
        self.chars += chars + extra_chars;
    }

    /// Appends a Bool, Int or Float.
    fn scalar(&mut self, memory: &mut impl Memory, scalar: Scalar) {
        let len = scalar.len();
        if self.write {
            self.position = scalar.write(memory, self.position);
        }
        self.bytes += len;
        self.chars += len;
    }

    /// Appends the String at `object` quoted with escapes.
    fn quoted(&mut self, memory: &mut impl Memory, object: u32) {
        self.ascii(memory, b"\"");
        let (len, chars) = (byte_len(memory, object), char_count(memory, object));
        self.escaped(memory, object + DATA, len, chars);
        self.ascii(memory, b"\"");
    }
}

/// Renders `raw` at nesting `depth`.
fn value<M: Memory>(
    memory: &mut M,
    vm: u32,
    raw: u64,
    depth: u32,
    out: &mut Out,
) -> Result<(), Fallback> {
    if depth > NATIVE_DEPTH {
        return Err(Fallback);
    }
    if raw == ValueRef::NONE.raw() {
        out.ascii(memory, b"None");
        return Ok(());
    }
    if let Some(scalar) = scalar(memory, raw) {
        out.scalar(memory, scalar);
        return Ok(());
    }
    if let Some(object) = string_object(raw) {
        out.quoted(memory, object);
        return Ok(());
    }
    if let Some(object) = bytes_object(raw) {
        bytes(memory, object, out);
        return Ok(());
    }
    if let Some(list) = list_object(raw) {
        out.ascii(memory, b"[");
        for index in 0..len(memory, list) {
            if index > 0 {
                out.ascii(memory, b", ");
            }
            let item = memory.load_u64(item_address(memory, list, index));
            value(memory, vm, item, depth + 1, out)?;
        }
        out.ascii(memory, b"]");
        return Ok(());
    }
    if let Some(object) = error_object(raw) {
        let parts = error_parts(memory, object);
        out.ascii(memory, b"Error(");
        for (index, part) in [parts.kind, parts.message, parts.data]
            .into_iter()
            .enumerate()
        {
            if index > 0 {
                out.ascii(memory, b", ");
            }
            value(memory, vm, present(part), depth + 1, out)?;
        }
        out.ascii(memory, b")");
        return Ok(());
    }
    if custom_kind(memory, vm, raw, TYPE_ENTRY_CUSTOM_RENDERING) {
        return Err(Fallback);
    }
    if let Some(object) = typed_object(memory, raw) {
        return nominal(memory, vm, object, depth, out);
    }
    if let Some(object) = enum_object(memory, raw) {
        return enumeration(memory, vm, object, depth, out);
    }
    if let Some(object) = untyped_object(memory, raw) {
        return entries(memory, vm, object, depth, out);
    }
    match crate::render::default_text(raw) {
        text @ ("Fn" | "Cell") => {
            out.ascii(memory, text.as_bytes());
            Ok(())
        }
        _ => Err(Fallback),
    }
}

/// Renders the Bytes at `object` as `b"..."` for UTF-8 contents, else as
/// `Bytes::from_hex("...")`.
fn bytes(memory: &mut impl Memory, object: u32, out: &mut Out) {
    let source = object + DATA;
    let len = bytes_len(memory, object);
    if let Ok(text) = core::str::from_utf8(memory.bytes(source, len)) {
        let chars = text.chars().count() as u32;
        out.ascii(memory, b"b\"");
        out.escaped(memory, source, len, chars);
        out.ascii(memory, b"\"");
        return;
    }
    out.ascii(memory, b"Bytes::from_hex(\"");
    for index in 0..len {
        let byte = memory.load_u8(source + index);
        let digits = [
            b"0123456789abcdef"[(byte >> 4) as usize],
            b"0123456789abcdef"[(byte & 0xf) as usize],
        ];
        out.ascii(memory, &digits);
    }
    out.ascii(memory, b"\")");
}

/// Renders the nominal value at `object` as `Name { field: value, ... }`.
fn nominal<M: Memory>(
    memory: &mut M,
    vm: u32,
    object: u32,
    depth: u32,
    out: &mut Out,
) -> Result<(), Fallback> {
    let entry = type_entry(memory, vm, type_id(memory, object)).ok_or(Fallback)?;
    let name = string_object(entry.identity).ok_or(Fallback)?;
    out.copy(
        memory,
        name + DATA,
        byte_len(memory, name),
        char_count(memory, name),
    );
    let count = entry.field_count.min(slot_count(memory, object));
    if count == 0 {
        out.ascii(memory, b" {}");
        return Ok(());
    }
    out.ascii(memory, b" { ");
    for index in 0..count {
        if index > 0 {
            out.ascii(memory, b", ");
        }
        let field =
            string_object(memory.load_u64(entry.field_names + index * 8)).ok_or(Fallback)?;
        out.copy(
            memory,
            field + DATA,
            byte_len(memory, field),
            char_count(memory, field),
        );
        out.ascii(memory, b": ");
        let slot = present(memory.load_u64(slot_address(object, index)));
        value(memory, vm, slot, depth + 1, out)?;
    }
    out.ascii(memory, b" }");
    Ok(())
}

/// Renders the enum at `object` as `Type::Variant` or `Type::Variant(value, ...)`.
fn enumeration<M: Memory>(
    memory: &mut M,
    vm: u32,
    object: u32,
    depth: u32,
    out: &mut Out,
) -> Result<(), Fallback> {
    let (identity, name) = names(memory, vm, object).ok_or(Fallback)?;
    // The identity names the declaring source before the type: keep the type alone.
    let identity_len = byte_len(memory, identity);
    let identity_bytes = memory.bytes(identity + DATA, identity_len);
    let start = identity_bytes
        .windows(2)
        .rposition(|pair| pair == b"::")
        .map_or(0, |position| position as u32 + 2);
    let type_chars = core::str::from_utf8(identity_bytes.get(start as usize..).unwrap_or(&[]))
        .map_or(0, |text| text.chars().count() as u32);
    out.copy(
        memory,
        identity + DATA + start,
        identity_len - start,
        type_chars,
    );
    out.ascii(memory, b"::");
    out.copy(
        memory,
        name + DATA,
        byte_len(memory, name),
        char_count(memory, name),
    );
    let count = payload_count(memory, object);
    if count > 0 {
        out.ascii(memory, b"(");
        for index in 0..count {
            if index > 0 {
                out.ascii(memory, b", ");
            }
            let field = present(memory.load_u64(field_address(object, index)));
            value(memory, vm, field, depth + 1, out)?;
        }
        out.ascii(memory, b")");
    }
    Ok(())
}

/// Renders the untyped Object at `object` as `{ key: value, ... }`, quoting keys that are no
/// identifiers.
fn entries<M: Memory>(
    memory: &mut M,
    vm: u32,
    object: u32,
    depth: u32,
    out: &mut Out,
) -> Result<(), Fallback> {
    let shape = shape(memory, object);
    let count = key_count(memory, shape);
    if count == 0 {
        out.ascii(memory, b"{}");
        return Ok(());
    }
    out.ascii(memory, b"{ ");
    for index in 0..count {
        if index > 0 {
            out.ascii(memory, b", ");
        }
        let name = string_object(key(memory, shape, index)).ok_or(Fallback)?;
        let len = byte_len(memory, name);
        let text = memory.bytes(name + DATA, len);
        let identifier = text
            .first()
            .is_some_and(|first| first.is_ascii_alphabetic() || *first == b'_')
            && text
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
        if identifier {
            out.copy(memory, name + DATA, len, char_count(memory, name));
        } else {
            out.quoted(memory, name);
        }
        out.ascii(memory, b": ");
        let item = memory.load_u64(value_address(memory, object, index));
        value(memory, vm, item, depth + 1, out)?;
    }
    out.ascii(memory, b" }");
    Ok(())
}
