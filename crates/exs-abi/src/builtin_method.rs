//! Built-in receiver methods that the runtime dispatches without a nominal implementation.
//!
//! The compiler calls a method in this table directly with its fixed number of arguments,
//! and the runtime's generic name-based dispatch resolves names through the same table, so
//! both paths reach identical behavior.

/// One built-in receiver method, identified across the compiler-runtime boundary by its index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum BuiltinMethod {
    /// The `add` method taking 1 argument.
    Add,
    /// The `sub` method taking 1 argument.
    Sub,
    /// The `mul` method taking 1 argument.
    Mul,
    /// The `div` method taking 1 argument.
    Div,
    /// The `div_euclid` method taking 1 argument.
    DivEuclid,
    /// The `rem_euclid` method taking 1 argument.
    RemEuclid,
    /// The `compare` method taking 1 argument.
    Compare,
    /// The `to_string` method taking 0 arguments.
    ToString,
    /// The `debug` method taking 0 arguments.
    Debug,
    /// The `abs` method taking 0 arguments.
    Abs,
    /// The `floor` method taking 0 arguments.
    Floor,
    /// The `ceil` method taking 0 arguments.
    Ceil,
    /// The `round` method taking 0 arguments.
    Round,
    /// The `trunc` method taking 0 arguments.
    Trunc,
    /// The `fract` method taking 0 arguments.
    Fract,
    /// The `signum` method taking 0 arguments.
    Signum,
    /// The `is_even` method taking 0 arguments.
    IsEven,
    /// The `is_odd` method taking 0 arguments.
    IsOdd,
    /// The `gcd` method taking 1 argument.
    Gcd,
    /// The `lcm` method taking 1 argument.
    Lcm,
    /// The `pow` method taking 1 argument.
    Pow,
    /// The `min` method taking 1 argument.
    Min,
    /// The `max` method taking 1 argument.
    Max,
    /// The `clamp` method taking 2 arguments.
    Clamp,
    /// The `to_float` method taking 0 arguments.
    ToFloat,
    /// The `to_int` method taking 0 arguments.
    ToInt,
    /// The `is_nan` method taking 0 arguments.
    IsNan,
    /// The `is_finite` method taking 0 arguments.
    IsFinite,
    /// The `is_infinite` method taking 0 arguments.
    IsInfinite,
    /// The `sqrt` method taking 0 arguments.
    Sqrt,
    /// The `clone` method taking 0 arguments.
    Clone,
    /// The `length` method taking 0 arguments.
    Length,
    /// The `is_empty` method taking 0 arguments.
    IsEmpty,
    /// The `to_list` method taking 0 arguments.
    ToList,
    /// The `slice` method taking 2 arguments.
    Slice,
    /// The `concat` method taking 1 argument.
    Concat,
    /// The `decode_utf8` method taking 0 arguments.
    DecodeUtf8,
    /// The `encode_utf8` method taking 0 arguments.
    EncodeUtf8,
    /// The `contains` method taking 1 argument.
    Contains,
    /// The `starts_with` method taking 1 argument.
    StartsWith,
    /// The `ends_with` method taking 1 argument.
    EndsWith,
    /// The `repeat` method taking 1 argument.
    Repeat,
    /// The `to_hex` method taking 0 arguments.
    ToHex,
    /// The `to_base64` method taking 0 arguments.
    ToBase64,
    /// The `trim` method taking 0 arguments.
    Trim,
    /// The `trim_start` method taking 0 arguments.
    TrimStart,
    /// The `trim_end` method taking 0 arguments.
    TrimEnd,
    /// The `replace` method taking 2 arguments.
    Replace,
    /// The `split` method taking 1 argument.
    Split,
    /// The `to_lowercase` method taking 0 arguments.
    ToLowercase,
    /// The `to_uppercase` method taking 0 arguments.
    ToUppercase,
    /// The `kind` method taking 0 arguments.
    Kind,
    /// The `message` method taking 0 arguments.
    Message,
    /// The `data` method taking 0 arguments.
    Data,
    /// The `cause` method taking 0 arguments.
    Cause,
    /// The `next` method taking 0 arguments.
    Next,
    /// The `push` method taking 1 argument.
    Push,
    /// The `get` method taking 1 argument.
    Get,
    /// The `first` method taking 0 arguments.
    First,
    /// The `last` method taking 0 arguments.
    Last,
    /// The `extend` method taking 1 argument.
    Extend,
    /// The `reverse` method taking 0 arguments.
    Reverse,
    /// The `reversed` method taking 0 arguments.
    Reversed,
    /// The `index_of` method taking 1 argument.
    IndexOf,
    /// The `last_index_of` method taking 1 argument.
    LastIndexOf,
    /// The `join` method taking 1 argument.
    Join,
    /// The `pop` method taking 0 arguments.
    Pop,
    /// The `insert` method taking 2 arguments.
    Insert,
    /// The `remove` method taking 1 argument.
    Remove,
    /// The `clear` method taking 0 arguments.
    Clear,
    /// The `has` method taking 1 argument.
    Has,
    /// The `delete` method taking 1 argument.
    Delete,
    /// The `keys` method taking 0 arguments.
    Keys,
    /// The `values` method taking 0 arguments.
    Values,
    /// The `get_or` method taking 2 arguments.
    GetOr,
    /// The `entries` method taking 0 arguments.
    Entries,
    /// The `merge` method taking 1 argument.
    Merge,
}

