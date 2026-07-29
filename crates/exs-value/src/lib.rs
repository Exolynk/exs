#![no_std]

//! Shared `ExS` values and their type metadata.

use core::num::NonZeroU32;

/// The inclusive lower bound of an `ExS` integer.
pub const MIN_INT: i64 = -(1_i64 << 55);
/// The inclusive upper bound of an `ExS` integer.
pub const MAX_INT: i64 = (1_i64 << 55) - 1;

/// An opaque reference to one runtime-allocated `ExS` value.
///
/// The contained index addresses one slot in the runtime-owned value table. Only `exs-runtime`
/// may construct or dereference it; compilers and runners pass it through their internal Wasm
/// ABI unchanged.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ValueRef(NonZeroU32);

impl ValueRef {
    /// Constructs a reference from a runtime-owned nonzero value-table index.
    ///
    /// # Safety
    ///
    /// `index` must identify a live `RtValue` slot owned by the current ExS runtime.
    #[must_use]
    pub const unsafe fn from_runtime_index(index: NonZeroU32) -> Self {
        Self(index)
    }

    /// Returns the runtime-owned value-table index.
    ///
    /// This is intended only for runtime dereferencing and must not cross the host boundary.
    #[must_use]
    pub const fn runtime_index(self) -> u32 {
        self.0.get()
    }
}

/// Returns whether `value` fits in `ExS`'s 56-bit integer range.
#[must_use]
pub const fn is_valid_int(value: i64) -> bool {
    value >= MIN_INT && value <= MAX_INT
}
