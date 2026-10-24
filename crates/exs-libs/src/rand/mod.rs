//! The `rand` library: random Ints, Floats, texts, and Bytes from a secure random source.

mod functions;
mod library;
mod source;

pub(crate) use library::{HOST_FUNCTIONS, SOURCE};
