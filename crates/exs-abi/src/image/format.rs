//! The image format of values crossing the boundary: inputs and host-call responses for
//! compiled modules, and every value both ways for Rust guests.
//!
//! An image is a sequence of 8-byte aligned records. It starts with an 8-byte header, the
//! [`IMAGE_MAGIC`] followed by the offset of the root record. Every record starts with a `u32`
//! kind and a `u32` argument; offsets are relative to the image start, and offset 0, the
//! header, means "none" where a record allows it. All numbers are little-endian.
//!
//! | Kind | Argument | Payload |
//! |---|---|---|
//! | [`RECORD_NONE`] | 0 | — |
//! | [`RECORD_BOOL`] | 0 or 1 | — |
//! | [`RECORD_INT`] | 0 | `i64` |
//! | [`RECORD_FLOAT`] | 0 | `f64` bits |
//! | [`RECORD_STRING`] | byte length | `u32` character count, `u32` 0, the UTF-8 bytes |
//! | [`RECORD_BYTES`] | length | the octets |
//! | [`RECORD_LIST`] | element count | one `u32` offset per element |
//! | [`RECORD_OBJECT`] | entry count | per entry a `u32` key offset (a String record) and a `u32` value offset |
//! | [`RECORD_TYPED`] | field count | `u32` identity offset (a String record), per field a `u32` name offset (a String record) and a `u32` value offset |
//! | [`RECORD_ENUM`] | field count | `u32` identity and variant offsets (String records), one `u32` offset per field |
//! | [`RECORD_ERROR`] | 1 if fatal | `u32` kind, message and data offsets, `u32` origin + 1 or 0, `u32` cause offset or 0, `u32` trace count, the `u64` trace entries |
//!
//! Payloads are padded to 8 bytes. Writers store valid UTF-8 with its character count, so a
//! compiled module copies Strings without validating them; readers of untrusted images
//! validate them.

/// Magic number of an image, `EXSI` in ASCII.
pub const IMAGE_MAGIC: u32 = u32::from_le_bytes(*b"EXSI");
/// Byte size of the image header.
pub const IMAGE_HEADER_SIZE: u32 = 8;
/// Byte size of a record's kind and argument.
pub const RECORD_HEADER_SIZE: u32 = 8;

/// The None record.
pub const RECORD_NONE: u32 = 0;
/// A Bool record.
pub const RECORD_BOOL: u32 = 1;
/// An Int record.
pub const RECORD_INT: u32 = 2;
/// A Float record.
pub const RECORD_FLOAT: u32 = 3;
/// A String record.
pub const RECORD_STRING: u32 = 4;
/// A Bytes record.
pub const RECORD_BYTES: u32 = 5;
/// A List record.
pub const RECORD_LIST: u32 = 6;
/// An untyped Object record.
pub const RECORD_OBJECT: u32 = 7;
/// An enum record.
pub const RECORD_ENUM: u32 = 8;
/// An Error record.
pub const RECORD_ERROR: u32 = 9;
/// A record of a nominal Object type's value.
pub const RECORD_TYPED: u32 = 10;
