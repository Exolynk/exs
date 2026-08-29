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

/// Selects the private host-resource budget used for one execution.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProtectionLevel {
    /// Allows the broadest resource headroom for trusted, ABI-compatible guests.
    Low,
    /// Balances compatibility and resource protection for general-purpose guest execution.
    #[default]
    Standard,
    /// Uses tighter host-resource budgets for untrusted multi-tenant execution.
    High,
}

/// Private runner resource limits derived from the public execution policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExecutionLimits {
    /// Maximum linear-memory bytes available to the Wasm instance.
    pub(crate) max_memory_bytes: usize,
    /// Maximum submitted WebAssembly module bytes accepted before compilation.
    pub(crate) max_module_bytes: usize,
    /// Maximum deterministic Wasmtime fuel units available to the root execution.
    pub(crate) max_fuel: u64,
    /// Maximum wall-clock time allowed for the root execution.
    pub(crate) timeout: Duration,
    /// Maximum number of elements retained by one Wasm function table.
    pub(crate) max_table_elements: usize,
    /// Maximum number of Wasm tables created by one store.
    pub(crate) max_tables: usize,
    /// Maximum number of Wasm linear memories created by one store.
    pub(crate) max_memories: usize,
    /// Maximum number of concurrently active language tasks, including the root task.
    pub(crate) max_tasks: usize,
    /// Maximum number of host calls started over the complete root execution.
    pub(crate) max_host_calls: usize,
    /// Maximum number of host calls that may await completion concurrently.
    pub(crate) max_pending_host_calls: usize,
    /// Maximum synchronous host responses retained before the runtime copies them.
    pub(crate) max_ready_responses: usize,
    /// Maximum total bytes retained by synchronous host responses before runtime copying.
    pub(crate) max_host_owned_bytes: usize,
    /// Maximum native Wasm call-stack bytes available to Wasmtime.
    pub(crate) max_wasm_stack_bytes: usize,
    /// Maximum bytes for a main input and every host request or response CBOR payload.
    pub(crate) max_cbor_payload_bytes: usize,
    /// Maximum bytes for the final result CBOR payload.
    pub(crate) max_result_bytes: usize,
    /// Maximum recursive CBOR value depth, including the root value.
    pub(crate) max_cbor_nesting: usize,
    /// Maximum direct entries in a decoded or encoded CBOR collection.
    pub(crate) max_cbor_collection_entries: usize,
}

impl ExecutionLimits {
    /// Derives runner-owned limits from the public execution policy.
    #[must_use]
    pub(crate) fn new(
        max_memory_bytes: usize,
        max_fuel: u64,
        timeout: Duration,
        protection: ProtectionLevel,
    ) -> Self {
        let (
            max_module_bytes,
            max_table_elements,
            max_tasks,
            max_host_calls,
            max_pending_host_calls,
            max_ready_responses,
            max_host_owned_bytes,
            max_wasm_stack_bytes,
            max_cbor_payload_bytes,
            max_result_bytes,
            max_cbor_nesting,
            max_cbor_collection_entries,
        ) = match protection {
            ProtectionLevel::Low => (
                4 * 1024 * 1024,
                65_536,
                4_096,
                100_000,
                512,
                512,
                max_memory_bytes / 2,
                2 * 1024 * 1024,
                max_memory_bytes / 4,
                max_memory_bytes / 4,
                128,
                65_536,
            ),
            ProtectionLevel::Standard => (
                4 * 1024 * 1024,
                16_384,
                1_024,
                10_000,
                128,
                128,
                max_memory_bytes / 4,
                1024 * 1024,
                max_memory_bytes / 8,
                max_memory_bytes / 8,
                64,
                65_536,
            ),
            ProtectionLevel::High => (
                4 * 1024 * 1024,
                4_096,
                128,
                1_000,
                32,
                32,
                max_memory_bytes / 8,
                512 * 1024,
                max_memory_bytes / 16,
                max_memory_bytes / 16,
                32,
                16_384,
            ),
        };
        Self {
            max_memory_bytes,
            max_module_bytes,
            max_fuel,
            timeout,
            max_table_elements,
            max_tables: 1,
            max_memories: 1,
            max_tasks,
            max_host_calls,
            max_pending_host_calls,
            max_ready_responses,
            max_host_owned_bytes,
            max_wasm_stack_bytes,
            max_cbor_payload_bytes,
            max_result_bytes,
            max_cbor_nesting,
            max_cbor_collection_entries,
        }
    }
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
        Self::new(
            16 * 1024 * 1024,
            10_000_000,
            Duration::from_secs(10),
            ProtectionLevel::Standard,
        )
    }
}
