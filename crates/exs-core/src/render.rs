//! Text of values: the default text formatted strings use for values that cannot render like
//! `+` operands, and the ExS literal spelling `debug` and the prelude's renderer build on.

use exs_layout::value::layout::{KIND_CELL, KIND_SHIFT};
use exs_layout::{TYPE_ERROR, TYPE_FN, TYPE_LIST, TYPE_NONE, TYPE_OBJECT, TYPE_UNIT, ValueRef};

use crate::enums::{enum_object, names};
use crate::lists::{allocate_list, item_address, list_reference, set_len};
use crate::memory::Memory;
use crate::strings::{DATA, byte_len, char_count, copy_string_object, string_object};
use crate::typed::{slot_count, type_entry, type_id, typed_object};

/// Returns the default `ToString` text of a value that is no String, Bool, Int or Float.
pub(crate) fn default_text(raw: u64) -> &'static str {
    let kind = (raw >> KIND_SHIFT) as u32;
    if kind & TYPE_NONE != 0 {
        "None"
    } else if kind & TYPE_UNIT != 0 {
        "Unit"
    } else if kind & TYPE_ERROR != 0 {
        "Error"
    } else if kind & TYPE_LIST != 0 {
        "[...]"
    } else if kind & TYPE_OBJECT != 0 {
        "{...}"
    } else if kind & TYPE_FN != 0 {
        "Fn"
    } else if kind & u32::from(KIND_CELL) != 0 {
        "Cell"
    } else {
        // No other kind of value exists in yet.
        "None"
    }
}

/// Returns the ExS literal spelling of a value as `debug` renders it: Strings quoted with
/// escapes, Bytes as a `b"..."` literal or `Bytes::from_hex(...)`, numbers, Bools, None and Unit as
/// their source spelling, and composites part by part. Returns None when the prelude must
/// render the value, because a part's type implements `Debug` or `ToString` or it nests too
/// deeply, and `None` when memory is exhausted. `raw` must be rooted.
pub fn debug_text(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<u64> {
    crate::literal::literal(memory, vm, raw)
}

/// Returns the escape of one ASCII byte inside a quoted literal, if it needs one.
fn escape(byte: u8) -> Option<&'static [u8]> {
    match byte {
        b'"' => Some(b"\\\""),
        b'\\' => Some(b"\\\\"),
        b'\n' => Some(b"\\n"),
        b'\r' => Some(b"\\r"),
        b'\t' => Some(b"\\t"),
        b'\0' => Some(b"\\0"),
        _ => None,
    }
}

/// Returns the bytes and characters escaping adds to the `len` bytes of text at `source`.
pub(crate) fn escaped_extra(memory: &impl Memory, source: u32, len: u32) -> (u32, u32) {
    let mut extra = 0;
    for index in 0..len {
        let byte = memory.load_u8(source + index);
        if let Some(escape) = escape(byte) {
            extra += escape.len() as u32 - 1;
        } else if byte < 0x20 || byte == 0x7f {
            // `\u{XX}` replaces one byte.
            extra += 5;
        }
    }
    (extra, extra)
}

/// Writes the `len` bytes of text at `source` to `target` with escapes and returns the end.
pub(crate) fn write_escaped(memory: &mut impl Memory, source: u32, len: u32, target: u32) -> u32 {
    let mut position = target;
    let mut put = |memory: &mut _, byte: u8| {
        Memory::store_u8(memory, position, byte);
        position += 1;
    };
    for index in 0..len {
        let byte = memory.load_u8(source + index);
        if let Some(escape) = escape(byte) {
            for byte in escape {
                put(memory, *byte);
            }
        } else if byte < 0x20 || byte == 0x7f {
            for byte in [
                b'\\',
                b'u',
                b'{',
                b"0123456789abcdef"[(byte >> 4) as usize],
                b"0123456789abcdef"[(byte & 0xf) as usize],
                b'}',
            ] {
                put(memory, byte);
            }
        } else {
            put(memory, byte);
        }
    }
    position
}

/// Returns an Object key as it renders inside an Object literal: an identifier as it is,
/// any other key quoted. `raw` must be a rooted String.
pub fn key_text(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<u64> {
    let Some(object) = string_object(raw) else {
        return debug_text(memory, vm, raw);
    };
    let bytes = memory.bytes(object + DATA, byte_len(memory, object));
    let identifier = bytes
        .first()
        .is_some_and(|first| first.is_ascii_alphabetic() || *first == b'_')
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
    if identifier {
        Some(raw)
    } else {
        debug_text(memory, vm, raw)
    }
}

/// Returns the name a nominal value or enum renders with: the type's source name, or
/// `Type::Variant` for an enum. Returns None for other values.
pub fn type_name(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<u64> {
    if let Some(object) = typed_object(memory, raw) {
        return Some(
            type_entry(memory, vm, type_id(memory, object))
                .filter(|entry| string_object(entry.identity).is_some())
                .map_or(ValueRef::NONE.raw(), |entry| entry.identity),
        );
    }
    let Some((identity, name)) =
        enum_object(memory, raw).and_then(|object| names(memory, vm, object))
    else {
        return Some(ValueRef::NONE.raw());
    };
    // The identity names the declaring source before the type: keep the type alone.
    let identity_bytes = memory.bytes(identity + DATA, byte_len(memory, identity));
    let start = identity_bytes
        .windows(2)
        .rposition(|pair| pair == b"::")
        .map_or(0, |position| position as u32 + 2);
    let type_len = byte_len(memory, identity) - start;
    let type_chars = core::str::from_utf8(&identity_bytes[start as usize..])
        .map_or(type_len, |text| text.chars().count() as u32);
    let name_len = byte_len(memory, name);
    copy_string_object(
        memory,
        vm,
        type_len + 2 + name_len,
        type_chars + 2 + char_count(memory, name),
        |memory, target| {
            memory.copy(target, identity + DATA + start, type_len);
            memory.store_u8(target + type_len, b':');
            memory.store_u8(target + type_len + 1, b':');
            memory.copy(target + type_len + 2, name + DATA, name_len);
        },
    )
}

/// Returns a new List of the field names of a nominal value in declaration order, or None
/// for other values.
pub fn field_names(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<u64> {
    let Some(object) = typed_object(memory, raw) else {
        return Some(ValueRef::NONE.raw());
    };
    let Some(entry) = type_entry(memory, vm, type_id(memory, object)) else {
        return Some(ValueRef::NONE.raw());
    };
    let count = entry.field_count.min(slot_count(memory, object));
    let list = allocate_list(memory, vm, count)?;
    for index in 0..count {
        let name = memory.load_u64(entry.field_names + index * 8);
        memory.store_u64(item_address(memory, list, index), name);
    }
    set_len(memory, list, count);
    list_reference(list)
}
