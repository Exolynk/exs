//! Immutable UTF-8 runtime strings.

use alloc::boxed::Box;
use alloc::string::String;
use core::cell::Cell;
use core::str::Utf8Error;

/// An immutable UTF-8 string owned by the ExS runtime.
pub(crate) struct RuntimeString {
    contents: Box<str>,
    /// Object entry position where this String was last found as a key.
    key_hint: Cell<u32>,
}

impl RuntimeString {
    /// Copies validated UTF-8 bytes into one immutable runtime string.
    pub(crate) fn from_utf8(bytes: &[u8]) -> Result<Self, Utf8Error> {
        let contents = core::str::from_utf8(bytes)?;
        Ok(Self {
            contents: Box::from(contents),
            key_hint: Cell::new(0),
        })
    }

    /// Converts one host-safe string into an immutable runtime string.
    pub(crate) fn from_string(value: String) -> Self {
        Self {
            contents: value.into_boxed_str(),
            key_hint: Cell::new(0),
        }
    }

    /// Returns the immutable UTF-8 contents.
    pub(crate) fn as_str(&self) -> &str {
        &self.contents
    }

    /// Returns the Object entry position where this String was last found as a key.
    pub(crate) const fn key_hint(&self) -> &Cell<u32> {
        &self.key_hint
    }
}
