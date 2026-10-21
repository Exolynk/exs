//! Frames of resumable functions: creation, calls and completion.

use core::mem::{offset_of, size_of};

use exs_abi::{STATUS_COMPLETE, STATUS_READY};
use exs_layout::{FRAME_UNTRACED, FrameObject, ObjectKind};

use super::host::Host;
use super::scheduler;
use super::state::{Field, load_address, store, store_address};
use crate::allocator::allocate;
use crate::memory::Memory;
use crate::vm::Vm;

/// Byte offset of the step index in a frame's header word.
const STEP: u32 = 4;
/// Byte offset of [`FrameObject::caller`].
const CALLER: u32 = offset_of!(FrameObject, caller) as u32;
/// Byte offset of [`FrameObject::destination`].
const DESTINATION: u32 = offset_of!(FrameObject, destination) as u32;
/// Byte offset of [`FrameObject::slot_count`].
const SLOT_COUNT: u32 = offset_of!(FrameObject, slot_count) as u32;
/// Byte offset of [`FrameObject::trace`].
const TRACE: u32 = offset_of!(FrameObject, trace) as u32;
/// Byte offset of the first durable slot.
const SLOTS: u32 = size_of::<FrameObject>() as u32;

/// Allocates an untraced frame of the step function `step` with `slot_count` empty slots that
/// returns into slot `destination` of the frame at `caller`, or completes its task when
/// `caller` is 0.
pub(super) fn allocate_frame(
    memory: &mut impl Memory,
    vm: u32,
    step: u32,
    slot_count: u32,
    caller: u32,
    destination: u32,
) -> Option<u32> {
    let size = slot_count.checked_mul(8)?.checked_add(SLOTS)?;
    let frame = allocate(memory, vm, size, ObjectKind::Frame)?;
    memory.store_u32(frame + STEP, step);
    memory.store_u32(frame + CALLER, caller);
    memory.store_u32(frame + DESTINATION, destination);
    memory.store_u32(frame + SLOT_COUNT, slot_count);
    memory.store_u64(frame + TRACE, FRAME_UNTRACED);
    Some(frame)
}

/// Pushes the trace entry of the frame at `frame` of the function `function_id`, with the
/// pending call site like a direct call, and records it in the frame. Returns `None` when
/// the trace stack is full.
fn push_trace(memory: &mut impl Memory, vm: u32, frame: u32, function_id: u32) -> Option<()> {
    let state = Vm(vm);
    let top = state.trace_top(memory);
    if top.checked_add(8)? > state.trace_limit(memory) {
        return None;
    }
    let site = state.pending_call_site(memory);
    let entry = u64::from(function_id) | u64::from(site) << 32;
    memory.store_u64(top, entry);
    memory.store_u64(frame + TRACE, entry);
    state.set_pending_call_site(memory, 0);
    state.set_trace_top(memory, top + 8);
    Some(())
}

/// Starts the execution with the root task, whose root frame of the step function `step` of
/// the function `function_id` has `slot_count` empty slots, and returns the frame's address.
pub fn frame_start(
    memory: &mut impl Memory,
    vm: u32,
    step: u32,
    function_id: u32,
    slot_count: u32,
) -> Option<u32> {
    for field in [
        Field::Frame,
        Field::Ready,
        Field::Completed,
        Field::Current,
        Field::QueueHead,
        Field::QueueTail,
        Field::Tasks,
        Field::HostCall,
    ] {
        store(memory, vm, field, 0);
    }
    let frame = allocate_frame(memory, vm, step, slot_count, 0, 0)?;
    store_address(memory, vm, Field::Frame, frame);
    let task = scheduler::task_new(memory, vm, frame)?;
    store_address(memory, vm, Field::Current, task);
    store(memory, vm, Field::Quantum, scheduler::TASK_QUANTUM);
    push_trace(memory, vm, frame, function_id)?;
    Some(frame)
}

/// Calls the step function `step` of the function `function_id` from the active frame: the
/// new frame of `slot_count` empty slots becomes active and returns into the caller's slot
/// `destination`. The call is a scheduler checkpoint. Returns the new frame's address for
/// its arguments.
pub fn frame_call(
    memory: &mut impl Memory,
    vm: u32,
    step: u32,
    function_id: u32,
    slot_count: u32,
    destination: u32,
) -> Option<u32> {
    // The caller stays rooted as the active frame while the child is allocated.
    let caller = load_address(memory, vm, Field::Frame);
    let frame = allocate_frame(memory, vm, step, slot_count, caller, destination)?;
    store_address(memory, vm, Field::Frame, frame);
    push_trace(memory, vm, frame, function_id)?;
    scheduler::checkpoint(memory, vm)?;
    Some(frame)
}

/// Completes the active frame at `frame` with `value` and returns the dispatch status.
///
/// The caller receives the value and becomes active, returning `STATUS_READY`. The root
/// frame of a `par` child completes the child and runs the next runnable task. The root
/// task's root frame, and like v1 any frame returning a fatal Error, end every task and
/// complete the execution with `STATUS_COMPLETE`.
pub fn frame_return(
    memory: &mut impl Memory,
    host: &mut impl Host,
    vm: u32,
    frame: u32,
    value: u64,
) -> Option<i32> {
    let caller = memory.load_u32(frame + CALLER);
    let fatal = crate::errors::is_fatal(memory, value);
    if caller == 0 && !fatal && scheduler::running_child(memory, vm) {
        return scheduler::complete_child(memory, host, vm, value);
    }
    if caller == 0 || fatal {
        scheduler::end_tasks(memory, host, vm);
        store(memory, vm, Field::Completed, value);
        return Some(STATUS_COMPLETE);
    }
    if memory.load_u64(frame + TRACE) != FRAME_UNTRACED {
        let state = Vm(vm);
        state.set_trace_top(memory, state.trace_top(memory).saturating_sub(8));
    }
    let destination = memory.load_u32(frame + DESTINATION);
    memory.store_u64(caller + SLOTS + destination * 8, value);
    store_address(memory, vm, Field::Frame, caller);
    Some(STATUS_READY)
}

/// Takes the result of the completed root task.
pub fn take_completed(memory: &mut impl Memory, vm: u32) -> u64 {
    let value = super::state::load(memory, vm, Field::Completed);
    store(memory, vm, Field::Completed, 0);
    value
}
