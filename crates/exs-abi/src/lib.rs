#![no_std]

//! Stable names and versions shared by `ExS` compiler, runtime, and runners.

extern crate alloc;

pub mod cbor;

pub use cbor::{
    CborError, CborLimits, ErrorSeverity, ExsError, ExsStackFrame, ExsValue, SourcePositionId,
};

/// The compiler/runtime ABI version for the current Phase-1 implementation.
///
/// Version 20 adds generic runner task-acquire and task-release imports.
pub const ABI_VERSION: u32 = 20;

/// Receiver method names implemented by the built-in runtime.
pub const RESERVED_METHOD_NAMES: &[&str] = &[
    "abs", "floor", "ceil", "round", "clone", "length", "is_empty", "kind", "message", "data",
    "cause", "push", "pop", "insert", "remove", "clear", "has", "delete", "keys", "values",
];

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
/// Runtime type-mask accepting every current source-visible value type.
pub const TYPE_ANY: u32 = TYPE_NONE
    | TYPE_ERROR
    | TYPE_BOOL
    | TYPE_INT
    | TYPE_FLOAT
    | TYPE_STRING
    | TYPE_LIST
    | TYPE_OBJECT
    | TYPE_FN;
/// Reserved nominal type tag used by the compiler-owned `std::Ordering` enum.
pub const STANDARD_ORDERING_TYPE_ID: u32 = 0;
/// Stable host-boundary identity used by the compiler-owned `std::Ordering` enum.
pub const STANDARD_ORDERING_TYPE_IDENTITY: &str = "std::Ordering";
/// The custom section emitted by compiled modules.
pub const MODULE_METADATA_SECTION: &str = "exs.meta";
/// The entry export invoked by runners.
pub const START_EXPORT: &str = "__exs_start";
/// The ABI-version export.
pub const ABI_VERSION_EXPORT: &str = "__exs_abi_version";
/// The runtime export allocating a host-writable input buffer in linear memory.
pub const INPUT_ALLOC_EXPORT: &str = "__exs_input_alloc";
/// The export returning the linear-memory pointer to the completed CBOR result.
pub const RESULT_POINTER_EXPORT: &str = "__exs_result_ptr";
/// The export returning the byte length of the completed CBOR result.
pub const RESULT_LENGTH_EXPORT: &str = "__exs_result_len";
/// The module name containing runner-provided Host ABI imports.
pub const HOST_IMPORT_MODULE: &str = "exs";
/// The language-neutral module name containing runner resource-metering imports.
pub const RUNNER_IMPORT_MODULE: &str = "runner";
/// The generic runner import that starts one host call.
pub const HOST_CALL_START_IMPORT: &str = "__exs_host_call_start";
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
/// A runnable execution that needs another runner turn.
pub const STATUS_READY: i32 = 0;
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
