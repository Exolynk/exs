//! Runtime value payloads and dynamic value operations.

mod bytes;
mod cell;
pub(crate) mod clone;
mod closure;
pub(crate) mod datetime;
mod error;
mod iterator;
pub(crate) mod list;
pub(crate) mod object;
mod string;

pub(crate) mod numeric;
pub(crate) mod operations;
pub(crate) mod reference;
mod view;

pub(crate) use bytes::RuntimeBytes;
pub(crate) use cell::RuntimeCellValue;
pub(crate) use closure::RuntimeClosure;
pub(crate) use error::RuntimeError;
pub(crate) use iterator::RuntimeIterator;
pub(crate) use list::RuntimeList;
pub(crate) use object::{RuntimeEnum, RuntimeObject, SharedName, same_name};
pub(crate) use string::RuntimeString;
pub(crate) use view::Value;

use alloc::boxed::Box;
use core::mem::{align_of, size_of};

/// The allocated payload of one ExS value.
///
/// None, Bool and Ints inside the `i32` range are immediate `ValueRef`s and never stored here;
/// `allocate` returns the immediate instead of storing such an Int. Complex runtime values must
/// use boxed payloads so adding them cannot increase the size of every allocated primitive value;
/// String and Bytes are stored inline because their payload already is one owning pointer.
#[repr(u8)]
pub(crate) enum RtValue {
    /// A structured language Error.
    Error(Box<RuntimeError>),
    /// A signed ExS integer outside the immediate `i32` range.
    Int(i64),
    /// An IEEE 754 binary64 floating-point number.
    Float(f64),
    /// An immutable UTF-8 string, stored inline as its single owned allocation.
    String(RuntimeString),
    /// An immutable raw-octet sequence, stored inline as its single owned allocation.
    Bytes(RuntimeBytes),
    /// A mutable ordered sequence.
    List(Box<RuntimeList>),
    /// A compiler-created lazy cursor over one immutable String or Bytes value.
    Iterator(Box<RuntimeIterator>),
    /// A mutable insertion-ordered string-keyed mapping.
    Object(Box<RuntimeObject>),
    /// Internal mutable storage shared by captured lexical bindings.
    Cell(Box<RuntimeCellValue>),
    /// A compiler-lifted function paired with captured lexical Cells.
    Closure(Box<RuntimeClosure>),
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
