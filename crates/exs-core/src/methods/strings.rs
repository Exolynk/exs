//! String methods with v1's results, Errors and order of checks.
//!
//! Indices count characters. Texts are read through short borrows of memory that end before
//! anything is allocated; Strings never move, so offsets stay valid.

use exs_layout::CaseTables;
use exs_value::ValueRef;

use crate::errors::recoverable;
use crate::lists::{allocate_list, item_address, list_reference, set_len};
use crate::memory::Memory;
use crate::methods::numbers::{exact_int, type_error};
use crate::strings::{DATA, copy_string_object, string_object, text};
use crate::vm::Vm;

/// Unwraps a check, returning its Error value from the enclosing method.
macro_rules! check {
    ($result:expr) => {
        match $result {
            Ok(value) => value,
            Err(error) => return error,
        }
    };
}

/// Reads the String receiver of a method, or creates the TypeError `message`.
fn string_receiver(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
    message: &str,
) -> Result<u32, Option<u64>> {
    string_object(raw).ok_or_else(|| type_error(memory, vm, message, raw))
}

/// Copies `text[start..end]`, a range on character boundaries of the String at `object`, into
/// a new String.
fn substring(
    memory: &mut impl Memory,
    vm: u32,
    object: u32,
    start: usize,
    end: usize,
) -> Option<u64> {
    let chars = text(memory, object).get(start..end)?.chars().count() as u32;
    let len = (end - start) as u32;
    copy_string_object(memory, vm, len, chars, |memory, position| {
        memory.copy(position, object + DATA + start as u32, len);
    })
}

/// Reads a character index argument of `slice` like v1.
fn char_index(memory: &mut impl Memory, vm: u32, raw: u64) -> Result<usize, Option<u64>> {
    match exact_int(memory, raw) {
        Some(index) if index >= 0 => {
            u32::try_from(index)
                .map(|index| index as usize)
                .map_err(|_| {
                    recoverable(
                        memory,
                        vm,
                        "IndexError",
                        "slice requires a supported non-negative Int index",
                        raw,
                    )
                })
        }
        _ => Err(recoverable(
            memory,
            vm,
            "IndexError",
            "slice requires non-negative Int indexes",
            raw,
        )),
    }
}

/// Evaluates `slice(start, end)` on a String by character indices.
pub(crate) fn slice(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
    start: u64,
    stop: u64,
) -> Option<u64> {
    let start = check!(char_index(memory, vm, start));
    let stop = check!(char_index(memory, vm, stop));
    let object = check!(string_receiver(
        memory,
        vm,
        raw,
        "slice requires a String receiver"
    ));
    let value = text(memory, object);
    let count = value.chars().count();
    if start > stop || stop > count {
        return recoverable(
            memory,
            vm,
            "IndexError",
            "String slice range is outside the String bounds",
            raw,
        );
    }
    let offset = |index: usize| {
        value
            .char_indices()
            .nth(index)
            .map_or(value.len(), |(offset, _)| offset)
    };
    let (start, stop) = (offset(start), offset(stop));
    substring(memory, vm, object, start, stop)
}

/// A String predicate between a receiver and an argument.
#[derive(Clone, Copy)]
pub(crate) enum Relation {
    /// `contains`.
    Contains,
    /// `starts_with`.
    StartsWith,
    /// `ends_with`.
    EndsWith,
}

/// Evaluates `contains`, `starts_with` or `ends_with` on a String.
pub(crate) fn relation(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
    other: u64,
    relation: Relation,
) -> Option<u64> {
    let (receiver_message, argument_message) = match relation {
        Relation::Contains => (
            "contains requires a String receiver",
            "contains requires a String argument",
        ),
        Relation::StartsWith => (
            "starts_with requires a String receiver",
            "starts_with requires a String argument",
        ),
        Relation::EndsWith => (
            "ends_with requires a String receiver",
            "ends_with requires a String argument",
        ),
    };
    let object = check!(string_receiver(memory, vm, raw, receiver_message));
    let Some(other_object) = string_object(other) else {
        return type_error(memory, vm, argument_message, other);
    };
    let (value, needle) = (
        text(memory, object).as_bytes(),
        text(memory, other_object).as_bytes(),
    );
    let result = match relation {
        Relation::Contains => find(value, needle).is_some(),
        Relation::StartsWith => value.starts_with(needle),
        Relation::EndsWith => value.ends_with(needle),
    };
    Some(ValueRef::boolean(result).raw())
}

