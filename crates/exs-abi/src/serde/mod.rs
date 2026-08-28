//! Serde adapters for the host-safe ExS value tree.

use alloc::string::{String, ToString};
use core::fmt;

use crate::ExsValue;
use serde::Serialize;
use serde::de::DeserializeOwned;

mod bytes;
mod deserialize;
mod serialize;

/// A Serde conversion error raised while translating a Rust value and an ExS boundary value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SerdeError {
    message: String,
}

impl SerdeError {
    /// Creates one conversion error with the supplied explanation.
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SerdeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl core::error::Error for SerdeError {}

impl serde::ser::Error for SerdeError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        Self::new(message.to_string())
    }
}

impl serde::de::Error for SerdeError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        Self::new(message.to_string())
    }
}

impl ExsValue {
    /// Serializes one Rust value into the host-safe ExS value tree.
    ///
    /// Rust structs become ExS Objects, vectors become Lists, byte buffers become Bytes, and
    /// Rust enums become tagged Objects accepted by the generated ExS boundary decoder.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported map keys or a custom serializer failure.
    pub fn from_serialize<T: Serialize>(value: &T) -> Result<Self, SerdeError> {
        serialize::serialize(value)
    }

    /// Deserializes this host-safe ExS value into one owned Rust value.
    ///
    /// # Errors
    ///
    /// Returns an error when the ExS value does not satisfy the Rust target type.
    pub fn into_deserialize<T: DeserializeOwned>(self) -> Result<T, SerdeError> {
        deserialize::deserialize(self)
    }
}
