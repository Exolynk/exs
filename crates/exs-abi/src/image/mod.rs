//! Images: the format in which values cross the boundary of runtime v2 modules and Rust
//! guests, laid out so a module adopts them without parsing.

mod format;
mod reader;
mod writer;

pub use format::*;

use crate::cbor::{CborError, CborLimits, ExsValue};

impl ExsValue {
    /// Writes this value as an image, keeping `limits` as if it were encoded as CBOR.
    ///
    /// # Errors
    ///
    /// Returns the exceeded limit, or `Encode` for an image beyond 4 GiB.
    pub fn to_image_with_limits(
        &self,
        limits: CborLimits,
    ) -> Result<alloc::vec::Vec<u8>, CborError> {
        writer::write_image(self, limits)
    }

    /// Reads the value of the image `bytes`, which may be untrusted, within `limits`.
    ///
    /// # Errors
    ///
    /// Returns `Malformed` for an invalid image, or the exceeded limit.
    pub fn from_image_with_limits(bytes: &[u8], limits: CborLimits) -> Result<Self, CborError> {
        reader::read_image(bytes, limits)
    }
}
