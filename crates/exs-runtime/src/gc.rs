//! Stop-the-world mark-and-sweep collection and compiler root-frame management.

use alloc::vec::Vec;

use exs_value::ValueRef;

use crate::state::{RootFrame, runtime};
use crate::value::RtValue;

/// Creates one compiler-generated root frame and returns its stack position token.
pub(crate) fn push_root_frame(slot_count: i32) -> i32 {
    let Ok(slot_count) = usize::try_from(slot_count) else {
        crate::runtime::trap();
    };
    let state = unsafe { runtime() };
    let Ok(frame) = i32::try_from(state.root_frames.len()) else {
        crate::runtime::trap();
    };
    let mut slots = Vec::new();
    slots.resize(slot_count, None);
    state.root_frames.push(RootFrame { slots });
    frame
}

/// Stores one compiler-local value reference in the current root frame.
pub(crate) fn set_root_frame_slot(frame: i32, slot: i32, value: ValueRef) {
    let frame = frame_index(frame);
    let slot = slot_index(slot);
    let state = unsafe { runtime() };
    let Some(current_frame) = state.root_frames.len().checked_sub(1) else {
        crate::runtime::trap();
    };
    if frame != current_frame {
        crate::runtime::trap();
    }
    let Some(slot) = state.root_frames[frame].slots.get_mut(slot) else {
        crate::runtime::trap();
    };
    *slot = Some(value);
}

/// Clears one compiler-local root after its value is no longer live.
pub(crate) fn clear_root_frame_slot(frame: i32, slot: i32) {
    let frame = frame_index(frame);
    let slot = slot_index(slot);
    let state = unsafe { runtime() };
    let Some(current_frame) = state.root_frames.len().checked_sub(1) else {
        crate::runtime::trap();
    };
    if frame != current_frame {
        crate::runtime::trap();
    }
    let Some(slot) = state.root_frames[frame].slots.get_mut(slot) else {
        crate::runtime::trap();
    };
    *slot = None;
}

/// Removes the current compiler-generated root frame.
pub(crate) fn pop_root_frame(frame: i32) {
    let frame = frame_index(frame);
    let state = unsafe { runtime() };
    let Some(current_frame) = state.root_frames.len().checked_sub(1) else {
        crate::runtime::trap();
    };
    if frame != current_frame {
        crate::runtime::trap();
    }
    let _removed = state.root_frames.pop();
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
    sweep();
}

/// Copies the current root set after clearing the previous collection's marks.
fn collect_roots() -> Vec<ValueRef> {
    let state = unsafe { runtime() };
    for slot in state.values.iter_mut().flatten() {
        slot.marked = false;
    }
    let mut roots = Vec::new();
    for frame in &state.root_frames {
        roots.extend(frame.slots.iter().flatten().copied());
    }
    roots.extend(state.temporary_roots.iter().copied());
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
        RtValue::Object(object) => worklist.extend(object.entries.iter().map(|(_, value)| *value)),
        RtValue::None
        | RtValue::Bool(_)
        | RtValue::Int(_)
        | RtValue::Float(_)
        | RtValue::String(_)
        | RtValue::BoxedFutureValue(_) => {}
    }
}

/// Releases all unreachable table entries and records their reusable indices.
fn sweep() {
    let state = unsafe { runtime() };
    for (index, slot) in state.values.iter_mut().enumerate() {
        if slot.as_ref().is_some_and(|slot| !slot.marked) {
            *slot = None;
            let Ok(index) = u32::try_from(index) else {
                crate::runtime::trap();
            };
            state.free_slots.push(index);
        }
    }
}

/// Converts one Wasm root-frame token to a Rust index.
fn frame_index(frame: i32) -> usize {
    match usize::try_from(frame) {
        Ok(frame) => frame,
        Err(_) => crate::runtime::trap(),
    }
}

/// Converts one Wasm root-frame slot token to a Rust index.
fn slot_index(slot: i32) -> usize {
    match usize::try_from(slot) {
        Ok(slot) => slot,
        Err(_) => crate::runtime::trap(),
    }
}

/// Converts one one-based ValueRef index to its zero-based table position.
fn value_index(reference: ValueRef) -> usize {
    reference.runtime_index() as usize - 1
}
