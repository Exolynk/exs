//! The direct boundary of runtime v2 modules.
//!
//! The runner reads results and host-call arguments from the module's memory instead of CBOR
//! the module encoded, and passes inputs and host-call responses as images the module adopts
//! without parsing.

mod image;
mod reader;

pub(crate) use image::write_image;
pub(crate) use reader::{ReadError, encode, is_direct, result};
