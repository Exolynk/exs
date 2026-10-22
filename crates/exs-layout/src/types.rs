//! Type masks of source-visible values and the compiler-owned standard types.

/// Runtime type-mask bit for None.
pub const TYPE_NONE: u32 = 1 << 0;
/// Runtime type-mask bit for Error.
pub const TYPE_ERROR: u32 = 1 << 1;
/// Runtime type-mask bit for Bool.
pub const TYPE_BOOL: u32 = 1 << 2;
/// Runtime type-mask bit for Int.
pub const TYPE_INT: u32 = 1 << 3;
/// Runtime type-mask bit for Float.
pub const TYPE_FLOAT: u32 = 1 << 4;
/// Runtime type-mask bit for String.
pub const TYPE_STRING: u32 = 1 << 5;
/// Runtime type-mask bit for List.
pub const TYPE_LIST: u32 = 1 << 6;
/// Runtime type-mask bit for Object.
pub const TYPE_OBJECT: u32 = 1 << 7;
/// Runtime type-mask bit for callable closure values.
pub const TYPE_FN: u32 = 1 << 8;
/// Runtime type-mask bit for immutable byte sequences.
pub const TYPE_BYTES: u32 = 1 << 9;
/// Runtime type-mask accepting every current source-visible value type.
pub const TYPE_ANY: u32 = TYPE_NONE
    | TYPE_ERROR
    | TYPE_BOOL
    | TYPE_INT
    | TYPE_FLOAT
    | TYPE_STRING
    | TYPE_LIST
    | TYPE_OBJECT
    | TYPE_FN
    | TYPE_BYTES;
/// Reserved nominal type tag used by the compiler-owned `std::Ordering` enum.
pub const STANDARD_ORDERING_TYPE_ID: u32 = 0;
/// Stable host-boundary identity used by the compiler-owned `std::Ordering` enum.
pub const STANDARD_ORDERING_TYPE_IDENTITY: &str = "std::Ordering";
