//! Runtime-owned mutable state for one Wasm instance.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::UnsafeCell;

use dlmalloc::GlobalDlmalloc;
use exs_abi::SourcePositionId;
use exs_value::ValueRef;

use crate::scheduler::ExecutionContext;
use crate::value::RtValue;

#[global_allocator]
static ALLOCATOR: GlobalDlmalloc = GlobalDlmalloc;

/// One runtime value-table entry and its collector mark state.
pub(crate) struct HeapSlot {
    /// Whether the current collection has reached this value.
    ///
    /// Always false outside a collection: the sweep resets it on every survivor.
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

/// Shadow stack of compiler-generated roots that generated code writes directly in memory.
///
/// Each slot holds a raw 64-bit `ValueRef`, or 0 when empty. Frames are pushed and popped by
/// moving the root-top register; the runtime only switches chunks when a frame does not fit.
/// Chunks are never reallocated while they hold active frames, so frame addresses stay valid.
pub(crate) struct RootStack {
    /// Fixed-size slot chunks, kept after use so later frames can reuse them.
    pub(crate) chunks: Vec<Box<[u64]>>,
    /// Index of the chunk holding the innermost frames, or `None` before the first frame.
    pub(crate) current: Option<usize>,
    /// Root-top address of every chunk below the current one, in chunk order.
    pub(crate) saved_tops: Vec<u32>,
}

impl RootStack {
    /// Creates an empty root stack without allocating chunks.
    const fn new() -> Self {
        Self {
            chunks: Vec::new(),
            current: None,
            saved_tops: Vec::new(),
        }
    }
}

/// Language stack of active direct calls, stored in linear memory for generated code.
///
/// Each entry holds the `u32` function id followed by the `u32` call site; the trace-top
/// register marks the next free entry. Only the runtime keeps addresses into the buffer, so it
/// may move the buffer when it grows.
pub(crate) struct TraceStack {
    /// Fixed-length entry storage, replaced when it grows; its used prefix ends at the
    /// trace-top register.
    pub(crate) buffer: Vec<u64>,
}

impl TraceStack {
    /// Creates an empty trace stack without allocating storage.
    const fn new() -> Self {
        Self { buffer: Vec::new() }
    }
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
    /// Linear-memory block that generated code reads and writes directly.
    ///
    /// Word 0 holds the continuation state in its low and the function identifier in its high
    /// 32 bits (see `exs_abi::ASYNC_FRAME_*_OFFSET`). Every later word is one raw `ValueRef`
    /// slot, or 0 while empty. The boxed slice never moves while the frame is alive.
    pub(crate) block: Box<[u64]>,
    /// Caller continuation, absent for the root resumable invocation.
    pub(crate) caller: Option<FrameContinuation>,
    /// Whether this frame owns one entry in the shared language error-trace stack.
    pub(crate) traced: bool,
}

impl AsyncFrame {
    /// Creates a frame at state 0 with `slot_count` empty slots.
    ///
    /// Reuses a pooled block of the same size when one is available.
    pub(crate) fn new(function_id: u32, slot_count: usize, traced: bool) -> Self {
        let pool = &mut unsafe { runtime() }.frame_block_pool;
        let mut block = match pool.iter().rposition(|block| block.len() == slot_count + 1) {
            Some(position) => {
                let mut block = pool.swap_remove(position);
                block.fill(0);
                block
            }
            None => {
                let mut block = Vec::new();
                block.resize(slot_count + 1, 0);
                block.into_boxed_slice()
            }
        };
        block[0] = u64::from(function_id) << 32;
        Self {
            block,
            caller: None,
            traced,
        }
    }

    /// Returns this frame's block to the reuse pool and yields its caller continuation.
    pub(crate) fn release(self) -> Option<FrameContinuation> {
        let pool = &mut unsafe { runtime() }.frame_block_pool;
        if pool.len() < FRAME_BLOCK_POOL_LEN {
            pool.push(self.block);
        }
        self.caller
    }

    /// Returns the compiler-assigned generated function identifier.
    pub(crate) fn function_id(&self) -> u32 {
        (self.block[0] >> 32) as u32
    }

    /// Returns the raw `ValueRef` slots following the header word.
    pub(crate) fn slots(&self) -> &[u64] {
        &self.block[1..]
    }