/// Every built-in method in index order.
const ALL: [BuiltinMethod; 77] = [
    BuiltinMethod::Add,
    BuiltinMethod::Sub,
    BuiltinMethod::Mul,
    BuiltinMethod::Div,
    BuiltinMethod::DivEuclid,
    BuiltinMethod::RemEuclid,
    BuiltinMethod::Compare,
    BuiltinMethod::ToString,
    BuiltinMethod::Debug,
    BuiltinMethod::Abs,
    BuiltinMethod::Floor,
    BuiltinMethod::Ceil,
    BuiltinMethod::Round,
    BuiltinMethod::Trunc,
    BuiltinMethod::Fract,
    BuiltinMethod::Signum,
    BuiltinMethod::IsEven,
    BuiltinMethod::IsOdd,
    BuiltinMethod::Gcd,
    BuiltinMethod::Lcm,
    BuiltinMethod::Pow,
    BuiltinMethod::Min,
    BuiltinMethod::Max,
    BuiltinMethod::Clamp,
    BuiltinMethod::ToFloat,
    BuiltinMethod::ToInt,
    BuiltinMethod::IsNan,
    BuiltinMethod::IsFinite,
    BuiltinMethod::IsInfinite,
    BuiltinMethod::Sqrt,
    BuiltinMethod::Clone,
    BuiltinMethod::Length,
    BuiltinMethod::IsEmpty,
    BuiltinMethod::ToList,
    BuiltinMethod::Slice,
    BuiltinMethod::Concat,
    BuiltinMethod::DecodeUtf8,
    BuiltinMethod::EncodeUtf8,
    BuiltinMethod::Contains,
    BuiltinMethod::StartsWith,
    BuiltinMethod::EndsWith,
    BuiltinMethod::Repeat,
    BuiltinMethod::ToHex,
    BuiltinMethod::ToBase64,
    BuiltinMethod::Trim,
    BuiltinMethod::TrimStart,
    BuiltinMethod::TrimEnd,
    BuiltinMethod::Replace,
    BuiltinMethod::Split,
    BuiltinMethod::ToLowercase,
    BuiltinMethod::ToUppercase,
    BuiltinMethod::Kind,
    BuiltinMethod::Message,
    BuiltinMethod::Data,
    BuiltinMethod::Cause,
    BuiltinMethod::Next,
    BuiltinMethod::Push,
    BuiltinMethod::Get,
    BuiltinMethod::First,
    BuiltinMethod::Last,
    BuiltinMethod::Extend,
    BuiltinMethod::Reverse,
    BuiltinMethod::Reversed,
    BuiltinMethod::IndexOf,
    BuiltinMethod::LastIndexOf,
    BuiltinMethod::Join,
    BuiltinMethod::Pop,
    BuiltinMethod::Insert,
    BuiltinMethod::Remove,
    BuiltinMethod::Clear,
    BuiltinMethod::Has,
    BuiltinMethod::Delete,
    BuiltinMethod::Keys,
    BuiltinMethod::Values,
    BuiltinMethod::GetOr,
    BuiltinMethod::Entries,
    BuiltinMethod::Merge,
];

