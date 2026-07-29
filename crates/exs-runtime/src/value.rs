//! Runtime-owned payloads addressed by `ValueRef`.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use core::str::Utf8Error;
use exs_value::ValueRef;

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

/// A mutable ordered sequence of runtime value references.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub struct RuntimeList {
    /// Elements in source-visible order.
    pub(crate) elements: Vec<ValueRef>,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
impl RuntimeList {
    /// Creates an empty runtime list.
    pub(crate) const fn new() -> Self {
        Self {
            elements: Vec::new(),
        }
    }
}

/// A mutable insertion-ordered mapping from string keys to runtime values.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub struct RuntimeObject {
    /// Key-value entries in insertion order.
    pub(crate) entries: Vec<(Box<str>, ValueRef)>,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
impl RuntimeObject {
    /// Creates an empty runtime object.
    pub(crate) const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
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
    /// A mutable ordered sequence.
    List(Box<RuntimeList>),
    /// A mutable insertion-ordered string-keyed mapping.
    Object(Box<RuntimeObject>),
    /// Reserved shape for future complex runtime values.
    ///
    /// Concrete future complex variants must follow this boxed form.
    #[doc(hidden)]
    BoxedFutureValue(Box<()>),
}
