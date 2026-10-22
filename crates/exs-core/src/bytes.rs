//! Bytes values: immutable octet sequences in the String object layout.

use exs_layout::ObjectKind;
use exs_layout::TYPE_BYTES;
use exs_layout::ValueRef;
use exs_layout::value::layout::{KIND_SHIFT, type_kind};

use crate::allocator::allocate;
use crate::memory::Memory;
use crate::strings::{DATA, byte_len};

/// Returns the address of the Bytes behind one raw reference, or `None` for other values.
#[must_use]
pub fn bytes_object(raw: u64) -> Option<u32> {
    (raw >> KIND_SHIFT == u64::from(type_kind(TYPE_BYTES))).then_some(raw as u32)
}

/// Returns the raw reference of the Bytes at `object`.
pub(crate) fn bytes_reference(object: u32) -> Option<u64> {
    ValueRef::from_parts(type_kind(TYPE_BYTES), 0, object).map(ValueRef::raw)
}

/// Returns the number of octets of the Bytes at `object`.
pub(crate) fn bytes_len(memory: &impl Memory, object: u32) -> u32 {
    byte_len(memory, object)
}

/// Borrows the octets of the Bytes at `object`.
pub(crate) fn octets(memory: &impl Memory, object: u32) -> &[u8] {
    memory.bytes(object + DATA, bytes_len(memory, object))
}

/// Creates Bytes of `len` octets that `write` stores at the address it receives.
///
/// Every value `write` reads must be rooted, because the Bytes are allocated first.
pub(crate) fn new_bytes<M: Memory>(
    memory: &mut M,
    vm: u32,
    len: u32,
    write: impl FnOnce(&mut M, u32),
) -> Option<u64> {
    let object = allocate(memory, vm, DATA.checked_add(len)?, ObjectKind::Bytes)?;
    memory.store_u32(object + crate::strings::LEN, len);
    write(memory, object + DATA);
    bytes_reference(object)
}
