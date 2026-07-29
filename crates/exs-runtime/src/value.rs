//! Runtime-owned payloads addressed by `ValueRef`.

use alloc::boxed::Box;
use alloc::string::String;
use core::str::Utf8Error;

/// An immutable UTF-8 string owned by the ExS runtime.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub struct RuntimeString {
    contents: Box<str>,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
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

/// The allocated payload of one ExS value.
///
/// Primitive values remain inline. Complex runtime values must use boxed payloads so adding them
/// cannot increase the size of every allocated primitive value.
#[repr(u8)]
pub enum RtValue {
    /// The singular null value.
    Null,
    /// A boolean value.
    Bool(bool),
    /// A signed ExS integer.
    Int(i64),
    /// An IEEE 754 binary64 floating-point number.
    Float(f64),
    /// An immutable UTF-8 string.
    String(Box<RuntimeString>),
    /// Reserved shape for future complex runtime values.
    ///
    /// Concrete String, List, Object, and other complex variants must follow this boxed form.
    #[doc(hidden)]
    BoxedFutureValue(Box<()>),
}
