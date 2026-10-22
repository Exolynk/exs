//! Compact Unicode case-mapping tables and their lookups.
//!
//! `String::to_lowercase` and `to_uppercase` of the standard library carry about 29 KiB of
//! tables, and every initialized data byte of a module costs fuel on every execution. These
//! tables hold the same mappings in about 3.4 KiB of varint records, generated from the
//! standard library of the toolchain, so results stay identical to Rust's. The compiler
//! only adds them to programs that change the case of Strings.
//!
//! [`CASE_TABLES`] starts with seven `u32` offsets: of the lowercase runs, lowercase
//! multi-character mappings, uppercase runs, uppercase multi-character mappings,
//! Case_Ignorable ranges, Cased ranges, and the end.

pub use crate::case_tables::{CASE_TABLES, CASE_TABLES_UNICODE_VERSION};

/// A view of the case tables.
#[derive(Clone, Copy)]
pub struct CaseTables<'a> {
    /// The whole table blob.
    bytes: &'a [u8],
}

/// A reader of one varint record stream.
struct Reader<'a> {
    /// The remaining bytes.
    bytes: &'a [u8],
}

impl Reader<'_> {
    /// Reads one little-endian base-128 varint.
    fn varint(&mut self) -> Option<u64> {
        let mut value = 0_u64;
        let mut shift = 0;
        loop {
            let (&byte, rest) = self.bytes.split_first()?;
            self.bytes = rest;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
            shift += 7;
            if shift > 63 {
                return None;
            }
        }
    }

    /// Reads one zigzag-encoded signed varint.
    fn signed(&mut self) -> Option<i64> {
        let value = self.varint()?;
        Some((value >> 1) as i64 ^ -((value & 1) as i64))
    }

    /// Reads one byte.
    fn byte(&mut self) -> Option<u8> {
        let (&byte, rest) = self.bytes.split_first()?;
        self.bytes = rest;
        Some(byte)
    }
}

/// Returns the character `code + delta`, if it is one.
fn offset_char(code: u32, delta: i64) -> Option<char> {
    char::from_u32(u32::try_from(i64::from(code) + delta).ok()?)
}

impl<'a> CaseTables<'a> {
    /// Returns a view of the table blob `bytes`.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// Returns section `index` of the blob.
    fn section(&self, index: usize) -> Reader<'a> {
        let offset = |position: usize| -> Option<usize> {
            let start = position * 4;
            let word = <[u8; 4]>::try_from(self.bytes.get(start..start + 4)?).ok()?;
            Some(u32::from_le_bytes(word) as usize)
        };
        let bytes = offset(index)
            .zip(offset(index + 1))
            .and_then(|(start, end)| self.bytes.get(start..end))
            .unwrap_or_default();
        Reader { bytes }
    }

    /// Writes the mapping of `c` by the run section `runs` and the multi-character section
    /// `multi` into `out` and returns the number of characters, as `char::to_lowercase` or
    /// `char::to_uppercase` would yield them.
    fn map(&self, runs: usize, multi: usize, c: char, out: &mut [char; 3]) -> usize {
        let code = u32::from(c);
        let mut reader = self.section(multi);
        let mut previous = 0_u32;
        while let Some(gap) = reader.varint() {
            let Some(entry) = u32::try_from(gap)
                .ok()
                .and_then(|gap| previous.checked_add(gap))
            else {
                break;
            };
            previous = entry;
            let count = usize::from(reader.byte().unwrap_or(0));
            let mut mapped = [c; 3];
            for slot in mapped.iter_mut().take(count) {
                *slot = reader
                    .signed()
                    .and_then(|delta| offset_char(entry, delta))
                    .unwrap_or(c);
            }
            if entry == code {
                *out = mapped;
                return count.min(3);
            }
            if entry > code {
                break;
            }
        }
        let mut reader = self.section(runs);
        let mut previous = 0_u32;
        while let (Some(gap), Some(shape), Some(delta)) =
            (reader.varint(), reader.varint(), reader.signed())
        {
            let Some(start) = u32::try_from(gap)
                .ok()
                .and_then(|gap| previous.checked_add(gap))
            else {
                break;
            };
            if code < start {
                break;
            }
            let count = (shape >> 1) as u32;
            let stride = (shape & 1) as u32 + 1;
            let last = start + (count.saturating_sub(1)) * stride;
            if code <= last && (code - start) % stride == 0 {
                out[0] = offset_char(code, delta).unwrap_or(c);
                return 1;
            }
            previous = last + 1;
        }
        out[0] = c;
        1
    }

    /// Writes the lowercase mapping of `c` into `out` and returns its length.
    pub fn lowercase(&self, c: char, out: &mut [char; 3]) -> usize {
        self.map(0, 1, c, out)
    }

    /// Writes the uppercase mapping of `c` into `out` and returns its length.
    pub fn uppercase(&self, c: char, out: &mut [char; 3]) -> usize {
        self.map(2, 3, c, out)
    }

    /// Returns whether `c` lies in one of the ranges of section `index`.
    fn in_ranges(&self, index: usize, c: char) -> bool {
        let code = u32::from(c);
        let mut reader = self.section(index);
        let mut previous = 0_u32;
        while let (Some(gap), Some(length)) = (reader.varint(), reader.varint()) {
            let Some(start) = u32::try_from(gap)
                .ok()
                .and_then(|gap| previous.checked_add(gap))
            else {
                return false;
            };
            if code < start {
                return false;
            }
            let end = start.saturating_add(u32::try_from(length).unwrap_or(u32::MAX));
            if code < end {
                return true;
            }
            previous = end;
        }
        false
    }

    /// Returns whether `c` has the Unicode property Case_Ignorable.
    #[must_use]
    pub fn is_case_ignorable(&self, c: char) -> bool {
        self.in_ranges(4, c)
    }

    /// Returns whether `c` has the Unicode property Cased; only meaningful for characters
    /// that are not Case_Ignorable, the only ones the final-sigma rule asks about.
    #[must_use]
    pub fn is_cased(&self, c: char) -> bool {
        self.in_ranges(5, c)
    }

    /// Returns the length of the table blob, the number of bytes a module must hold.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns whether the table blob is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}
