//! Layout tests for the runtime payload enum.

use core::mem::{align_of, size_of};

use exs_runtime::RtValue;

const MAX_INLINE_PAYLOAD_SIZE: usize = max(size_of::<i64>(), size_of::<Box<()>>());
const MAX_RT_VALUE_SIZE: usize = align_up(
    MAX_INLINE_PAYLOAD_SIZE + size_of::<u8>(),
    align_of::<RtValue>(),
);

// This compile-time check fails when a future complex RtValue variant is stored inline instead of
// behind a Box, which would make every value-table entry larger.
const _: () = assert!(size_of::<RtValue>() <= MAX_RT_VALUE_SIZE);

/// Keeps complex runtime variants boxed so primitives do not pay for their payload size.
#[test]
fn rt_value_is_no_larger_than_one_boxed_or_numeric_payload() {
    assert!(size_of::<RtValue>() <= MAX_RT_VALUE_SIZE);
}

/// Aligns one size to its enclosing enum alignment.
/// Returns the larger layout size for the compile-time assertion.
const fn max(left: usize, right: usize) -> usize {
    if left > right { left } else { right }
}

/// Aligns one size to its enclosing enum alignment in a compile-time assertion.
const fn align_up(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}
