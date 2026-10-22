//! The tasks of a resumable execution and their deterministic scheduling.
//!
//! Every task is a heap object linked into the list of live tasks. Runnable tasks wait in a
//! first-in, first-out run queue; the running task keeps its frame, host call, ready result
//! and quantum in the VM's task state and saves them into its object when it stops. A task
//! yields to runnable siblings after [`TASK_QUANTUM`] scheduler checkpoints.

use core::mem::{offset_of, size_of};

use exs_abi::STATUS_PENDING;
use exs_layout::STATUS_READY;
use exs_layout::{FRAME_UNTRACED, FrameObject, ObjectKind, TaskObject};

use super::host::Host;
use super::state::{Field, load, load_address, store, store_address};
use crate::allocator::allocate;
use crate::lists;
use crate::memory::Memory;
use crate::vm::Vm;

/// Number of scheduler checkpoints a task may pass before it yields.
pub(super) const TASK_QUANTUM: u64 = 64;

/// Byte offset of the state code in a task's header word.
const STATE: u32 = 4;
/// Byte offset of [`TaskObject::frame`].
const FRAME: u32 = offset_of!(TaskObject, frame) as u32;
/// Byte offset of [`TaskObject::queue`].
const QUEUE: u32 = offset_of!(TaskObject, queue) as u32;
/// Byte offset of [`TaskObject::link`].
const LINK: u32 = offset_of!(TaskObject, link) as u32;
/// Byte offset of [`TaskObject::parent`].
const PARENT: u32 = offset_of!(TaskObject, parent) as u32;
/// Byte offset of [`TaskObject::ready`].
const READY: u32 = offset_of!(TaskObject, ready) as u32;
/// Byte offset of [`TaskObject::group`].
const GROUP: u32 = offset_of!(TaskObject, group) as u32;
/// Byte offset of [`TaskObject::host_call`].
const HOST_CALL: u32 = offset_of!(TaskObject, host_call) as u32;
/// Byte offset of [`TaskObject::index`].
const INDEX: u32 = offset_of!(TaskObject, index) as u32;
/// Byte offset of [`TaskObject::waiting`].
const WAITING: u32 = offset_of!(TaskObject, waiting) as u32;
/// Byte offset of [`TaskObject::quantum`].
const QUANTUM: u32 = offset_of!(TaskObject, quantum) as u32;

/// Byte offset of [`FrameObject::caller`].
const FRAME_CALLER: u32 = offset_of!(FrameObject, caller) as u32;
/// Byte offset of [`FrameObject::trace`].
const FRAME_TRACE: u32 = offset_of!(FrameObject, trace) as u32;

/// The lifecycle state of one task, stored in its header word.
#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u32)]
enum TaskState {
    /// Waiting in the run queue.
    Runnable = 0,
    /// Running.
    Running = 1,
    /// Suspended until the runner delivers its host call's response.
    WaitingHost = 2,
    /// Suspended until its `par` children complete.
    WaitingChildren = 3,
}

/// Sets the state of the task at `task`.
fn set_state(memory: &mut impl Memory, task: u32, state: TaskState) {
    memory.store_u32(task + STATE, state as u32);
}

/// Returns whether the task at `task` is in `state`.
fn has_state(memory: &impl Memory, task: u32, state: TaskState) -> bool {
    memory.load_u32(task + STATE) == state as u32
}

/// Allocates a task whose active frame is `frame`, links it into the live tasks and returns
/// its address. Its frame must be rooted by the caller.
pub(super) fn task_new(memory: &mut impl Memory, vm: u32, frame: u32) -> Option<u32> {
    let task = allocate(memory, vm, size_of::<TaskObject>() as u32, ObjectKind::Task)?;
    memory.store_u32(task + FRAME, frame);
    memory.store_u32(task + QUANTUM, TASK_QUANTUM as u32);
    memory.store_u32(task + LINK, load_address(memory, vm, Field::Tasks));
    store_address(memory, vm, Field::Tasks, task);
    Some(task)
}

