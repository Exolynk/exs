//! Values that cross the boundary between a module and its runner.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Default maximum payload size of one value.
const DEFAULT_MAX_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;
/// Default maximum nesting depth of one value.
const DEFAULT_MAX_NESTING: usize = 64;
/// Default maximum direct entries in one collection of a value.
const DEFAULT_MAX_COLLECTION_ENTRIES: usize = 65_536;

/// A host-safe ExS value represented independently of the runtime heap.
///
/// This representation contains only values that can safely cross the Wasm-host boundary. Runtime
/// references, object identities, and cycles are never exposed through this type.
#[derive(Clone, Debug, PartialEq)]
pub enum ExsValue {
    /// The absence of a value, such as a missing Object key.
    None,
    /// The result of a function that completes without a value.
    Unit,
    /// A structured ExS language error.
    Error(ExsError),
    /// An ExS boolean.
    Bool(bool),
    /// An ExS signed integer.
    Int(i64),
    /// An ExS IEEE 754 binary64 floating-point number.
    Float(f64),
    /// An immutable UTF-8 ExS string.
    String(String),
    /// An immutable sequence of raw octets.
    Bytes(Vec<u8>),
    /// An ordered sequence of host-safe ExS values.
    List(Vec<ExsValue>),
    /// An insertion-ordered mapping from string keys to host-safe ExS values.
    Object(Vec<(String, ExsValue)>),
    /// A value of a nominal Object type with its opaque type identity and named fields.
    Typed {
        /// Resolver-derived nominal type identity, such as `exo::dom::Div`.
        type_id: String,
        /// Field names and values in entry order.
        fields: Vec<(String, ExsValue)>,
    },
    /// A nominal enum value with an opaque type identity and ordered variant fields.
    Enum {
        /// Resolver-derived nominal enum identity.
        type_id: String,
        /// Source-visible enum variant name.
        variant: String,
        /// Variant payload fields in declaration order.
        fields: Vec<ExsValue>,
    },
}

/// The severity of one ExS language Error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorSeverity {
    /// A language error from which execution may safely continue.
    Recoverable,
    /// A fault that terminates the current execution context.
    Fatal,
}

/// One source position assigned by the compiler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourcePositionId(pub u32);

/// One language-level call frame stored in an Error trace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExsStackFrame {
    /// Compiler-assigned source function identifier.
    pub function_id: u32,
    /// Compiler-assigned source position for the call site.
    pub call_site: SourcePositionId,
}

/// A host-safe ExS language Error.
#[derive(Clone, Debug, PartialEq)]
pub struct ExsError {
    /// Whether execution may continue after this Error.
    pub severity: ErrorSeverity,
    /// Stable machine-readable error kind.
    pub kind: String,
    /// Human-readable error message.
    pub message: String,
    /// Additional language data associated with the Error.
    pub data: Box<ExsValue>,
    /// Compiler-assigned source position that created the Error.
    pub origin: Option<SourcePositionId>,
    /// Language-level call frames from innermost to outermost.
    pub trace: Vec<ExsStackFrame>,
    /// An optional prior Error or related language value.
    pub cause: Option<Box<ExsValue>>,
}

/// Why a value could not cross the runner boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryError {
    /// The input does not hold a well-formed value.
    Malformed,
    /// The value is too large to be represented at all.
    TooLarge,
    /// The value exceeded the configured nesting limit.
    NestingLimitExceeded,
    /// The value exceeded the configured collection-entry limit.
    CollectionLimitExceeded,
    /// The value exceeded the configured payload limit.
    PayloadLimitExceeded,
}

impl fmt::Display for BoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Malformed => "malformed boundary value",
            Self::TooLarge => "boundary value is too large",
            Self::NestingLimitExceeded => "nesting limit exceeded",
            Self::CollectionLimitExceeded => "collection-entry limit exceeded",
            Self::PayloadLimitExceeded => "payload limit exceeded",
        })
    }
}

/// Limits applied to one value crossing the runner boundary.
///
/// The payload size of a value is the size of its compact encoding: one to nine bytes per
/// scalar and per length, plus the bytes of Strings and Bytes, independent of how the value is
/// represented while it crosses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoundaryLimits {
    /// Maximum payload size of one value.
    pub max_payload_bytes: usize,
    /// Maximum recursive value depth, including the root value.
    pub max_nesting: usize,
    /// Maximum direct entries in any List, Object, enum payload, or Error trace.
    pub max_collection_entries: usize,
}

impl BoundaryLimits {
    /// Returns limits that explicitly disable every bound.
    #[must_use]
    pub const fn unrestricted() -> Self {
        Self {
            max_payload_bytes: usize::MAX,
            max_nesting: usize::MAX,
            max_collection_entries: usize::MAX,
        }
    }
}

impl Default for BoundaryLimits {
    /// Creates conservative limits suitable for one untrusted host-boundary value.
    fn default() -> Self {
        Self {
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
            max_nesting: DEFAULT_MAX_NESTING,
            max_collection_entries: DEFAULT_MAX_COLLECTION_ENTRIES,
        }
    }
}
