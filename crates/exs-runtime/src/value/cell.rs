//! Mutable lexical storage shared by captured bindings.

use exs_value::ValueRef;

/// One GC-traced mutable binding slot retained by one or more closures.
pub(crate) struct RuntimeCellValue {
    /// The current source-visible value held by this lexical binding.
    pub(crate) value: ValueRef,
}

impl RuntimeCellValue {
    /// Creates one shared lexical binding initialized with the supplied value.
    pub(crate) const fn new(value: ValueRef) -> Self {
        Self { value }
    }
}
