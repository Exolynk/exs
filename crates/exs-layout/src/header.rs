//! The header that starts every heap object.

/// Byte size of [`ObjectHeader`].
pub const HEADER_SIZE: u32 = 8;

/// Alignment of every heap object and of every object field holding a `ValueRef`.
pub const OBJECT_ALIGN: u32 = 8;

/// GC bit set while a collection has found the object reachable.
pub const GC_MARK: u8 = 1;

/// GC bit of objects outside the collected heap, such as literal Strings in data segments.
pub const GC_STATIC: u8 = 2;

/// Kind of one heap object, stored in the first header byte.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectKind {
    /// A free cell inside a size-class page; never reachable from values.
    Free = 0,
    /// Immutable UTF-8 text.
    String = 1,
    /// Immutable raw octets.
    Bytes = 2,
    /// An Int outside the immediate `i32` range.
    BigInt = 3,
    /// An IEEE 754 binary64 number.
    Float = 4,
    /// A growable array of `ValueRef` slots backing Lists and Object values.
    Buffer = 5,
    /// A mutable ordered sequence.
    List = 6,
    /// An Object of a declared nominal type with fixed field slots.
    TypedObject = 7,
    /// An untyped Object whose keys are described by a shape.
    Object = 8,
    /// The ordered key list shared by untyped Objects with the same keys.
    Shape = 9,
    /// A value of a declared enum variant.
    Enum = 10,
    /// Mutable storage of one captured binding.
    Cell = 11,
    /// A lifted function with its captured Cells.
    Closure = 12,
    /// A structured language Error.
    Error = 13,
    /// The durable state of one resumable function invocation.
    Frame = 14,
    /// One task of a resumable execution: a root task or a `par` child.
    Task = 15,
}

impl ObjectKind {
    /// Returns the kind stored in one header byte, or `None` for an unknown byte.
    #[must_use]
    pub const fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            0 => Self::Free,
            1 => Self::String,
            2 => Self::Bytes,
            3 => Self::BigInt,
            4 => Self::Float,
            5 => Self::Buffer,
            6 => Self::List,
            7 => Self::TypedObject,
            8 => Self::Object,
            9 => Self::Shape,
            10 => Self::Enum,
            11 => Self::Cell,
            12 => Self::Closure,
            13 => Self::Error,
            14 => Self::Frame,
            15 => Self::Task,
            _ => return None,
        })
    }
}

/// The first eight bytes of every heap object.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectHeader {
    /// One [`ObjectKind`] byte.
    pub kind: u8,
    /// GC bits such as [`GC_MARK`] and [`GC_STATIC`].
    pub gc: u8,
    /// Size-class index of the cell holding the object, or `u16::MAX` for large objects.
    pub class: u16,
    /// Kind-specific word: nominal type id, slot count, function id or severity.
    pub word: u32,
}

/// Byte offset of [`ObjectHeader::kind`].
pub const HEADER_KIND_OFFSET: u32 = core::mem::offset_of!(ObjectHeader, kind) as u32;
/// Byte offset of [`ObjectHeader::gc`].
pub const HEADER_GC_OFFSET: u32 = core::mem::offset_of!(ObjectHeader, gc) as u32;
/// Byte offset of [`ObjectHeader::class`].
pub const HEADER_CLASS_OFFSET: u32 = core::mem::offset_of!(ObjectHeader, class) as u32;
/// Byte offset of [`ObjectHeader::word`].
pub const HEADER_WORD_OFFSET: u32 = core::mem::offset_of!(ObjectHeader, word) as u32;

const _: () = assert!(core::mem::size_of::<ObjectHeader>() == HEADER_SIZE as usize);
const _: () = assert!(HEADER_KIND_OFFSET == 0 && HEADER_GC_OFFSET == 1);
const _: () = assert!(HEADER_CLASS_OFFSET == 2 && HEADER_WORD_OFFSET == 4);
