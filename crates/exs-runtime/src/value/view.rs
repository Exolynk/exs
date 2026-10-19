//! Borrowed read-only view of one runtime value, immediate or heap-allocated.

use super::{
    RuntimeBytes, RuntimeCellValue, RuntimeClosure, RuntimeError, RuntimeIterator, RuntimeList,
    RuntimeObject, RuntimeString,
};

/// The read-only shape of one ExS value regardless of whether it is immediate or heap-allocated.
///
/// Immediate and boxed Ints both read as [`Value::Int`], so callers never observe the encoding.
#[derive(Clone, Copy)]
pub(crate) enum Value<'a> {
    /// The absence variant shared by Options and empty operations.
    None,
    /// A structured language Error.
    Error(&'a RuntimeError),
    /// A boolean value.
    Bool(bool),
    /// A signed ExS integer.
    Int(i64),
    /// An IEEE 754 binary64 floating-point number.
    Float(f64),
    /// An immutable UTF-8 string.
    String(&'a RuntimeString),
    /// An immutable raw-octet sequence.
    Bytes(&'a RuntimeBytes),
    /// A mutable ordered sequence.
    List(&'a RuntimeList),
    /// A compiler-created lazy cursor over one immutable String or Bytes value.
    Iterator(&'a RuntimeIterator),
    /// A mutable insertion-ordered string-keyed mapping.
    Object(&'a RuntimeObject),
    /// Internal mutable storage shared by captured lexical bindings.
    Cell(&'a RuntimeCellValue),
    /// A compiler-lifted function paired with captured lexical Cells.
    Closure(&'a RuntimeClosure),
    /// Reserved shape for future host-owned runtime values.
    Future,
}
