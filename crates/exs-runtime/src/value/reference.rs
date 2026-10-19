//! Runtime-owned parts of the `ValueRef` encoding: tagging and locating heap values.
//!
//! The shared bit layout and the immediate encoding live in `exs_value`; this module only adds
//! what needs runtime payloads, namely choosing the tag of a newly allocated heap value.

use core::num::NonZeroU32;

use exs_abi::{
    TYPE_BYTES, TYPE_ERROR, TYPE_FLOAT, TYPE_FN, TYPE_INT, TYPE_LIST, TYPE_OBJECT, TYPE_STRING,
};
use exs_value::ValueRef;
use exs_value::layout::{
    INT_BOXED, KIND_CELL, KIND_FUTURE, KIND_ITERATOR, object_type_extra, type_kind,
};

use crate::runtime::trap;
use crate::value::RtValue;

/// Returns the zero-based value-table position of a heap reference, or `None` for an immediate.
pub(crate) fn heap_index(reference: ValueRef) -> Option<usize> {
    if reference.is_immediate() {
        return None;
    }
    Some(payload_index(reference))
}

/// Returns the zero-based value-table position of a reference already known to be on the heap.
pub(crate) fn payload_index(reference: ValueRef) -> usize {
    match (reference.payload() as usize).checked_sub(1) {
        Some(index) => index,
        None => trap(),
    }
}

/// Builds the tagged reference of one heap value stored at a one-based table index.
pub(crate) fn heap_reference(value: &RtValue, index: NonZeroU32) -> ValueRef {
    let (kind, extra) = match value {
        RtValue::Error(_) => (type_kind(TYPE_ERROR), 0),
        RtValue::Int(_) => (type_kind(TYPE_INT), INT_BOXED),
        RtValue::Float(_) => (type_kind(TYPE_FLOAT), 0),
        RtValue::String(_) => (type_kind(TYPE_STRING), 0),
        RtValue::Bytes(_) => (type_kind(TYPE_BYTES), 0),
        RtValue::List(_) => (type_kind(TYPE_LIST), 0),
        RtValue::Iterator(_) => (KIND_ITERATOR, 0),
        RtValue::Object(object) => (type_kind(TYPE_OBJECT), object_type_extra(object.type_id)),
        RtValue::Cell(_) => (KIND_CELL, 0),
        RtValue::Closure(_) => (type_kind(TYPE_FN), 0),
        RtValue::BoxedFutureValue(_) => (KIND_FUTURE, 0),
    };
    match ValueRef::from_parts(kind, extra, index.get()) {
        Some(reference) => reference,
        None => trap(),
    }
}
