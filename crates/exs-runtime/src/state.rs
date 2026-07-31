//! Runtime-owned mutable state for one Wasm instance.

use alloc::vec::Vec;
use core::cell::UnsafeCell;

use dlmalloc::GlobalDlmalloc;
use exs_abi::{ExsStackFrame, SourcePositionId};
use exs_value::ValueRef;

use crate::value::RtValue;

#[global_allocator]
static ALLOCATOR: GlobalDlmalloc = GlobalDlmalloc;

/// One runtime value-table entry and its collector mark state.
pub(crate) struct HeapSlot {
    /// Whether the current collection has reached this value.
    pub(crate) marked: bool,
    /// The runtime-owned language payload.
    pub(crate) value: RtValue,
}

impl HeapSlot {
    /// Creates an unmarked value-table entry.
    pub(crate) const fn new(value: RtValue) -> Self {
        Self {
            marked: false,
            value,
        }
    }
}

/// Compiler-generated roots for one active ExS function invocation.
pub(crate) struct RootFrame {
    /// One optional ValueRef for every compiler-declared value local and parameter.
    pub(crate) slots: Vec<Option<ValueRef>>,
}

/// The caller location that receives one completed resumable-function result.
#[derive(Clone, Copy)]
pub(crate) struct FrameContinuation {
    /// One-based runtime async-frame identifier.
    pub(crate) frame: u32,
    /// Destination slot in the caller frame.
    pub(crate) slot: u32,
}

/// Persistent state for one compiler-generated resumable function invocation.
pub(crate) struct AsyncFrame {
    /// Compiler-assigned generated function identifier.
    pub(crate) function_id: u32,
    /// Compiler-assigned continuation-graph state identifier.
    pub(crate) state: u32,
    /// Values that must survive a pending host call.
    pub(crate) slots: Vec<Option<ValueRef>>,
    /// Caller continuation, absent for the root resumable invocation.
    pub(crate) caller: Option<FrameContinuation>,
}

/// Mutable state isolated to one instantiated Phase-1 runtime.
pub(crate) struct RuntimeState {
    /// Runtime values addressed by one-based `ValueRef` indices.
    pub(crate) values: Vec<Option<HeapSlot>>,
    /// Reusable zero-based indices of swept value-table entries.
    pub(crate) free_slots: Vec<u32>,
    /// Active compiler-generated function root frames in call order.
    pub(crate) root_frames: Vec<RootFrame>,
    /// Durable frames for the currently active resumable root execution.
    pub(crate) async_frames: Vec<Option<AsyncFrame>>,
    /// Reusable zero-based async-frame indexes.
    pub(crate) free_async_frames: Vec<u32>,
    /// One-based identifier of the frame executed by the generated dispatcher.
    pub(crate) active_async_frame: Option<u32>,
    /// Root result retained after the final resumable frame completes.
    pub(crate) completed_async_result: Option<ValueRef>,
    /// CBOR request bytes for the active generic host call.
    pub(crate) host_request_buffer: Vec<u8>,
    /// CBOR response bytes copied from the runner for a synchronous host result.
    pub(crate) host_response_buffer: Vec<u8>,
    /// The next monotonic HostCallId assigned within this Wasm instance.
    pub(crate) next_host_call_id: u64,
    /// The HostCallId whose runner response is currently available.
    pub(crate) active_host_call: Option<u64>,
    /// A locally generated ready host Error, such as an invalid dynamic call name.
    pub(crate) ready_host_result: Option<ValueRef>,
    /// Native runtime values temporarily protected across further allocations.
    pub(crate) temporary_roots: Vec<ValueRef>,
    /// Source position applied to the next recoverable runtime Error.
    pub(crate) current_source_position: Option<SourcePositionId>,
    /// Active direct language calls from root to innermost frame.
    pub(crate) frames: Vec<ExsStackFrame>,
    /// Call-site position consumed by the next generated function entry.
    pub(crate) pending_call_site: Option<SourcePositionId>,
    /// CBOR bytes supplied by the runner for the next root execution.
    pub(crate) input_buffer: Vec<u8>,
    /// Bytes copied from one compiler-owned passive literal data segment.
    pub(crate) literal_buffer: Vec<u8>,
    /// CBOR bytes holding the completed root result.
    pub(crate) result_buffer: Vec<u8>,
}

impl RuntimeState {
    /// Creates empty value and CBOR buffer stores.
    const fn new() -> Self {
        Self {
            values: Vec::new(),
            free_slots: Vec::new(),
            root_frames: Vec::new(),
            async_frames: Vec::new(),
            free_async_frames: Vec::new(),
            active_async_frame: None,
            completed_async_result: None,
            host_request_buffer: Vec::new(),
            host_response_buffer: Vec::new(),
            next_host_call_id: 1,
            active_host_call: None,
            ready_host_result: None,
            temporary_roots: Vec::new(),
            current_source_position: None,
            frames: Vec::new(),
            pending_call_site: None,
            input_buffer: Vec::new(),
            literal_buffer: Vec::new(),
            result_buffer: Vec::new(),
        }
    }
}

/// Mutable single-threaded state for one Wasm module instance.
struct RuntimeCell(UnsafeCell<RuntimeState>);

unsafe impl Sync for RuntimeCell {}

static RUNTIME: RuntimeCell = RuntimeCell(UnsafeCell::new(RuntimeState::new()));

/// Returns the state for the current single-threaded Wasm instance.
///
/// # Safety
///
/// Callers must not retain references across reentrant execution. Phase 1 has no reentrancy and
/// every mutable operation runs to completion before another runtime entry point begins.
pub(crate) unsafe fn runtime() -> &'static mut RuntimeState {
    unsafe { &mut *RUNTIME.0.get() }
}