    /// Returns the mutable raw `ValueRef` slots following the header word.
    pub(crate) fn slots_mut(&mut self) -> &mut [u64] {
        &mut self.block[1..]
    }
}

/// Mutable state isolated to one instantiated Phase-1 runtime.
pub(crate) struct RuntimeState {
    /// Runtime values addressed by one-based `ValueRef` indices.
    pub(crate) values: Vec<Option<HeapSlot>>,
    /// Reusable zero-based indices of swept value-table entries.
    pub(crate) free_slots: Vec<u32>,
    /// Value-table size that triggers the next automatic garbage collection.
    pub(crate) next_gc_at: usize,
    /// Mark worklist reused by every collection to keep its capacity.
    pub(crate) gc_worklist: Vec<ValueRef>,
    /// Released async frame blocks kept for reuse by later frames of the same size.
    pub(crate) frame_block_pool: Vec<Box<[u64]>>,
    /// Compiler-generated function roots written directly by generated code.
    pub(crate) root_stack: RootStack,
    /// Durable frames for the currently active resumable root execution.
    pub(crate) async_frames: Vec<Option<AsyncFrame>>,
    /// Reusable zero-based async-frame indexes.
    pub(crate) free_async_frames: Vec<u32>,
    /// Scheduler state for the active root execution, if any.
    pub(crate) execution: Option<ExecutionContext>,
    /// Root result retained after the final resumable frame completes.
    pub(crate) completed_async_result: Option<ValueRef>,
    /// CBOR request bytes for the active generic host call.
    pub(crate) host_request_buffer: Vec<u8>,
    /// CBOR response bytes copied from the runner for a synchronous host result.
    pub(crate) host_response_buffer: Vec<u8>,
    /// The next monotonic HostCallId assigned within this Wasm instance.
    pub(crate) next_host_call_id: u64,
    /// Native runtime values temporarily protected across further allocations.
    pub(crate) temporary_roots: Vec<ValueRef>,
    /// Source position applied to the next recoverable runtime Error.
    pub(crate) current_source_position: Option<SourcePositionId>,
    /// Active direct language calls from root to innermost frame.
    pub(crate) trace_stack: TraceStack,
    /// CBOR bytes supplied by the runner for the next root execution.
    pub(crate) input_buffer: Vec<u8>,
    /// Bytes copied from one compiler-owned passive literal data segment.
    pub(crate) literal_buffer: Vec<u8>,
    /// CBOR bytes holding the completed root result.
    pub(crate) result_buffer: Vec<u8>,
    /// String literal values cached in compiler globals, kept alive as permanent GC roots.
    pub(crate) pinned_literals: Vec<ValueRef>,
}

impl RuntimeState {
    /// Creates empty value and CBOR buffer stores.
    const fn new() -> Self {
        Self {
            values: Vec::new(),
            free_slots: Vec::new(),
            next_gc_at: crate::gc::INITIAL_COLLECTION_THRESHOLD,
            gc_worklist: Vec::new(),
            frame_block_pool: Vec::new(),
            root_stack: RootStack::new(),
            async_frames: Vec::new(),
            free_async_frames: Vec::new(),
            execution: None,
            completed_async_result: None,
            host_request_buffer: Vec::new(),
            host_response_buffer: Vec::new(),
            next_host_call_id: 1,
            temporary_roots: Vec::new(),
            current_source_position: None,
            trace_stack: TraceStack::new(),
            input_buffer: Vec::new(),
            literal_buffer: Vec::new(),
            result_buffer: Vec::new(),
            pinned_literals: Vec::new(),
        }
    }
}

/// Maximum number of released async frame blocks kept for reuse.
const FRAME_BLOCK_POOL_LEN: usize = 16;

/// Runtime registers that generated code reads and writes directly in linear memory.
///
/// The compiler addresses this block through the static's exported address, so generated
/// code needs neither a runtime call nor a Wasm global to reach it.
#[repr(C)]
pub struct RuntimeRegisters {
    /// Call-site source position for the next generated function entry, or 0 when absent.
    pub pending_call_site: u32,
    /// Remaining scheduler checkpoints of the running task.
    pub quantum_remaining: u32,
    /// Address of the next free root-stack slot in the current chunk.
    pub root_top: u32,
    /// End address of the current root-stack chunk, or 0 before the first frame.
    pub root_limit: u32,
    /// Start address of the current root-stack chunk.
    pub root_floor: u32,
    /// Address of the next free error-trace entry.
    pub trace_top: u32,
    /// End address of the error-trace buffer, or 0 before the first entry.
    pub trace_limit: u32,
}

/// Interior-mutable holder of the runtime registers.
#[repr(transparent)]
pub struct RegistersCell(pub UnsafeCell<RuntimeRegisters>);

// SAFETY: the runtime is single-threaded; generated code and the runtime never run concurrently.
unsafe impl Sync for RegistersCell {}

/// The runtime register block exported by address to generated code.
#[unsafe(no_mangle)]
pub static __EXS_RT_REGISTERS: RegistersCell = RegistersCell(UnsafeCell::new(RuntimeRegisters {
    pending_call_site: 0,
    quantum_remaining: 0,
    root_top: 0,
    root_limit: 0,
    root_floor: 0,
    trace_top: 0,
    trace_limit: 0,
}));

// Generated code addresses the registers with the shared ABI offsets.
const _: () = assert!(
    core::mem::offset_of!(RuntimeRegisters, pending_call_site)
        == exs_abi::REGISTERS_CALL_SITE_OFFSET as usize
);
const _: () = assert!(
    core::mem::offset_of!(RuntimeRegisters, quantum_remaining)
        == exs_abi::REGISTERS_QUANTUM_OFFSET as usize
);
const _: () = assert!(
    core::mem::offset_of!(RuntimeRegisters, root_top)
        == exs_abi::REGISTERS_ROOT_TOP_OFFSET as usize
);
const _: () = assert!(
    core::mem::offset_of!(RuntimeRegisters, root_limit)
        == exs_abi::REGISTERS_ROOT_LIMIT_OFFSET as usize
);
const _: () = assert!(
    core::mem::offset_of!(RuntimeRegisters, root_floor)
        == exs_abi::REGISTERS_ROOT_FLOOR_OFFSET as usize
);
const _: () = assert!(
    core::mem::offset_of!(RuntimeRegisters, trace_top)
        == exs_abi::REGISTERS_TRACE_TOP_OFFSET as usize
);
const _: () = assert!(
    core::mem::offset_of!(RuntimeRegisters, trace_limit)
        == exs_abi::REGISTERS_TRACE_LIMIT_OFFSET as usize
);

/// Returns the runtime registers shared with generated code.
///
/// # Safety
///
/// Callers must not retain the reference across generated code that writes the registers.
pub(crate) unsafe fn registers() -> &'static mut RuntimeRegisters {
    unsafe { &mut *__EXS_RT_REGISTERS.0.get() }
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