/// Which ends `trim` removes whitespace from.
#[derive(Clone, Copy)]
pub(crate) enum Trim {
    /// `trim`.
    Both,
    /// `trim_start`.
    Start,
    /// `trim_end`.
    End,
}

/// Evaluates `trim`, `trim_start` or `trim_end` with Unicode whitespace.
pub(crate) fn trim(memory: &mut impl Memory, vm: u32, raw: u64, ends: Trim) -> Option<u64> {
    let message = match ends {
        Trim::Both => "trim requires a String receiver",
        Trim::Start => "trim_start requires a String receiver",
        Trim::End => "trim_end requires a String receiver",
    };
    let object = check!(string_receiver(memory, vm, raw, message));
    let value = text(memory, object);
    let trimmed = match ends {
        Trim::Both => value.trim(),
        Trim::Start => value.trim_start(),
        Trim::End => value.trim_end(),
    };
    let start = trimmed.as_ptr() as usize - value.as_ptr() as usize;
    let end = start + trimmed.len();
    substring(memory, vm, object, start, end)
}

/// Reads the non-negative count of `repeat` like v1.
fn repeat_count(memory: &mut impl Memory, vm: u32, raw: u64) -> Result<u32, Option<u64>> {
    match exact_int(memory, raw) {
        Some(count) if count >= 0 => u32::try_from(count).map_err(|_| {
            recoverable(
                memory,
                vm,
                "ValueError",
                "repeat count is outside the supported range",
                raw,
            )
        }),
        _ => Err(type_error(
            memory,
            vm,
            "repeat requires a non-negative Int count",
            raw,
        )),
    }
}

/// Evaluates `repeat(count)` on a String.
pub(crate) fn repeat(memory: &mut impl Memory, vm: u32, raw: u64, count: u64) -> Option<u64> {
    let count = check!(repeat_count(memory, vm, count));
    let object = check!(string_receiver(
        memory,
        vm,
        raw,
        "repeat requires a String receiver"
    ));
    let value = text(memory, object);
    let (len, chars) = (value.len() as u32, value.chars().count() as u32);
    let Some(total) = len.checked_mul(count) else {
        return recoverable(
            memory,
            vm,
            "ValueError",
            "repeated String is too large",
            raw,
        );
    };
    copy_string_object(
        memory,
        vm,
        total,
        chars.saturating_mul(count),
        |memory, position| {
            for index in 0..count {
                memory.copy(position + index * len, object + DATA, len);
            }
        },
    )
}

/// Returns the byte offset of the first occurrence of `needle` in `haystack`, 0 for an empty
/// needle.
///
/// Matches of valid UTF-8 in valid UTF-8 always start on character boundaries, so this equals
/// `str::find`, without the panic paths of the standard searcher, whose formatting tables
/// would enlarge the template's data.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let last = haystack.len().checked_sub(needle.len())?;
    (0..=last).find(|start| haystack.get(*start..*start + needle.len()) == Some(needle))
}

/// Steps through the pieces between the matches of a delimiter, exactly like `str::split`:
/// an empty delimiter matches at every character boundary, including both ends.
///
/// The state is plain offsets, so callers may allocate between steps.
struct Splitter {
    /// Byte offset where the next piece starts.
    start: usize,
    /// Byte offset where the next match is searched, past the end when there is none.
    search: usize,
    /// Whether the last piece was returned.
    finished: bool,
}

