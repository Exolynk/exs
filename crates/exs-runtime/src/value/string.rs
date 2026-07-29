//! Immutable UTF-8 runtime strings.

use alloc::boxed::Box;
use alloc::string::String;
use core::str::Utf8Error;

/// An immutable UTF-8 string owned by the ExS runtime.
pub(crate) struct RuntimeString {
    contents: Box<str>,
}

impl RuntimeString {
    /// Copies validated UTF-8 bytes into one immutable runtime string.
    pub(crate) fn from_utf8(bytes: &[u8]) -> Result<Self, Utf8Error> {
        let contents = core::str::from_utf8(bytes)?;
        Ok(Self {
            contents: Box::from(contents),
        })
    }

    /// Converts one host-safe string into an immutable runtime string.
    pub(crate) fn from_string(value: String) -> Self {
        Self {
            contents: value.into_boxed_str(),
        }
    }

    /// Returns the immutable UTF-8 contents.
    pub(crate) fn as_str(&self) -> &str {
        &self.contents
    }
}