impl BuiltinMethod {
    /// Resolves one source method name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        ALL.iter().copied().find(|method| method.name() == name)
    }

    /// Resolves one index produced by [`BuiltinMethod::id`].
    #[must_use]
    pub fn from_id(id: u32) -> Option<Self> {
        ALL.get(usize::try_from(id).ok()?).copied()
    }

    /// Returns the stable index passed across the compiler-runtime boundary.
    #[must_use]
    pub const fn id(self) -> u32 {
        self as u32
    }

    /// Returns the source method name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Sub => "sub",
            Self::Mul => "mul",
            Self::Div => "div",
            Self::DivEuclid => "div_euclid",
            Self::RemEuclid => "rem_euclid",
            Self::Compare => "compare",
            Self::ToString => "to_string",
            Self::Debug => "debug",
            Self::Abs => "abs",
            Self::Floor => "floor",
            Self::Ceil => "ceil",
            Self::Round => "round",
            Self::Trunc => "trunc",
            Self::Fract => "fract",
            Self::Signum => "signum",
            Self::IsEven => "is_even",
            Self::IsOdd => "is_odd",
            Self::Gcd => "gcd",
            Self::Lcm => "lcm",
            Self::Pow => "pow",
            Self::Min => "min",
            Self::Max => "max",
            Self::Clamp => "clamp",
            Self::ToFloat => "to_float",
            Self::ToInt => "to_int",
            Self::IsNan => "is_nan",
            Self::IsFinite => "is_finite",
            Self::IsInfinite => "is_infinite",
            Self::Sqrt => "sqrt",
            Self::Clone => "clone",
            Self::Length => "length",
            Self::IsEmpty => "is_empty",
            Self::ToList => "to_list",
            Self::Slice => "slice",
            Self::Concat => "concat",
            Self::DecodeUtf8 => "decode_utf8",
            Self::EncodeUtf8 => "encode_utf8",
            Self::Contains => "contains",
            Self::StartsWith => "starts_with",
            Self::EndsWith => "ends_with",
            Self::Repeat => "repeat",
            Self::ToHex => "to_hex",
            Self::ToBase64 => "to_base64",
            Self::Trim => "trim",
            Self::TrimStart => "trim_start",
            Self::TrimEnd => "trim_end",
            Self::Replace => "replace",
            Self::Split => "split",
            Self::ToLowercase => "to_lowercase",
            Self::ToUppercase => "to_uppercase",
            Self::Kind => "kind",
            Self::Message => "message",
            Self::Data => "data",
            Self::Cause => "cause",
            Self::Next => "next",
            Self::Push => "push",
            Self::Get => "get",
            Self::First => "first",
            Self::Last => "last",
            Self::Extend => "extend",
            Self::Reverse => "reverse",
            Self::Reversed => "reversed",
            Self::IndexOf => "index_of",
            Self::LastIndexOf => "last_index_of",
            Self::Join => "join",
            Self::Pop => "pop",
            Self::Insert => "insert",
            Self::Remove => "remove",
            Self::Clear => "clear",
            Self::Has => "has",
            Self::Delete => "delete",
            Self::Keys => "keys",
            Self::Values => "values",
            Self::GetOr => "get_or",
            Self::Entries => "entries",
            Self::Merge => "merge",
        }
    }

    /// Returns the exact number of arguments after the receiver.
    #[must_use]
    pub const fn arity(self) -> usize {
        match self {
            Self::ToString
            | Self::Debug
            | Self::Abs
            | Self::Floor
            | Self::Ceil
            | Self::Round
            | Self::Trunc
            | Self::Fract
            | Self::Signum
            | Self::IsEven
            | Self::IsOdd
            | Self::ToFloat
            | Self::ToInt
            | Self::IsNan
            | Self::IsFinite
            | Self::IsInfinite
            | Self::Sqrt
            | Self::Clone
            | Self::Length
            | Self::IsEmpty
            | Self::ToList
            | Self::DecodeUtf8
            | Self::EncodeUtf8
            | Self::ToHex
            | Self::ToBase64
            | Self::Trim
            | Self::TrimStart
            | Self::TrimEnd
            | Self::ToLowercase
            | Self::ToUppercase
            | Self::Kind
            | Self::Message
            | Self::Data
            | Self::Cause
            | Self::Next
            | Self::First
            | Self::Last
            | Self::Reverse
            | Self::Reversed
            | Self::Pop
            | Self::Clear
            | Self::Keys
            | Self::Values
            | Self::Entries => 0,
            Self::Add
            | Self::Sub
            | Self::Mul
            | Self::Div
            | Self::DivEuclid
            | Self::RemEuclid
            | Self::Compare
            | Self::Gcd
            | Self::Lcm
            | Self::Pow
            | Self::Min
            | Self::Max
            | Self::Concat
            | Self::Contains
            | Self::StartsWith
            | Self::EndsWith
            | Self::Repeat
            | Self::Split
            | Self::Push
            | Self::Get
            | Self::Extend
            | Self::IndexOf
            | Self::LastIndexOf
            | Self::Join
            | Self::Remove
            | Self::Has
            | Self::Delete
            | Self::Merge => 1,
            Self::Clamp | Self::Slice | Self::Replace | Self::Insert | Self::GetOr => 2,
        }
    }
}
