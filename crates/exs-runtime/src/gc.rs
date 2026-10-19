//! Stop-the-world mark-and-sweep collection and compiler root-frame management.

use alloc::vec::Vec;
use core::num::NonZeroU32;

use exs_value::ValueRef;

use crate::state::{RootFrame, runtime};
use crate::value::RtValue;

/// Minimum value-table size before automatic collection becomes worthwhile.
pub(crate) const INITIAL_COLLECTION_THRESHOLD: usize = 256;

/// Minimum slot count of one shadow root stack chunk.
const ROOT_CHUNK_SLOTS: usize = 4096;

/// Creates one zeroed compiler-generated root frame and returns its first slot's address.
///
/// Generated code roots a value with a plain `i32.store` of its raw index at `base + slot * 4`
/// and clears it by storing 0, so the slot count must cover every slot the compiler emits.
pub(crate) fn push_root_frame(slot_count: i32) -> i32 {
    let Ok(len) = usize::try_from(slot_count) else {
        crate::runtime::trap();
    };
    let stack = &mut unsafe { runtime() }.root_stack;
    let (mut chunk, mut start) = stack
        .frames
        .last()
        .map_or((0, 0), |frame| (frame.chunk, frame.start + frame.len));
    if stack
        .chunks
        .get(chunk)
        .is_some_and(|slots| start + len > slots.len())
    {
        chunk += 1;
        start = 0;
    }
    // Chunks after the active top frame hold no live frames, so an undersized one may be replaced.
    if stack
        .chunks
        .get(chunk)
        .is_none_or(|slots| len > slots.len())
    {
        let mut slots = Vec::new();
        slots.resize(len.max(ROOT_CHUNK_SLOTS), 0);
        let slots = slots.into_boxed_slice();
        if chunk < stack.chunks.len() {
            stack.chunks[chunk] = slots;
        } else {
            stack.chunks.push(slots);
        }
    }
    let Some(slots) = stack.chunks[chunk].get_mut(start..start + len) else {
        crate::runtime::trap();
    };
    slots.fill(0);
    let Ok(base) = i32::try_from(slots.as_mut_ptr() as usize) else {
        crate::runtime::trap();
    };
    stack.frames.push(RootFrame {
        chunk,
        start,
        len,
        base,
    });
    base
}

/// Removes the current compiler-generated root frame identified by its base address.
pub(crate) fn pop_root_frame(base: i32) {
    let stack = &mut unsafe { runtime() }.root_stack;
    if stack.frames.last().is_none_or(|frame| frame.base != base) {
        crate::runtime::trap();
    }
    let _removed = stack.frames.pop();
}

/// Returns a checkpoint for temporary native runtime roots.
pub(crate) fn temporary_root_checkpoint() -> usize {
    unsafe { runtime() }.temporary_roots.len()
}

/// Protects one native runtime value until its checkpoint is restored.
pub(crate) fn push_temporary_root(value: ValueRef) {
    unsafe { runtime() }.temporary_roots.push(value);
}

/// Restores the temporary-root stack to one earlier checkpoint.
pub(crate) fn restore_temporary_roots(checkpoint: usize) {
    let roots = &mut unsafe { runtime() }.temporary_roots;
    if checkpoint > roots.len() {
        crate::runtime::trap();
    }
    roots.truncate(checkpoint);
}

/// Marks every value reachable from runtime and compiler roots, then sweeps all other slots.
pub(crate) fn collect() {
    let mut worklist = collect_roots();
    while let Some(reference) = worklist.pop() {
        mark(reference, &mut worklist);
    }
    let live_values = sweep();
    let state = unsafe { runtime() };
    state.next_gc_at = live_values
        .saturating_mul(2)
        .max(INITIAL_COLLECTION_THRESHOLD);
}

/// Copies the current root set after clearing the previous collection's marks.
fn collect_roots() -> Vec<ValueRef> {
    let state = unsafe { runtime() };
    for slot in state.values.iter_mut().flatten() {
        slot.marked = false;
    }
    let mut roots = Vec::new();
    for frame in &state.root_stack.frames {
        let slots = &state.root_stack.chunks[frame.chunk][frame.start..frame.start + frame.len];
        roots.extend(slots.iter().copied().filter_map(raw_root));
    }
    for frame in state.async_frames.iter().flatten() {
        roots.extend(frame.slots.iter().flatten().copied());
    }
    roots.extend(state.completed_async_result);
    if let Some(execution) = &state.execution {
        roots.extend(execution.roots());
    }
    roots.extend(state.temporary_roots.iter().copied());
    roots.extend(state.shared_values.iter());
    roots.extend(state.pinned_literals.iter().copied());
    roots
}

/// Marks one reachable slot and queues its owned child references.
fn mark(reference: ValueRef, worklist: &mut Vec<ValueRef>) {
    let index = value_index(reference);
    let state = unsafe { runtime() };
    let Some(slot) = state.values.get_mut(index).and_then(Option::as_mut) else {
        crate::runtime::trap();
    };
    if slot.marked {
        return;
    }
    slot.marked = true;
    match &slot.value {
        RtValue::Error(error) => {
            worklist.push(error.data);
            if let Some(cause) = error.cause {
                worklist.push(cause);
            }
        }
        RtValue::List(list) => worklist.extend(list.elements.iter().copied()),
        RtValue::Iterator(iterator) => worklist.push(iterator.source()),
        RtValue::Object(object) => {
            worklist.extend(object.entries.iter().map(|(_, value)| *value));
            if let Some(enum_data) = &object.enum_data {
                worklist.extend(enum_data.fields.iter().copied());
            }
        }
        RtValue::Cell(cell) => worklist.push(cell.value),
        RtValue::Closure(closure) => worklist.extend(closure.captures.iter().copied()),
        RtValue::None
        | RtValue::Bool(_)
        | RtValue::Int(_)
        | RtValue::Float(_)
        | RtValue::String(_)
        | RtValue::Bytes(_)
        | RtValue::BoxedFutureValue(_) => {}
    }
}

/// Releases all unreachable table entries and records their reusable indices.
fn sweep() -> usize {
    let state = unsafe { runtime() };
    let mut live_values = 0;
    for (index, slot) in state.values.iter_mut().enumerate() {
        if slot.as_ref().is_some_and(|slot| !slot.marked) {
            *slot = None;
            let Ok(index) = u32::try_from(index) else {
                crate::runtime::trap();
            };
            state.free_slots.push(index);
        } else if slot.is_some() {
            live_values += 1;
        }
    }
    live_values
}

/// Converts one raw shadow-stack slot to its rooted ValueRef, or `None` for an empty slot.
fn raw_root(slot: u32) -> Option<ValueRef> {
    // SAFETY: generated code only stores ValueRef indices produced by this runtime, or 0.
    NonZeroU32::new(slot).map(|index| unsafe { ValueRef::from_runtime_index(index) })
}

/// Converts one one-based ValueRef index to its zero-based table position.
fn value_index(reference: ValueRef) -> usize {
    reference.runtime_index() as usize - 1
}
