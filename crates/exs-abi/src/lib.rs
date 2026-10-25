#![no_std]

//! Stable names and versions shared by `ExS` compiler, runtime, and runners.

extern crate alloc;

use alloc::vec::Vec;

pub mod image;
#[cfg(feature = "serde")]
mod serde;
pub mod value;
#[cfg(feature = "serde")]
pub use serde::SerdeError;

/// An explicit byte-buffer field for Serde-backed ExS boundary data.
///
/// This wrapper serializes as [`ExsValue::Bytes`], while ordinary `Vec<T>` values retain their
/// generic `List<T>` representation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bytes(Vec<u8>);

impl Bytes {
    /// Creates an ExS byte-buffer value from its owned bytes.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Returns the byte-buffer contents.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// Consumes this wrapper and returns its owned byte buffer.
    #[must_use]
    pub fn into_inner(self) -> Vec<u8> {
        self.0
    }
}

impl From<Vec<u8>> for Bytes {
    /// Wraps an owned byte buffer for the ExS Serde boundary.
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

pub use value::{
    BoundaryError, BoundaryLimits, ErrorSeverity, ExsError, ExsStackFrame, ExsValue,
    SourcePositionId,
};

/// The ABI version shared by compiled modules, Rust guests and runners.
///
/// Version 27 exchanges values of nominal Object types as [`ExsValue::Typed`] with their type
/// identity.
pub const ABI_VERSION: u32 = 28;

/// Stable host-boundary identity used by the compiler-owned `IteratorStep` enum.
pub const STANDARD_ITERATOR_STEP_TYPE_IDENTITY: &str = "<std>/iterator.exs::IteratorStep";
/// Stable host-boundary identity of the standard `DateTime` type.
pub const STANDARD_DATETIME_TYPE_IDENTITY: &str = "<std>/datetime.exs::DateTime";
/// Stable host-boundary identity of the standard `Duration` type.
pub const STANDARD_DURATION_TYPE_IDENTITY: &str = "<std>/duration.exs::Duration";
/// Runner-internal host name used by the built-in `Host::sleep` operation.
pub const HOST_SLEEP_HOST_NAME: &str = "__exs.host.sleep";
/// Runner-internal host name used by the built-in `Host::now` operation.
pub const HOST_NOW_HOST_NAME: &str = "__exs.host.now";
/// Runner-internal host name used by the built-in `Host::elapsed` operation.
pub const HOST_ELAPSED_HOST_NAME: &str = "__exs.host.elapsed";
/// Runner-internal host name used to render an instant in one IANA time zone.
pub const HOST_DATETIME_IN_TIMEZONE_HOST_NAME: &str = "__exs.host.datetime_in_timezone";
/// Runner-internal host name used to resolve civil components in one IANA time zone.
pub const HOST_DATETIME_FROM_COMPONENTS_HOST_NAME: &str = "__exs.host.datetime_from_components";
/// Runner-internal host name used to open a host-owned pull stream.
pub const HOST_STREAM_OPEN_HOST_NAME: &str = "__exs.host.stream_open";
/// Runner-internal host name used to advance one host-owned pull stream.
pub const HOST_STREAM_NEXT_HOST_NAME: &str = "__exs.host.stream_next";
/// One Host operation implemented directly by every runner backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuiltinHostOperation {
    /// Suspends execution for one Duration.
    Sleep,
    /// Returns the current wall-clock DateTime.
    Now,
    /// Returns elapsed monotonic Duration for the root execution.
    Elapsed,
    /// Converts one DateTime to an IANA time zone.
    DateTimeInTimezone,
    /// Resolves civil DateTime components in an IANA time zone.
    DateTimeFromComponents,
    /// Opens one host-owned pull stream.
    StreamOpen,
    /// Advances one host-owned pull stream.
    StreamNext,
}

impl BuiltinHostOperation {
    /// Every operation reserved by the runner Host ABI.
    pub const ALL: &[Self] = &[
        Self::Sleep,
        Self::Now,
        Self::Elapsed,
        Self::DateTimeInTimezone,
        Self::DateTimeFromComponents,
        Self::StreamOpen,
        Self::StreamNext,
    ];

