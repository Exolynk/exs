//! Runtime closure payloads retaining compiler-generated function identities and captures.

use alloc::vec::Vec;

use exs_value::ValueRef;

/// One callable value paired with the Cells visible to its lifted function body.
pub(crate) struct RuntimeClosure {
    /// The compiler-generated lifted function identity selected by dynamic invocation.
    pub(crate) function_id: u32,
    /// Durable frame capacity required by the lifted function invocation.
    pub(crate) slot_count: u32,
    /// Number of source arguments accepted by the lifted function invocation.
    pub(crate) arity: u32,
    /// Captured lexical Cells in compiler-defined first-use order.
    pub(crate) captures: Vec<ValueRef>,
}

impl RuntimeClosure {
    /// Creates one callable value with its shared captured binding Cells.
    pub(crate) fn new(
        function_id: u32,
        slot_count: u32,
        arity: u32,
        captures: Vec<ValueRef>,
    ) -> Self {
        Self {
            function_id,
            slot_count,
            arity,
            captures,
        }
    }
}
