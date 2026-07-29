//! Runtime-owned mutable state for one Wasm instance.

use alloc::vec::Vec;
use core::cell::UnsafeCell;

use dlmalloc::GlobalDlmalloc;

use crate::value::RtValue;

#[global_allocator]
static ALLOCATOR: GlobalDlmalloc = GlobalDlmalloc;

/// Mutable state isolated to one instantiated Phase-1 runtime.
pub(crate) struct RuntimeState {
    /// Runtime values addressed by one-based `ValueRef` indices.
    pub(crate) values: Vec<RtValue>,
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
