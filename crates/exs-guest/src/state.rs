//! Singleton state and target runtime support for one guest Wasm instance.

use alloc::vec::Vec;
use core::cell::UnsafeCell;

use crate::{ExsValue, GuestFuture};

/// Retains the guest future or completed result for the current guest instance.
pub(crate) enum Execution {
    /// No execution is active and no result is available.
    Idle,
    /// One future is suspended or currently being polled by a runner callback.
    Running(GuestFuture),
    /// One completed CBOR result remains readable by the runner.
    Completed(Vec<u8>),
}

/// Stores asynchronous host responses and the next stable host-call identity.
pub(crate) struct HostState {
    /// Next host-call identifier to assign.
    pub(crate) next_call_id: i64,
    /// Responses supplied through `__exs_resume_host` but not yet consumed by a future.
    pub(crate) responses: Vec<(i64, ExsValue)>,
}

impl HostState {
    /// Creates empty per-instance host-call state.
    pub(crate) const fn new() -> Self {
        Self {
            next_call_id: 1,
            responses: Vec::new(),
        }
    }
}

/// Provides interior mutability for the single-threaded Wasm guest instance.
pub(crate) struct GuestCell<T>(UnsafeCell<T>);

// The ABI invokes a guest instance serially; Wasm threads are not supported by this SDK.
unsafe impl<T> Sync for GuestCell<T> {}

/// Retains every writable runner buffer until the matching ABI callback consumes it.
static BUFFERS: GuestCell<Vec<Vec<u8>>> = GuestCell(UnsafeCell::new(Vec::new()));
/// Retains the active or completed guest root execution.
static EXECUTION: GuestCell<Execution> = GuestCell(UnsafeCell::new(Execution::Idle));
/// Retains host completions independent of the currently polled future borrow.
static HOST_STATE: GuestCell<HostState> = GuestCell(UnsafeCell::new(HostState::new()));

/// Returns mutable access to runner-writable guest buffers.
pub(crate) fn buffers_mut() -> &'static mut Vec<Vec<u8>> {
    // Wasm runner calls are serialized for each instantiated module.
    unsafe { &mut *BUFFERS.0.get() }
}

/// Returns mutable access to the active root execution state.
pub(crate) fn execution_mut() -> &'static mut Execution {
    // Wasm runner calls are serialized for each instantiated module.
    unsafe { &mut *EXECUTION.0.get() }
}

/// Returns mutable access to host-call continuation state.
pub(crate) fn host_state_mut() -> &'static mut HostState {
    // Wasm runner calls are serialized for each instantiated module.
    unsafe { &mut *HOST_STATE.0.get() }
}

/// Supplies the global allocator required by `alloc` on no-std Wasm guests.
#[cfg(all(target_arch = "wasm32", feature = "no-std"))]
#[global_allocator]
static ALLOCATOR: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

/// Traps immediately when a no-std guest panics.
#[cfg(all(target_arch = "wasm32", feature = "no-std"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}
