//! Runtime value payloads and dynamic value operations.

mod error;
pub(crate) mod list;
pub(crate) mod object;
mod string;

pub(crate) mod numeric;
pub(crate) mod operations;

pub(crate) use error::RuntimeError;
pub(crate) use list::RuntimeList;
pub(crate) use object::RuntimeObject;
pub(crate) use string::RuntimeString;

use alloc::boxed::Box;
use core::mem::{align_of, size_of};
use exs_value::ValueRef;

/// The allocated payload of one ExS value.
///
/// Primitive values remain inline. Complex runtime values must use boxed payloads so adding them
/// cannot increase the size of every allocated primitive value.
#[repr(u8)]
pub(crate) enum RtValue {
    /// The absence variant shared by Options and empty operations.
    None,
    /// A successful Option or Result payload.
    Ok(ValueRef),
    /// A structured language Error.
    Error(Box<RuntimeError>),
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
    #[allow(dead_code)]
    #[doc(hidden)]
    BoxedFutureValue(Box<()>),
}

/// Largest permitted inline payload in one runtime value table entry.
const MAX_INLINE_PAYLOAD_SIZE: usize = if size_of::<i64>() > size_of::<Box<()>>() {
    size_of::<i64>()
} else {
    size_of::<Box<()>>()
};

/// Largest permitted RtValue size after enum alignment.
const MAX_RT_VALUE_SIZE: usize =
    (MAX_INLINE_PAYLOAD_SIZE + size_of::<u8>() + align_of::<RtValue>() - 1)
        & !(align_of::<RtValue>() - 1);

// This fails at compile time when a complex RtValue variant is not boxed, which would make every
// value-table entry larger than the largest primitive or boxed payload plus enum overhead.
const _: () = assert!(size_of::<RtValue>() <= MAX_RT_VALUE_SIZE);