    /// Returns the stable Host ABI name for this operation.
    #[must_use]
    pub const fn host_name(self) -> &'static str {
        match self {
            Self::Sleep => HOST_SLEEP_HOST_NAME,
            Self::Now => HOST_NOW_HOST_NAME,
            Self::Elapsed => HOST_ELAPSED_HOST_NAME,
            Self::DateTimeInTimezone => HOST_DATETIME_IN_TIMEZONE_HOST_NAME,
            Self::DateTimeFromComponents => HOST_DATETIME_FROM_COMPONENTS_HOST_NAME,
            Self::StreamOpen => HOST_STREAM_OPEN_HOST_NAME,
            Self::StreamNext => HOST_STREAM_NEXT_HOST_NAME,
        }
    }
}

/// Resolves one stable Host ABI name to its runner-provided operation.
#[must_use]
pub fn builtin_host_operation(name: &str) -> Option<BuiltinHostOperation> {
    BuiltinHostOperation::ALL
        .iter()
        .copied()
        .find(|operation| operation.host_name() == name)
}

/// Returns whether `name` is reserved for one runner-provided `Host` operation.
#[must_use]
pub fn is_reserved_host_name(name: &str) -> bool {
    builtin_host_operation(name).is_some()
}
/// The custom section emitted by compiled modules.
pub const MODULE_METADATA_SECTION: &str = "exs.meta";
/// Prefix for compiler-generated entry exports invoked by runners.
pub const START_EXPORT_PREFIX: &str = "__exs_start_";
/// The ABI-version export.
pub const ABI_VERSION_EXPORT: &str = "__exs_abi_version";
/// The runtime export allocating a host-writable input buffer in linear memory.
pub const INPUT_ALLOC_EXPORT: &str = "__exs_input_alloc";
/// The export of a Rust guest returning where the image of its completed result lies: the
/// linear-memory address in the low 32 bits and the byte length in the high 32 bits.
pub const RESULT_IMAGE_EXPORT: &str = "__exs_result_image";
/// The export of a compiled module returning the completed result as a raw `ValueRef`,
/// which the runner reads from memory instead of a result image.
pub const RESULT_VALUE_EXPORT: &str = "__exs_result_value";
/// The export of a compiled module returning the address of its VM state block.
pub const VM_EXPORT: &str = "__exs_vm";
/// The module name containing runner-provided Host ABI imports.
pub const HOST_IMPORT_MODULE: &str = "exs";
/// The language-neutral module name containing runner resource-metering imports.
pub const RUNNER_IMPORT_MODULE: &str = "runner";
/// The generic runner import that starts one host call.
pub const HOST_CALL_START_IMPORT: &str = "__exs_host_call_start";
/// The runner import of compiled modules that starts one host call with the arguments as a
/// List value the runner reads from module memory.
pub const HOST_CALL_START_VALUE_IMPORT: &str = "__exs_host_call_start_value";
/// The runner import returning one ready host response byte length.
pub const HOST_CALL_RESPONSE_LENGTH_IMPORT: &str = "__exs_host_call_response_len";
/// The runner import copying one ready host response into Wasm linear memory.
pub const HOST_CALL_RESPONSE_COPY_IMPORT: &str = "__exs_host_call_response_copy";
/// The runner import that acquires one active language-task permit.
pub const RUNNER_TASK_ACQUIRE_IMPORT: &str = "__runner_task_acquire";
/// The runner import that releases one active language-task permit.
pub const RUNNER_TASK_RELEASE_IMPORT: &str = "__runner_task_release";
/// The compiler-generated export used by runners to resume a completed host call.
pub const RESUME_HOST_EXPORT: &str = "__exs_resume_host";
/// The compiler-generated export used by runners to cancel a suspended root execution.
pub const CANCEL_EXPORT: &str = "__exs_cancel";

/// A successfully completed execution.
pub const STATUS_COMPLETE: i32 = 2;
/// A root execution suspended while waiting for a host-call completion.
pub const STATUS_PENDING: i32 = 1;
/// A root execution was cancelled before its pending host call completed.
pub const STATUS_CANCELLED: i32 = 3;

/// A host call completed synchronously and has a response available to the runtime.
pub const HOST_CALL_READY: i32 = 0;
/// A host call is pending and will be resumed by the runner.
pub const HOST_CALL_PENDING: i32 = 1;
/// A host call failed at the runner-technical layer.
pub const HOST_CALL_FATAL: i32 = 2;
/// The arguments of a direct-boundary host call cannot cross the boundary; the module reports
/// a recoverable SerializationError instead of calling.
pub const HOST_CALL_UNSUPPORTED: i32 = 3;