/// Makes the task at `task` the running task with its saved state, rebuilding the Error
/// trace of its frames. Returns `None` when the trace stack is too small.
pub(super) fn run(memory: &mut impl Memory, vm: u32, task: u32) -> Option<()> {
    set_state(memory, task, TaskState::Running);
    store_address(memory, vm, Field::Current, task);
    store_address(memory, vm, Field::Frame, memory.load_u32(task + FRAME));
    store(memory, vm, Field::Ready, memory.load_u64(task + READY));
    store(
        memory,
        vm,
        Field::HostCall,
        memory.load_u64(task + HOST_CALL),
    );
    store(
        memory,
        vm,
        Field::Quantum,
        u64::from(memory.load_u32(task + QUANTUM)),
    );
    rebuild_trace(memory, vm, task)
}

/// Saves the running task's state into its object and leaves no task running. Returns the
/// task's address, 0 when none ran.
fn stop(memory: &mut impl Memory, vm: u32) -> u32 {
    let task = load_address(memory, vm, Field::Current);
    if task == 0 {
        return 0;
    }
    memory.store_u32(task + FRAME, load_address(memory, vm, Field::Frame));
    memory.store_u64(task + READY, load(memory, vm, Field::Ready));
    memory.store_u64(task + HOST_CALL, load(memory, vm, Field::HostCall));
    memory.store_u32(task + QUANTUM, load(memory, vm, Field::Quantum) as u32);
    for field in [Field::Current, Field::Frame, Field::Ready, Field::HostCall] {
        store(memory, vm, field, 0);
    }
    task
}

/// Appends the task at `task` to the run queue.
fn enqueue(memory: &mut impl Memory, vm: u32, task: u32) {
    set_state(memory, task, TaskState::Runnable);
    memory.store_u32(task + QUEUE, 0);
    let tail = load_address(memory, vm, Field::QueueTail);
    if tail == 0 {
        store_address(memory, vm, Field::QueueHead, task);
    } else {
        memory.store_u32(tail + QUEUE, task);
    }
    store_address(memory, vm, Field::QueueTail, task);
}

/// Runs the first task of the run queue, returning `STATUS_READY`, or `STATUS_PENDING` when
/// no task is runnable. Returns `None` when the trace stack is too small.
pub(super) fn schedule(memory: &mut impl Memory, vm: u32) -> Option<i32> {
    let task = load_address(memory, vm, Field::QueueHead);
    if task == 0 {
        return Some(STATUS_PENDING);
    }
    let next = memory.load_u32(task + QUEUE);
    store_address(memory, vm, Field::QueueHead, next);
    if next == 0 {
        store_address(memory, vm, Field::QueueTail, 0);
    }
    run(memory, vm, task)?;
    Some(STATUS_READY)
}

/// Rebuilds the Error trace of the task at `task`: the traced frames of the tasks it was
/// started from, outermost first, then its own. The root frame of a `par` child has
/// no entry.
fn rebuild_trace(memory: &mut impl Memory, vm: u32, task: u32) -> Option<()> {
    let state = Vm(vm);
    let base = state.trace_base(memory);
    let mut count = 0_u32;
    each_trace(memory, task, |_, _| count += 1);
    let top = count.checked_mul(8)?.checked_add(base)?;
    if top > state.trace_limit(memory) {
        return None;
    }
    // Frames are visited innermost first, so entries are written from the top down.
    let mut entry = top;
    each_trace(memory, task, |memory, trace| {
        entry -= 8;
        memory.store_u64(entry, trace);
    });
    state.set_trace_top(memory, top);
    Some(())
}

