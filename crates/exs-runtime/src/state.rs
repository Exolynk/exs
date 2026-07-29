//! Runtime-owned mutable state for one Wasm instance.

use alloc::vec::Vec;
use core::cell::UnsafeCell;

use dlmalloc::GlobalDlmalloc;
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

/// Mutable state isolated to one instantiated Phase-1 runtime.
pub(crate) struct RuntimeState {
    /// Runtime values addressed by one-based `ValueRef` indices.
    pub(crate) values: Vec<Option<HeapSlot>>,
    /// Reusable zero-based indices of swept value-table entries.
    pub(crate) free_slots: Vec<u32>,
    /// Active compiler-generated function root frames in call order.
    pub(crate) root_frames: Vec<RootFrame>,
    /// Native runtime values temporarily protected across further allocations.
    pub(crate) temporary_roots: Vec<ValueRef>,
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
            temporary_roots: Vec::new(),
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
