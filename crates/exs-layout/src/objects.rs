//! Fixed parts of every heap object kind.
//!
//! Variable-length parts (String bytes, `ValueRef` slots) directly follow the fixed part, which
//! always ends 8-byte aligned. Address fields hold `u32` linear-memory addresses of other heap
//! objects, 0 meaning none; `ValueRef` fields hold raw 64-bit references, 0 meaning empty.

use crate::header::ObjectHeader;

/// Immutable UTF-8 text; `header.word` is its character count. `len` bytes follow the fixed
/// part. Bytes objects share the layout with `header.word` unused.
#[repr(C)]
pub struct StringObject {
    /// Common header.
    pub header: ObjectHeader,
    /// Byte length of the text.
    pub len: u32,
    /// Content hash, or 0 while not computed.
    pub hash: u32,
}

/// Immutable octets in the same layout as [`StringObject`].
pub type BytesObject = StringObject;

/// An Int outside the immediate `i32` range.
#[repr(C)]
pub struct BigIntObject {
    /// Common header.
    pub header: ObjectHeader,
    /// The Int value.
    pub value: i64,
}

/// An IEEE 754 binary64 number.
#[repr(C)]
pub struct FloatObject {
    /// Common header.
    pub header: ObjectHeader,
    /// The number.
    pub value: f64,
}

/// Growable `ValueRef` storage; `header.word` is the slot count. The slots follow.
#[repr(C)]
pub struct BufferObject {
    /// Common header.
    pub header: ObjectHeader,
}

/// A mutable ordered sequence over one [`BufferObject`] whose slot count is its capacity.
#[repr(C)]
pub struct ListObject {
    /// Common header.
    pub header: ObjectHeader,
    /// Number of used slots.
    pub len: u32,
    /// Address of the element Buffer.
    pub items: u32,
}

/// An Object of a nominal type; `header.word` is the type id. Field slots follow in
/// declaration order.
#[repr(C)]
pub struct TypedObject {
    /// Common header.
    pub header: ObjectHeader,
    /// Number of declared field slots.
    pub slot_count: u32,
    /// Address of an untyped Object holding keys outside the declaration, or 0.
    pub extra: u32,
}

/// An untyped Object; its keys live in a shape and its values in a Buffer.
#[repr(C)]
pub struct ObjectObject {
    /// Common header.
    pub header: ObjectHeader,
    /// Address of the [`ShapeObject`] describing the keys.
    pub shape: u32,
    /// Address of the Buffer holding one value per shape key.
    pub values: u32,
}

/// An ordered key list shared by untyped Objects; `header.word` is the key count.
///
/// Shapes form a tree from the empty root shape: every child extends its parent by one key.
/// The children of one shape are linked through `first_child` and `next_sibling`.
#[repr(C)]
pub struct ShapeObject {
    /// Common header.
    pub header: ObjectHeader,
    /// Address of the shape this one extends by its last key, or 0 for the empty shape.
    pub parent: u32,
    /// Address of the Buffer holding the key Strings in order, 0 for the empty shape.
    pub keys: u32,
    /// Address of the first shape extending this one, or 0.
    pub first_child: u32,
    /// Address of the next shape extending the same parent, or 0.
    pub next_sibling: u32,
}

/// A value of one enum variant; `header.word` is the enum type id. Payload slots follow.
#[repr(C)]
pub struct EnumObject {
    /// Common header.
    pub header: ObjectHeader,
    /// Index of the variant inside its enum declaration.
    pub variant: u32,
    /// Number of payload slots.
    pub field_count: u32,
}

/// Mutable storage of one captured binding.
#[repr(C)]
pub struct CellObject {
    /// Common header.
    pub header: ObjectHeader,
    /// The captured value.
    pub value: u64,
}

/// A lifted function with its captured Cells; `header.word` is the function id. Capture
/// slots follow.
#[repr(C)]
pub struct ClosureObject {
    /// Common header.
    pub header: ObjectHeader,
    /// Number of capture slots.
    pub capture_count: u32,
    /// Reserved, keeps the slots 8-byte aligned.
    pub reserved: u32,
}

/// A structured language Error; `header.word` is its severity.
#[repr(C)]
pub struct ErrorObject {
    /// Common header.
    pub header: ObjectHeader,
    /// Source position id of its origin, or 0.
    pub origin: u32,
    /// Address of a Bytes object holding the encoded trace entries, or 0.
    pub trace: u32,
    /// Error kind String.
    pub kind: u64,
    /// Message String.
    pub message: u64,
    /// Attached data value.
    pub data: u64,
}

/// Durable state of one resumable invocation; `header.word` is the function id. Slots follow.
#[repr(C)]
pub struct FrameObject {
    /// Common header.
    pub header: ObjectHeader,
    /// Continuation state to resume at.
    pub state: u32,
    /// Address of the caller Frame, or 0 for a task root.
    pub caller: u32,
    /// Caller slot that receives the result.
    pub destination: u32,
    /// Number of durable slots.
    pub slot_count: u32,
}

/// A free cell of a size-class page, linked into its class's free list.
#[repr(C)]
pub struct FreeCell {
    /// Common header with kind [`crate::ObjectKind::Free`].
    pub header: ObjectHeader,
    /// Address of the next free cell of the same class, or 0.
    pub next: u32,
    /// Reserved, keeps the minimum cell 16 bytes long.
    pub reserved: u32,
}

/// Asserts that a fixed object part ends 8-byte aligned.
macro_rules! assert_aligned {
    ($($object:ty),* $(,)?) => {
        $(const _: () = assert!(core::mem::size_of::<$object>().is_multiple_of(8));)*
    };
}

assert_aligned!(
    StringObject,
    BigIntObject,
    FloatObject,
    BufferObject,
    ListObject,
    TypedObject,
    ObjectObject,
    ShapeObject,
    EnumObject,
    CellObject,
    ClosureObject,
    ErrorObject,
    FrameObject,
    FreeCell,
);