impl Splitter {
    /// Starts splitting at the beginning of the text.
    const fn new() -> Self {
        Self {
            start: 0,
            search: 0,
            finished: false,
        }
    }

    /// Returns the byte range of the next piece of the String at `object` split by the String
    /// at `delimiter`.
    fn next(
        &mut self,
        memory: &impl Memory,
        object: u32,
        delimiter: u32,
    ) -> Option<(usize, usize)> {
        if self.finished {
            return None;
        }
        let value = text(memory, object);
        let needle = text(memory, delimiter);
        let found = value
            .as_bytes()
            .get(self.search..)
            .and_then(|rest| find(rest, needle.as_bytes()))
            .map(|offset| self.search + offset);
        let Some(found) = found else {
            self.finished = true;
            return Some((self.start, value.len()));
        };
        let piece = (self.start, found);
        if needle.is_empty() {
            self.start = found;
            let width = value
                .get(found..)
                .and_then(|rest| rest.chars().next())
                .map_or(1, char::len_utf8);
            self.search = found + width;
        } else {
            self.start = found + needle.len();
            self.search = self.start;
        }
        Some(piece)
    }
}

/// Evaluates `split(delimiter)` into a new List of Strings.
pub(crate) fn split(memory: &mut impl Memory, vm: u32, raw: u64, delimiter: u64) -> Option<u64> {
    let object = check!(string_receiver(
        memory,
        vm,
        raw,
        "split requires a String receiver"
    ));
    let Some(delimiter_object) = string_object(delimiter) else {
        return type_error(memory, vm, "split requires a String delimiter", delimiter);
    };
    let mut count = 0_u32;
    let mut counter = Splitter::new();
    while counter.next(memory, object, delimiter_object).is_some() {
        count += 1;
    }
    let list = allocate_list(memory, vm, count)?;
    let state = Vm(vm);
    let root = state.push_root(memory, list_reference(list)?)?;
    let mut splitter = Splitter::new();
    let mut outcome = Some(());
    for index in 0..count {
        let part = splitter
            .next(memory, object, delimiter_object)
            .and_then(|(start, end)| substring(memory, vm, object, start, end));
        let Some(part) = part else {
            outcome = None;
            break;
        };
        memory.store_u64(item_address(memory, list, index), part);
        set_len(memory, list, index + 1);
    }
    state.pop_roots(memory, root);
    outcome.and_then(|()| list_reference(list))
}

/// Evaluates `replace(from, to)`: every non-overlapping match replaced, like `str::replace`,
/// which equals joining the pieces of `split(from)` with `to`.
pub(crate) fn replace(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
    from: u64,
    to: u64,
) -> Option<u64> {
    let object = check!(string_receiver(
        memory,
        vm,
        raw,
        "replace requires a String receiver"
    ));
    let Some(from_object) = string_object(from) else {
        return type_error(memory, vm, "replace requires a String search value", from);
    };
    let Some(to_object) = string_object(to) else {
        return type_error(memory, vm, "replace requires a String replacement", to);
    };
    let (mut len, mut chars, mut pieces) = (0_u32, 0_u32, 0_u32);
    let mut measure = Splitter::new();
    while let Some((start, end)) = measure.next(memory, object, from_object) {
        let piece = text(memory, object).get(start..end)?;
        len = len.checked_add(piece.len() as u32)?;
        chars = chars.checked_add(piece.chars().count() as u32)?;
        pieces += 1;
    }
    let replacement = text(memory, to_object);
    let (to_len, to_chars) = (replacement.len() as u32, replacement.chars().count() as u32);
    let matches = pieces.saturating_sub(1);
    let len = len.checked_add(to_len.checked_mul(matches)?)?;
    let chars = chars.checked_add(to_chars.checked_mul(matches)?)?;
    copy_string_object(memory, vm, len, chars, |memory, mut position| {
        let mut writer = Splitter::new();
        let mut first = true;
        while let Some((start, end)) = writer.next(memory, object, from_object) {
            if !first {
                memory.copy(position, to_object + DATA, to_len);
                position += to_len;
            }
            first = false;
            let piece_len = (end - start) as u32;
            memory.copy(position, object + DATA + start as u32, piece_len);
            position += piece_len;
        }
    })
}

