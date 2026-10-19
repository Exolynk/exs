//! Internal lazy iterators for immutable runtime collections.

use exs_value::ValueRef;

/// Mutable cursor state for one compiler-created iterator.
pub(crate) enum RuntimeIterator {
    /// Iterates UTF-8 scalar values without materializing a List of Strings.
    String {
        /// Immutable source String retained for the iterator lifetime.
        source: ValueRef,
        /// Byte offset of the next UTF-8 scalar.
        next_byte: usize,
    },
    /// Iterates immutable Bytes without materializing a List of Int values.
    Bytes {
        /// Immutable source Bytes retained for the iterator lifetime.
        source: ValueRef,
        /// Octet offset of the next yielded value.
        next_index: usize,
    },
    /// Iterates an unshared prelude Range natively without reading its Object fields.
    Range {
        /// Next candidate item.
        current: i64,
        /// Range bound.
        end: i64,
        /// Signed distance between items.
        step: i64,
        /// Whether `end` itself is yielded.
        inclusive: bool,
        /// Whether the Range is exhausted.
        done: bool,
    },
}

impl RuntimeIterator {
    /// Returns the source value that must remain reachable during iteration, if any.
    pub(crate) const fn source(&self) -> Option<ValueRef> {
        match self {
            Self::String { source, .. } | Self::Bytes { source, .. } => Some(*source),
            Self::Range { .. } => None,
        }
    }
}
