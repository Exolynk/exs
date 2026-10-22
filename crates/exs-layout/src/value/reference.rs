//! The 64-bit tagged `ValueRef` and access to its parts.

use core::num::NonZeroU64;

use crate::types::TYPE_OBJECT;

use super::layout::{EXTRA_SHIFT, KIND_SHIFT, type_kind};

/// An opaque 64-bit tagged reference to one `ExS` value.
///
/// Bits 63..48 hold exactly one kind flag, bits 47..32 hold kind-specific extra bits and bits
/// 31..0 hold the payload: either an immediate scalar or the linear-memory address of a heap
/// object. Because a kind flag is always set, a valid reference is never zero. The layout is
/// defined in [`super::layout`]; generated code and the core library create heap references,
/// and runners read them from module memory as an `i64` and may test the documented bits.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ValueRef(NonZeroU64);

impl ValueRef {
    /// Builds a reference from its kind flag, extra bits and payload.
    ///
    /// Returns `None` when `kind` is zero, because every valid reference carries a kind flag.
    #[inline]
    #[must_use]
    pub const fn from_parts(kind: u16, extra: u16, payload: u32) -> Option<Self> {
        if kind == 0 {
            return None;
        }
        let raw = ((kind as u64) << KIND_SHIFT) | ((extra as u64) << EXTRA_SHIFT) | payload as u64;
        match NonZeroU64::new(raw) {
            Some(raw) => Some(Self(raw)),
            None => None,
        }
    }

    /// Rebuilds a reference from its raw 64-bit form, or `None` for an empty (zero) slot.
    ///
    /// # Safety
    ///
    /// A nonzero `raw` must have been produced by [`ValueRef::raw`] in the current ExS runtime,
    /// and any heap index it carries must still identify a live runtime slot.
    #[must_use]
    pub const unsafe fn from_raw(raw: u64) -> Option<Self> {
        match NonZeroU64::new(raw) {
            Some(raw) => Some(Self(raw)),
            None => None,
        }
    }

    /// Returns the raw 64-bit form passed through the Wasm ABI.
    #[inline]
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0.get()
    }

    /// Returns the 16 kind-flag bits.
    #[inline]
    #[must_use]
    pub const fn kind(self) -> u16 {
        (self.0.get() >> KIND_SHIFT) as u16
    }

    /// Returns the 16 kind-specific extra bits.
    #[inline]
    #[must_use]
    pub const fn extra(self) -> u16 {
        (self.0.get() >> EXTRA_SHIFT) as u16
    }

    /// Returns the 32 payload bits.
    #[inline]
    #[must_use]
    pub const fn payload(self) -> u32 {
        self.0.get() as u32
    }

    /// Returns the source-visible `crate::TYPE_*` mask of this value, 0 for internal kinds.
    #[inline]
    #[must_use]
    pub const fn type_mask(self) -> u32 {
        self.kind() as u32 & crate::TYPE_ANY
    }

    /// Returns whether this value's type is one of the `crate::TYPE_*` bits in `mask`.
    #[inline]
    #[must_use]
    pub const fn has_type(self, mask: u32) -> bool {
        self.type_mask() & mask != 0
    }

    /// Returns the nominal type id encoded in an Object's extra bits.
    ///
    /// `None` means the value is no Object, an untyped Object, or an Object whose id exceeds
    /// [`super::layout::MAX_INLINE_TYPE_ID`] and is only known to the heap value.
    #[inline]
    #[must_use]
    pub const fn inline_type_id(self) -> Option<u32> {
        if self.kind() != type_kind(TYPE_OBJECT) {
            return None;
        }
        (self.extra() as u32).checked_sub(1)
    }
}
