//! Images: the format in which values cross the boundary of compiled modules and Rust guests,
//! laid out so a module adopts them without parsing.

mod format;
mod reader;
mod writer;

pub use format::*;

use crate::value::{BoundaryError, BoundaryLimits, ExsValue};

impl ExsValue {
    /// Writes this value as an image within `limits`.
    ///
    /// # Errors
    ///
    /// Returns the exceeded limit, or `Encode` for an image beyond 4 GiB.
    pub fn to_image_with_limits(
        &self,
        limits: BoundaryLimits,
    ) -> Result<alloc::vec::Vec<u8>, BoundaryError> {
        writer::write_image(self, limits)
    }

    /// Reads the value of the image `bytes`, which may be untrusted, within `limits`.
    ///
    /// # Errors
    ///
    /// Returns `Malformed` for an invalid image, or the exceeded limit.
    pub fn from_image_with_limits(
        bytes: &[u8],
        limits: BoundaryLimits,
    ) -> Result<Self, BoundaryError> {
        reader::read_image(bytes, limits)
    }
}