/// Calls `visit` with the trace entry of every traced frame of the task at `task` and of the
/// tasks it was started from, innermost first.
fn each_trace<M: Memory>(memory: &mut M, task: u32, mut visit: impl FnMut(&mut M, u64)) {
    let mut current = task;
    while current != 0 {
        let mut frame = memory.load_u32(current + FRAME);
        while frame != 0 {
            let trace = memory.load_u64(frame + FRAME_TRACE);
            if trace != FRAME_UNTRACED {
                visit(memory, trace);
            }
            frame = memory.load_u32(frame + FRAME_CALLER);
        }
        current = memory.load_u32(current + PARENT);
    }
}

/// Suspends the running task until the runner delivers the response of its pending host
/// call and runs the next runnable task. Returns the dispatch status.
pub fn task_suspend(memory: &mut impl Memory, vm: u32) -> Option<i32> {
    let task = stop(memory, vm);
    if task != 0 {
        set_state(memory, task, TaskState::WaitingHost);
    }
    schedule(memory, vm)
}

/// Passes one scheduler checkpoint of the running task, which yields to the next runnable
/// task once its quantum is used up.
pub(super) fn checkpoint(memory: &mut impl Memory, vm: u32) -> Option<()> {
    let quantum = load(memory, vm, Field::Quantum).saturating_sub(1);
    store(memory, vm, Field::Quantum, quantum);
    if quantum == 0 {
        task_checkpoint(memory, vm)?;
    }
    Some(())
}

/// Handles the running task's used-up quantum, which generated code counts down inline: the
/// quantum restarts, and when another task is runnable the running task yields to it.
/// Returns 1 when it yielded, else 0.
pub fn task_checkpoint(memory: &mut impl Memory, vm: u32) -> Option<i32> {
    store(memory, vm, Field::Quantum, TASK_QUANTUM);
    if load_address(memory, vm, Field::QueueHead) == 0 {
        return Some(0);
    }
    let task = stop(memory, vm);
    enqueue(memory, vm, task);
    schedule(memory, vm)?;
    Some(1)
}

/// Delivers a runner response decoded into `value` to the task waiting for host call
/// `call_id`, which becomes runnable and runs when no task does. Returns `false` when no
/// task waits for the call, because it was cancelled.
pub(super) fn deliver(memory: &mut impl Memory, vm: u32, call_id: u64, value: u64) -> Option<bool> {
    let mut task = load_address(memory, vm, Field::Tasks);
    while task != 0 {
        if has_state(memory, task, TaskState::WaitingHost)
            && memory.load_u64(task + HOST_CALL) == call_id
        {
            memory.store_u64(task + READY, value);
            enqueue(memory, vm, task);
            if load_address(memory, vm, Field::Current) == 0 {
                schedule(memory, vm)?;
            }
            return Some(true);
        }
        task = memory.load_u32(task + LINK);
    }
    Some(false)
}

/// Returns whether a live task waits for host call `call_id`.
pub(super) fn waits_for(memory: &impl Memory, vm: u32, call_id: u64) -> bool {
    let mut task = load_address(memory, vm, Field::Tasks);
    while task != 0 {
        if has_state(memory, task, TaskState::WaitingHost)
            && memory.load_u64(task + HOST_CALL) == call_id
        {
            return true;
        }
        task = memory.load_u32(task + LINK);
    }
    false
}

/// Creates the result List of a `par` expression with `count` tasks, which the running task
/// keeps in its frame while its children fill it.
pub fn parallel_new(memory: &mut impl Memory, vm: u32, count: u32) -> Option<u64> {
    let list = lists::allocate_list(memory, vm, count)?;
    lists::set_len(memory, list, count);
    lists::list_reference(list)
}

