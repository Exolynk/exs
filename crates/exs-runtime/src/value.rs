//! Runtime-owned payloads addressed by `ValueRef`.

use alloc::boxed::Box;

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
    /// Reserved shape for future complex runtime values.
    ///
    /// Concrete String, List, Object, and other complex variants must follow this boxed form.
    #[doc(hidden)]
    BoxedFutureValue(Box<()>),
}
