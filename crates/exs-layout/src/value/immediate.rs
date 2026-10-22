//! Values encoded entirely inside one `ValueRef`.
//!
//! None, Bool and every Int inside the `i32` range are immediates and never allocate. An Int
//! that fits an `i32` is always immediate, so equal immediates have equal raw references.

use crate::types::{TYPE_BOOL, TYPE_INT, TYPE_NONE};

use super::layout::{INT_BOXED, type_kind};
use super::reference::ValueRef;

/// The scalar decoded from one immediate reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Immediate {
    /// The None value.
    None,
    /// One Bool value.
    Bool(bool),
    /// One Int value inside the `i32` range.
    Int(i32),
}

/// Builds one immediate reference from a statically nonzero kind.
const fn immediate(type_bit: u32, payload: u32) -> ValueRef {
    match ValueRef::from_parts(type_kind(type_bit), 0, payload) {
        Some(value) => value,
        None => panic!("immediate kinds are nonzero"),
    }
}

impl ValueRef {
    /// The immediate None value.
    pub const NONE: Self = immediate(TYPE_NONE, 0);
    /// The immediate `false` value.
    pub const FALSE: Self = immediate(TYPE_BOOL, 0);
    /// The immediate `true` value.
    pub const TRUE: Self = immediate(TYPE_BOOL, 1);

    /// Returns the immediate Bool value.
    #[inline]
    #[must_use]
    pub const fn boolean(flag: bool) -> Self {
        if flag { Self::TRUE } else { Self::FALSE }
    }

    /// Returns the immediate Int when `value` fits in an `i32`, otherwise `None`.
    #[inline]
    #[must_use]
    pub const fn small_int(value: i64) -> Option<Self> {
        if value < i32::MIN as i64 || value > i32::MAX as i64 {
            return None;
        }
        // The Int kind is nonzero, so this never fails, and unlike `immediate` it cannot panic.
        Self::from_parts(type_kind(TYPE_INT), 0, (value as i32).cast_unsigned())
    }

    /// Decodes this reference when it is an immediate, or returns `None` for a heap value.
    #[inline]
    #[must_use]
    pub const fn immediate(self) -> Option<Immediate> {
        let kind = self.kind();
        if kind == type_kind(TYPE_NONE) {
            Some(Immediate::None)
        } else if kind == type_kind(TYPE_BOOL) {
            Some(Immediate::Bool(self.payload() != 0))
        } else if kind == type_kind(TYPE_INT) && self.extra() & INT_BOXED == 0 {
            Some(Immediate::Int(self.payload().cast_signed()))
        } else {
            None
        }
    }

    /// Returns whether this reference is an immediate and owns no heap slot.
    #[inline]
    #[must_use]
    pub const fn is_immediate(self) -> bool {
        self.immediate().is_some()
    }
}