/// Starts a `par` child of the running task that completes element `index` of the rooted
/// result List `group`: its root frame of the step function `step` has `slot_count` empty
/// slots and no trace entry. Returns the frame's address for the arguments.
pub fn task_spawn(
    memory: &mut impl Memory,
    host: &mut impl Host,
    vm: u32,
    group: u64,
    index: u32,
    step: u32,
    slot_count: u32,
) -> Option<u32> {
    if !host.acquire() {
        return None;
    }
    let parent = load_address(memory, vm, Field::Current);
    // The task is linked into the live tasks, and so rooted, before its frame is allocated.
    let task = task_new(memory, vm, 0)?;
    memory.store_u32(task + PARENT, parent);
    memory.store_u64(task + GROUP, group);
    memory.store_u32(task + INDEX, index);
    let frame = super::frames::allocate_frame(memory, vm, step, slot_count, 0, 0)?;
    memory.store_u32(task + FRAME, frame);
    enqueue(memory, vm, task);
    if parent != 0 {
        let waiting = memory.load_u32(parent + WAITING);
        memory.store_u32(parent + WAITING, waiting.checked_add(1)?);
    }
    Some(frame)
}

/// Suspends the running task until its `par` children complete, unless they already did.
/// Returns the dispatch status.
pub fn parallel_wait(memory: &mut impl Memory, vm: u32) -> Option<i32> {
    let task = load_address(memory, vm, Field::Current);
    if task == 0 || memory.load_u32(task + WAITING) == 0 {
        return Some(STATUS_READY);
    }
    stop(memory, vm);
    set_state(memory, task, TaskState::WaitingChildren);
    schedule(memory, vm)
}

/// Returns whether the running task is a `par` child.
pub(super) fn running_child(memory: &impl Memory, vm: u32) -> bool {
    let task = load_address(memory, vm, Field::Current);
    task != 0 && memory.load_u32(task + PARENT) != 0
}

/// Completes the running `par` child with `value`: stores it in its result List, wakes its
/// parent once every sibling completed, releases the child's permit and runs the next
/// runnable task. Returns the dispatch status.
pub(super) fn complete_child(
    memory: &mut impl Memory,
    host: &mut impl Host,
    vm: u32,
    value: u64,
) -> Option<i32> {
    let task = stop(memory, vm);
    if let Some(list) = lists::list_object(memory.load_u64(task + GROUP)) {
        let index = memory.load_u32(task + INDEX);
        if index < lists::len(memory, list) {
            memory.store_u64(lists::item_address(memory, list, index), value);
        }
    }
    let parent = memory.load_u32(task + PARENT);
    let waiting = memory.load_u32(parent + WAITING).saturating_sub(1);
    memory.store_u32(parent + WAITING, waiting);
    if waiting == 0 && has_state(memory, parent, TaskState::WaitingChildren) {
        enqueue(memory, vm, parent);
    }
    unlink(memory, vm, task);
    host.release();
    schedule(memory, vm)
}

/// Removes the task at `task` from the live tasks.
fn unlink(memory: &mut impl Memory, vm: u32, task: u32) {
    let next = memory.load_u32(task + LINK);
    let mut previous = load_address(memory, vm, Field::Tasks);
    if previous == task {
        store_address(memory, vm, Field::Tasks, next);
        return;
    }
    while previous != 0 {
        let link = memory.load_u32(previous + LINK);
        if link == task {
            memory.store_u32(previous + LINK, next);
            return;
        }
        previous = link;
    }
}

/// Ends every task: releases the permits of all `par` children, whose host calls are
/// abandoned, and clears the scheduler. The root task's permit stays with the caller.
pub(super) fn end_tasks(memory: &mut impl Memory, host: &mut impl Host, vm: u32) {
    let mut task = load_address(memory, vm, Field::Tasks);
    while task != 0 {
        if memory.load_u32(task + PARENT) != 0 {
            host.release();
        }
        task = memory.load_u32(task + LINK);
    }
    for field in [
        Field::Frame,
        Field::Ready,
        Field::Current,
        Field::QueueHead,
        Field::QueueTail,
        Field::Tasks,
        Field::HostCall,
    ] {
        store(memory, vm, field, 0);
    }
    let state = Vm(vm);
    state.set_trace_top(memory, state.trace_base(memory));
}
