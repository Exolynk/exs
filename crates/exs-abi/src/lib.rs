#![no_std]

//! Stable names and versions shared by `ExS` compiler, runtime, and runners.

/// The compiler/runtime ABI version for the initial implementation.
pub const ABI_VERSION: u32 = 1;
/// The custom section emitted by compiled modules.
pub const MODULE_METADATA_SECTION: &str = "exs.meta";
/// The entry export invoked by runners.
pub const START_EXPORT: &str = "__exs_start";
/// The ABI-version export.
pub const ABI_VERSION_EXPORT: &str = "__exs_abi_version";
/// The result-kind export.
pub const RESULT_KIND_EXPORT: &str = "__exs_result_kind";
/// The result-value export.
pub const RESULT_VALUE_EXPORT: &str = "__exs_result_value";

/// A successfully completed execution.
pub const STATUS_COMPLETE: i32 = 2;
/// A runnable execution that needs another runner turn.
pub const STATUS_READY: i32 = 0;
