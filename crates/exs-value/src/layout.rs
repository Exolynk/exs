//! Bit layout shared by the runtime and compiler-generated code.
//!
//! A raw `ValueRef` is `kind << 48 | extra << 32 | payload`. The kind field holds exactly one
//! flag: a source-visible `exs_abi::TYPE_*` bit or one runtime-internal kind above `TYPE_ANY`.
//! Generated code may test the kind and extra bits directly, so these values are part of the
//! compiler/runtime contract.

use exs_abi::{TYPE_ANY, TYPE_BOOL, TYPE_INT, TYPE_OBJECT};

/// Bit position of the 16 type-flag bits inside one raw `ValueRef`.
pub const KIND_SHIFT: u32 = 48;

/// Bit position of the 16 extra bits inside one raw `ValueRef`.
pub const EXTRA_SHIFT: u32 = 32;

/// Runtime-internal kind of compiler-created iterator cursors.
pub const KIND_ITERATOR: u16 = 1 << 10;
/// Runtime-internal kind of captured-binding Cells.
pub const KIND_CELL: u16 = 1 << 11;
/// Runtime-internal kind of the reserved future boxed value shape.
pub const KIND_FUTURE: u16 = 1 << 12;
/// Runtime-internal kind of the frames of resumable functions in runtime v2.
pub const KIND_FRAME: u16 = 1 << 13;

// Every source-visible type bit must fit the kind field, and internal kinds must never satisfy a
// source type contract.
const _: () = assert!(TYPE_ANY <= u16::MAX as u32);
const _: () = assert!(
    (KIND_ITERATOR | KIND_CELL | KIND_FUTURE | KIND_FRAME) as u32 & TYPE_ANY == 0,
    "internal kinds overlap source type bits"
);

/// Int extra bit marking a heap-boxed Int outside the `i32` range.
///
/// An Int reference without this bit is an immediate whose payload is the `i32` value.
pub const INT_BOXED: u16 = 1;

/// Largest nominal type id encoded in the Object extra bits.
///
/// Object references carry `type_id + 1` in their extra bits for ids up to this bound, so
/// generated code can test small nominal types without a runtime call. Extra 0 means the Object
/// is untyped or its id is too large, in which case only the heap value is authoritative.
pub const MAX_INLINE_TYPE_ID: u32 = u16::MAX as u32 - 1;

/// Converts one source-visible `exs_abi::TYPE_*` bit into the 16-bit kind field.
#[inline]
#[must_use]
pub const fn type_kind(type_bit: u32) -> u16 {
    type_bit as u16
}

/// Runtime v2: the Object extra bits of a typed Object or enum whose nominal type id is too
/// large to inline, so only its heap header names the type. Unlike extra bits 0, which mark
/// untyped Objects, they never match an untyped Object's inline tests.
pub const HEAP_TYPE_EXTRA: u16 = u16::MAX;

/// Runtime v2: the largest nominal type id carried in the extra bits, one below v1's limit,
/// as [`HEAP_TYPE_EXTRA`] is reserved.
pub const NOMINAL_MAX_INLINE_TYPE_ID: u32 = u16::MAX as u32 - 2;

/// Runtime v2: returns the Object extra bits of a value of the nominal type `type_id`.
#[inline]
#[must_use]
pub const fn nominal_extra(type_id: u32) -> u16 {
    if type_id <= NOMINAL_MAX_INLINE_TYPE_ID {
        (type_id + 1) as u16
    } else {
        HEAP_TYPE_EXTRA
    }
}

/// Runtime v2: returns the upper 32 bits of every value of the nominal type `type_id`, or
/// `None` when the id is not inlined and only the heap header names it.
#[inline]
#[must_use]
pub const fn nominal_tag(type_id: u32) -> Option<u32> {
    if type_id > NOMINAL_MAX_INLINE_TYPE_ID {
        return None;
    }
    Some(tag(type_kind(TYPE_OBJECT), nominal_extra(type_id)))
}

/// Returns the Object extra bits encoding one nominal type id, or 0 when it is not inlined.
#[inline]
#[must_use]
pub const fn object_type_extra(type_id: Option<u32>) -> u16 {
    match type_id {
        Some(type_id) if type_id <= MAX_INLINE_TYPE_ID => (type_id + 1) as u16,
        _ => 0,
    }
}

/// Returns the upper 32 bits (kind and extra) of references with these parts.
///
/// Generated code compares `raw >> EXTRA_SHIFT` against such a tag to identify a kind and its
/// extra bits with one test.
#[inline]
#[must_use]
pub const fn tag(kind: u16, extra: u16) -> u32 {
    ((kind as u32) << (KIND_SHIFT - EXTRA_SHIFT)) | extra as u32
}

/// Upper 32 bits shared by every Bool, all of which are immediates.
pub const BOOL_TAG: u32 = tag(type_kind(TYPE_BOOL), 0);

/// Upper 32 bits shared by every immediate (unboxed) Int.
pub const SMALL_INT_TAG: u32 = tag(type_kind(TYPE_INT), 0);

/// Returns the upper 32 bits of every Object carrying `type_id`, or `None` when the id is too
/// large to be encoded and only the heap value can answer.
#[inline]
#[must_use]
pub const fn object_tag(type_id: u32) -> Option<u32> {
    if type_id > MAX_INLINE_TYPE_ID {
        return None;
    }
    Some(tag(
        type_kind(TYPE_OBJECT),
        object_type_extra(Some(type_id)),
    ))
}
