#![no_std]

//! Stable names and versions shared by `ExS` compiler, runtime, and runners.

extern crate alloc;

pub mod cbor;

pub use cbor::{CborError, ExsValue};

/// The compiler/runtime ABI version for the current Phase-1 implementation.
pub const ABI_VERSION: u32 = 2;
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

/// A successfully completed execution.
pub const STATUS_COMPLETE: i32 = 2;
/// A runnable execution that needs another runner turn.
pub const STATUS_READY: i32 = 0;
