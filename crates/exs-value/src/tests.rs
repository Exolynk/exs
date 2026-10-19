//! Unit tests for the `ValueRef` layout and immediate encoding.

use exs_abi::{TYPE_BOOL, TYPE_ERROR, TYPE_INT, TYPE_NONE, TYPE_OBJECT};

use crate::layout::{
    BOOL_TAG, EXTRA_SHIFT, INT_BOXED, KIND_ITERATOR, MAX_INLINE_TYPE_ID, SMALL_INT_TAG, object_tag,
    object_type_extra, type_kind,
};
use crate::{Immediate, ValueRef};

/// Every part round-trips at its boundaries without leaking into neighbouring bits.
#[test]
fn parts_round_trip_at_boundaries() {
    for (kind, extra, payload) in [
        (1, 0, 0),
        (1 << 15, u16::MAX, u32::MAX),
        (1 << 3, 1, i32::MIN.cast_unsigned()),
        (1 << 3, 0, i32::MAX.cast_unsigned()),
        (1 << 7, u16::MAX, 1),
    ] {
        let Some(value) = ValueRef::from_parts(kind, extra, payload) else {
            panic!("valid parts were rejected");
        };
        assert_eq!(value.kind(), kind);
        assert_eq!(value.extra(), extra);
        assert_eq!(value.payload(), payload);
        assert_eq!(unsafe { ValueRef::from_raw(value.raw()) }, Some(value));
    }
}

/// A reference without a kind flag is rejected, so zero stays the empty-slot marker.
#[test]
fn zero_kind_is_rejected() {
    assert_eq!(ValueRef::from_parts(0, u16::MAX, u32::MAX), None);
    assert_eq!(unsafe { ValueRef::from_raw(0) }, None);
}

/// The kind flags occupy the top 16 bits so generated code can test them with one shift.
#[test]
fn kind_occupies_top_bits() {
    let Some(value) = ValueRef::from_parts(1 << 9, 0, 0) else {
        panic!("valid parts were rejected");
    };
    assert_eq!(value.raw(), 1 << (48 + 9));
}

/// Nominal ids map to nonzero extra bits up to the bound and to 0 beyond it.
#[test]
fn object_type_extra_boundaries() {
    assert_eq!(object_type_extra(None), 0);
    assert_eq!(object_type_extra(Some(0)), 1);
    assert_eq!(object_type_extra(Some(MAX_INLINE_TYPE_ID)), u16::MAX);
    assert_eq!(object_type_extra(Some(MAX_INLINE_TYPE_ID + 1)), 0);
    assert_eq!(object_type_extra(Some(u32::MAX)), 0);
    assert_eq!(object_tag(MAX_INLINE_TYPE_ID + 1), None);
}

/// Encoded Object ids are read back, and untyped or large-id Objects report none.
#[test]
fn reads_inline_type_ids() {
    let object = |type_id| {
        ValueRef::from_parts(type_kind(TYPE_OBJECT), object_type_extra(type_id), 1)
            .unwrap_or_else(|| panic!("valid parts were rejected"))
    };
    for type_id in [0, 1, MAX_INLINE_TYPE_ID] {
        let value = object(Some(type_id));
        assert_eq!(value.inline_type_id(), Some(type_id));
        assert_eq!(
            Some((value.raw() >> EXTRA_SHIFT) as u32),
            object_tag(type_id)
        );
    }
    assert_eq!(object(None).inline_type_id(), None);
    assert_eq!(object(Some(MAX_INLINE_TYPE_ID + 1)).inline_type_id(), None);
    assert_eq!(ValueRef::TRUE.inline_type_id(), None);
}

/// Immediates decode to their scalars, and Ints switch to heap form exactly outside `i32`.
#[test]
fn encodes_immediates() {
    assert_eq!(ValueRef::NONE.immediate(), Some(Immediate::None));
    assert_eq!(
        ValueRef::boolean(false).immediate(),
        Some(Immediate::Bool(false))
    );
    assert_eq!(
        ValueRef::boolean(true).immediate(),
        Some(Immediate::Bool(true))
    );
    for value in [0, -1, i32::MIN, i32::MAX] {
        let reference = ValueRef::small_int(i64::from(value));
        assert_eq!(
            reference.and_then(ValueRef::immediate),
            Some(Immediate::Int(value))
        );
    }
    assert_eq!(ValueRef::small_int(i64::from(i32::MAX) + 1), None);
    assert_eq!(ValueRef::small_int(i64::from(i32::MIN) - 1), None);
    let boxed = ValueRef::from_parts(type_kind(TYPE_INT), INT_BOXED, 1);
    assert_eq!(boxed.map(ValueRef::is_immediate), Some(false));
}

/// Immediate tags and raw forms match the documented layout used by generated code.
#[test]
fn immediate_raw_forms_match_layout() {
    assert_eq!(ValueRef::NONE.raw(), 1 << 48);
    assert_eq!(ValueRef::FALSE.raw(), 1 << 50);
    assert_eq!(ValueRef::TRUE.raw(), (1 << 50) | 1);
    assert_eq!(
        ValueRef::small_int(-1).map(ValueRef::raw),
        Some((1 << 51) | 0xFFFF_FFFF)
    );
    assert_eq!((ValueRef::TRUE.raw() >> EXTRA_SHIFT) as u32, BOOL_TAG);
    let zero = ValueRef::small_int(0).map(|value| (value.raw() >> EXTRA_SHIFT) as u32);
    assert_eq!(zero, Some(SMALL_INT_TAG));
}

/// Type masks expose only source-visible type bits.
#[test]
fn type_masks_hide_internal_kinds() {
    assert!(ValueRef::NONE.has_type(TYPE_NONE));
    assert!(!ValueRef::TRUE.has_type(TYPE_INT | TYPE_ERROR));
    assert!(ValueRef::TRUE.has_type(TYPE_BOOL));
    let iterator = ValueRef::from_parts(KIND_ITERATOR, 0, 1);
    assert_eq!(iterator.map(ValueRef::type_mask), Some(0));
}