/// Returns the case tables of the program, which the compiler includes whenever it changes
/// the case of Strings.
fn case_tables(memory: &impl Memory, vm: u32) -> Option<CaseTables<'_>> {
    let address = Vm(vm).case_tables(memory);
    if address == 0 {
        return None;
    }
    let header = memory.bytes(address, 28);
    let end = <[u8; 4]>::try_from(header.get(24..28)?).ok()?;
    Some(CaseTables::new(
        memory.bytes(address, u32::from_le_bytes(end)),
    ))
}

/// Applies the final-sigma test of `str::to_lowercase` to characters in `characters`: skip
/// Case_Ignorable ones, then report whether the first other one is Cased.
fn ignorable_then_cased(
    tables: &CaseTables<'_>,
    mut characters: impl Iterator<Item = char>,
) -> bool {
    characters
        .find(|c| !tables.is_case_ignorable(*c))
        .is_some_and(|c| tables.is_cased(c))
}

/// Writes the case mapping of the character at byte `offset` of `value` into `out` and
/// returns its length, like `str::to_lowercase` and `str::to_uppercase`.
fn map_at(
    tables: &CaseTables<'_>,
    value: &str,
    offset: usize,
    c: char,
    upper: bool,
    out: &mut [char; 3],
) -> usize {
    if upper {
        return tables.uppercase(c, out);
    }
    if c == 'Σ' {
        let before = value.get(..offset).unwrap_or_default().chars().rev();
        let after = value.get(offset + 2..).unwrap_or_default().chars();
        let final_form =
            ignorable_then_cased(tables, before) && !ignorable_then_cased(tables, after);
        out[0] = if final_form { 'ς' } else { 'σ' };
        return 1;
    }
    tables.lowercase(c, out)
}

/// Evaluates `to_lowercase` or `to_uppercase` with full Unicode case mapping.
pub(crate) fn change_case(memory: &mut impl Memory, vm: u32, raw: u64, upper: bool) -> Option<u64> {
    let message = if upper {
        "to_uppercase requires a String receiver"
    } else {
        "to_lowercase requires a String receiver"
    };
    let object = check!(string_receiver(memory, vm, raw, message));
    let tables = case_tables(memory, vm)?;
    let value = text(memory, object);
    let (mut len, mut chars) = (0_u32, 0_u32);
    let mut out = ['\0'; 3];
    for (offset, c) in value.char_indices() {
        let count = map_at(&tables, value, offset, c, upper, &mut out);
        for mapped in out.iter().take(count) {
            len = len.checked_add(mapped.len_utf8() as u32)?;
            chars += 1;
        }
    }
    copy_string_object(memory, vm, len, chars, |memory, mut position| {
        let mut offset = 0;
        loop {
            // Each character is read and mapped through short borrows before it is written.
            let mut encoded = [0_u8; 12];
            let mut written = 0;
            let next = {
                let Some(tables) = case_tables(memory, vm) else {
                    return;
                };
                let value = text(memory, object);
                let Some(c) = value.get(offset..).and_then(|rest| rest.chars().next()) else {
                    return;
                };
                let mut out = ['\0'; 3];
                let count = map_at(&tables, value, offset, c, upper, &mut out);
                for mapped in out.iter().take(count) {
                    let mut buffer = [0_u8; 4];
                    let bytes = mapped.encode_utf8(&mut buffer).as_bytes();
                    if let Some(target) = encoded.get_mut(written..written + bytes.len()) {
                        target.copy_from_slice(bytes);
                    }
                    written += bytes.len();
                }
                offset + c.len_utf8()
            };
            for (index, byte) in (0_u32..).zip(encoded.iter().take(written)) {
                memory.store_u8(position + index, *byte);
            }
            position += written as u32;
            offset = next;
        }
    })
}
