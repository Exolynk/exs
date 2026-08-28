//! Immutable runtime byte sequences.

use alloc::boxed::Box;
use alloc::vec::Vec;

/// An immutable raw-octet sequence owned by the ExS runtime.
pub(crate) struct RuntimeBytes {
    contents: Box<[u8]>,
}

impl RuntimeBytes {
    /// Copies raw octets into one immutable runtime byte sequence.
    pub(crate) fn from_slice(bytes: &[u8]) -> Self {
        Self {
            contents: Box::from(bytes),
        }
    }

    /// Takes raw octets into one immutable runtime byte sequence.
    pub(crate) fn from_vec(bytes: Vec<u8>) -> Self {
        Self {
            contents: bytes.into_boxed_slice(),
        }
    }

    /// Returns the immutable raw octets.
    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.contents
    }
}
