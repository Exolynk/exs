#![no_std]

//! Shared `ExS` values and their type metadata.

/// The inclusive lower bound of an `ExS` integer.
pub const MIN_INT: i64 = -(1_i64 << 55);
/// The inclusive upper bound of an `ExS` integer.
pub const MAX_INT: i64 = (1_i64 << 55) - 1;

/// The opaque, tagged runtime carrier for every `ExS` value.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Value(u64);

const TAG_MASK: u64 = 0xff;
const INT_TAG: u64 = 0x10;
const FALSE_TAG: u64 = 0x01;
const TRUE_TAG: u64 = 0x02;

impl Value {
    /// Constructs an `ExS` integer when `value` lies in the supported range.
    #[must_use]
    pub const fn int(value: i64) -> Option<Self> {
        if is_valid_int(value) {
            Some(Self((value.cast_unsigned() << 8) | INT_TAG))
        } else {
            None
        }
    }

    /// Constructs an `ExS` boolean.
    #[must_use]
    pub const fn bool(value: bool) -> Self {
        Self(if value { TRUE_TAG } else { FALSE_TAG })
    }

    /// Returns the raw runtime representation for Wasm ABI calls.
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Reconstructs a value from a runtime ABI representation.
    #[must_use]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// Reads this value as an integer.
    #[must_use]
    pub const fn as_int(self) -> Option<i64> {
        if self.0 & TAG_MASK == INT_TAG {
            Some(self.0.cast_signed() >> 8)
        } else {
            None
        }
    }

    /// Reads this value as a boolean.
    #[must_use]
    pub const fn as_bool(self) -> Option<bool> {
        match self.0 {
            FALSE_TAG => Some(false),
            TRUE_TAG => Some(true),
            _ => None,
        }
    }
}

/// Returns whether `value` fits in `ExS`'s 56-bit integer range.
#[must_use]
pub const fn is_valid_int(value: i64) -> bool {
    value >= MIN_INT && value <= MAX_INT
}
