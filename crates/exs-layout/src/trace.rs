//! Where each object kind stores references, for the collector.

use core::mem::{offset_of, size_of};

use crate::header::ObjectKind;
use crate::objects::{
    BufferObject, CellObject, ClosureObject, EnumObject, ErrorObject, FrameObject, ListObject,
    ObjectObject, ShapeObject, TypedObject,
};

/// The `ValueRef` fields of one object kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueSlots {
    /// The kind holds no `ValueRef`s.
    None,
    /// `ValueRef`s at these fixed byte offsets.
    Fixed(&'static [u32]),
    /// A run of consecutive `ValueRef` slots starting at `first`, whose count is the `u32`
    /// at byte offset `count` (4 addresses the header word).
    Counted {
        /// Byte offset of the `u32` slot count.
        count: u32,
        /// Byte offset of the first slot.
        first: u32,
    },
}

/// The references of one object kind: `ValueRef` slots plus `u32` address fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TraceLayout {
    /// Raw 64-bit references.
    pub values: ValueSlots,
    /// Byte offsets of `u32` fields holding addresses of other heap objects, 0 meaning none.
    pub addresses: &'static [u32],
}

/// Byte offset of the header word, which holds the slot count of Buffers.
const HEADER_WORD: u32 = 4;

/// Converts one field offset into the `u32` used by layouts.
const fn offset(value: usize) -> u32 {
    value as u32
}

/// Address fields of Lists.
const LIST_ADDRESSES: &[u32] = &[offset(offset_of!(ListObject, items))];
/// Address fields of typed Objects.
const TYPED_OBJECT_ADDRESSES: &[u32] = &[offset(offset_of!(TypedObject, extra))];
/// Address fields of untyped Objects.
const OBJECT_ADDRESSES: &[u32] = &[
    offset(offset_of!(ObjectObject, shape)),
    offset(offset_of!(ObjectObject, values)),
];
/// Address fields of shapes.
const SHAPE_ADDRESSES: &[u32] = &[
    offset(offset_of!(ShapeObject, parent)),
    offset(offset_of!(ShapeObject, keys)),
    offset(offset_of!(ShapeObject, first_child)),
    offset(offset_of!(ShapeObject, next_sibling)),
];
/// `ValueRef` fields of Cells.
const CELL_VALUES: &[u32] = &[offset(offset_of!(CellObject, value))];
/// `ValueRef` fields of Errors.
const ERROR_VALUES: &[u32] = &[
    offset(offset_of!(ErrorObject, kind)),
    offset(offset_of!(ErrorObject, message)),
    offset(offset_of!(ErrorObject, data)),
    offset(offset_of!(ErrorObject, cause)),
];
/// Address fields of Errors.
const ERROR_ADDRESSES: &[u32] = &[offset(offset_of!(ErrorObject, trace))];
/// Address fields of Frames.
const FRAME_ADDRESSES: &[u32] = &[offset(offset_of!(FrameObject, caller))];

/// Returns where objects of `kind` store references.
#[must_use]
pub const fn trace_layout(kind: ObjectKind) -> TraceLayout {
    const NONE: &[u32] = &[];
    match kind {
        ObjectKind::Free
        | ObjectKind::String
        | ObjectKind::Bytes
        | ObjectKind::BigInt
        | ObjectKind::Float => TraceLayout {
            values: ValueSlots::None,
            addresses: NONE,
        },
        ObjectKind::Buffer => TraceLayout {
            values: ValueSlots::Counted {
                count: HEADER_WORD,
                first: offset(size_of::<BufferObject>()),
            },
            addresses: NONE,
        },
        ObjectKind::List => TraceLayout {
            values: ValueSlots::None,
            addresses: LIST_ADDRESSES,
        },
        ObjectKind::TypedObject => TraceLayout {
            values: ValueSlots::Counted {
                count: offset(offset_of!(TypedObject, slot_count)),
                first: offset(size_of::<TypedObject>()),
            },
            addresses: TYPED_OBJECT_ADDRESSES,
        },
        ObjectKind::Object => TraceLayout {
            values: ValueSlots::None,
            addresses: OBJECT_ADDRESSES,
        },
        ObjectKind::Shape => TraceLayout {
            values: ValueSlots::None,
            addresses: SHAPE_ADDRESSES,
        },
        ObjectKind::Enum => TraceLayout {
            values: ValueSlots::Counted {
                count: offset(offset_of!(EnumObject, field_count)),
                first: offset(size_of::<EnumObject>()),
            },
            addresses: NONE,
        },
        ObjectKind::Cell => TraceLayout {
            values: ValueSlots::Fixed(CELL_VALUES),
            addresses: NONE,
        },
        ObjectKind::Closure => TraceLayout {
            values: ValueSlots::Counted {
                count: offset(offset_of!(ClosureObject, capture_count)),
                first: offset(size_of::<ClosureObject>()),
            },
            addresses: NONE,
        },
        ObjectKind::Error => TraceLayout {
            values: ValueSlots::Fixed(ERROR_VALUES),
            addresses: ERROR_ADDRESSES,
        },
        ObjectKind::Frame => TraceLayout {
            values: ValueSlots::Counted {
                count: offset(offset_of!(FrameObject, slot_count)),
                first: offset(size_of::<FrameObject>()),
            },
            addresses: FRAME_ADDRESSES,
        },
    }
}
