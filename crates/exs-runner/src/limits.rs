//! Shared execution-resource limits for native ExS runners.

use std::time::Duration;

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
use exs_abi::CborLimits;

/// Identifies the resource exhausted by one runner execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LimitKind {
    /// The WebAssembly linear memory grew beyond its configured byte limit.
    Memory,
    /// The submitted WebAssembly module exceeded its configured input-size limit.
    Module,
    /// The native WebAssembly engine consumed all configured fuel.
    Fuel,
    /// The root execution exceeded its wall-clock deadline.
    Timeout,
    /// The runtime attempted to create more language tasks than allowed.
    Tasks,
    /// The execution started more host calls than allowed.
    HostCalls,
    /// The execution held more unresolved host calls than allowed.
    PendingHostCalls,
    /// The execution retained more ready synchronous host responses than allowed.
    ReadyResponses,
    /// The execution retained more host-owned response bytes than allowed.
    HostOwnedBytes,
    /// The native WebAssembly call stack exceeded its configured byte limit.
    WasmStack,
    /// A main input or host CBOR request/response exceeded its byte limit.
    CborPayload,
    /// A final result CBOR payload exceeded its byte limit.
    Result,
    /// A CBOR value exceeded its recursive nesting limit.
    CborNesting,
    /// A CBOR collection exceeded its configured entry limit.
    CborCollectionEntries,
}

impl std::fmt::Display for LimitKind {
    /// Formats one stable resource category for diagnostics.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Memory => "memory",
            Self::Module => "module",
            Self::Fuel => "fuel",
            Self::Timeout => "timeout",
            Self::Tasks => "tasks",
            Self::HostCalls => "host calls",
            Self::PendingHostCalls => "pending host calls",
            Self::ReadyResponses => "ready host responses",
            Self::HostOwnedBytes => "host-owned bytes",
            Self::WasmStack => "WebAssembly stack",
            Self::CborPayload => "CBOR payload",
            Self::Result => "result",
            Self::CborNesting => "CBOR nesting",
            Self::CborCollectionEntries => "CBOR collection entries",
        };
        formatter.write_str(name)
    }
}

/// Configurable resource limits for one native root execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionLimits {
    /// Maximum linear-memory bytes available to the Wasm instance.
    pub max_memory_bytes: usize,
    /// Maximum submitted WebAssembly module bytes accepted before compilation.
    pub max_module_bytes: usize,
    /// Maximum deterministic Wasmtime fuel units available to the root execution.
    pub max_fuel: u64,
    /// Maximum wall-clock time allowed for the root execution.
    pub timeout: Duration,
    /// Maximum number of concurrently active language tasks, including the root task.
    pub max_tasks: usize,
    /// Maximum number of host calls started over the complete root execution.
    pub max_host_calls: usize,
    /// Maximum number of host calls that may await completion concurrently.
    pub max_pending_host_calls: usize,
    /// Maximum synchronous host responses retained before the runtime copies them.
    pub max_ready_responses: usize,
    /// Maximum total bytes retained by synchronous host responses before runtime copying.
    pub max_host_owned_bytes: usize,
    /// Maximum native Wasm call-stack bytes available to Wasmtime.
    pub max_wasm_stack_bytes: usize,
    /// Maximum bytes for a main input and every host request or response CBOR payload.
    pub max_cbor_payload_bytes: usize,
    /// Maximum bytes for the final result CBOR payload.
    pub max_result_bytes: usize,
    /// Maximum recursive CBOR value depth, including the root value.
    pub max_cbor_nesting: usize,
    /// Maximum direct entries in a decoded or encoded CBOR collection.
    pub max_cbor_collection_entries: usize,
}

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
impl ExecutionLimits {
    /// Returns the CBOR resource limits derived from this runner policy.
    #[must_use]
    pub(crate) const fn cbor_limits(&self) -> CborLimits {
        CborLimits {
            max_payload_bytes: self.max_cbor_payload_bytes,
            max_nesting: self.max_cbor_nesting,
            max_collection_entries: self.max_cbor_collection_entries,
        }
    }
}

impl Default for ExecutionLimits {
    /// Creates conservative limits suitable for execution of untrusted ExS source.
    fn default() -> Self {
        Self {
            max_memory_bytes: 16 * 1024 * 1024,
            max_module_bytes: 4 * 1024 * 1024,
            max_fuel: 10_000_000,
            timeout: Duration::from_secs(10),
            max_tasks: 1_024,
            max_host_calls: 10_000,
            max_pending_host_calls: 128,
            max_ready_responses: 128,
            max_host_owned_bytes: 4 * 1024 * 1024,
            max_wasm_stack_bytes: 1024 * 1024,
            max_cbor_payload_bytes: 2 * 1024 * 1024,
            max_result_bytes: 2 * 1024 * 1024,
            max_cbor_nesting: 64,
            max_cbor_collection_entries: 65_536,
        }
    }
}
