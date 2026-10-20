//! Stop-the-world mark-and-sweep collection and compiler root-frame management.

use alloc::vec::Vec;
use exs_value::ValueRef;

use crate::state::{registers, runtime};
use crate::value::RtValue;
use crate::value::reference;

/// Minimum value-table size before automatic collection becomes worthwhile.
///
/// Typical short scripts stay below it and never collect.
pub(crate) const INITIAL_COLLECTION_THRESHOLD: usize = 4096;

/// Factor between the live values after a collection and the table size of the next one.
const COLLECTION_GROWTH_FACTOR: usize = 3;

/// Minimum slot count of one shadow root stack chunk.
const ROOT_CHUNK_SLOTS: usize = 4096;

/// Byte size of one root-stack slot holding a raw `ValueRef`.
const ROOT_SLOT_SIZE: usize = 8;

/// Creates one zeroed compiler-generated root frame and returns its first slot's address.
///
/// Generated code roots a value with a plain `i64.store` of its raw form at `base + slot * 8`
/// and clears it by storing 0, so the slot count must cover every slot the compiler emits.
/// Generated functions push frames inline and only call this when a frame needs a new chunk.
pub(crate) fn push_root_frame(slot_count: i32) -> i32 {
    let Ok(len) = usize::try_from(slot_count) else {
        crate::runtime::trap();
    };
    let Some(bytes) = len
        .checked_mul(ROOT_SLOT_SIZE)
        .and_then(|bytes| u32::try_from(bytes).ok())
    else {
        crate::runtime::trap();
    };
    let registers = unsafe { registers() };
    let fits = registers.root_limit != 0
        && registers
            .root_top
            .checked_add(bytes)
            .is_some_and(|end| end <= registers.root_limit);
    let base = if fits {
        let base = registers.root_top;
        registers.root_top = base + bytes;
        base
    } else {
        enter_next_chunk(len, bytes)
    };
    // SAFETY: `base..base + bytes` lies inside the current chunk, which this runtime owns.
    unsafe { core::ptr::write_bytes(base as usize as *mut u64, 0, len) };
    let Ok(base) = i32::try_from(base) else {
        crate::runtime::trap();
    };
    base
}

/// Moves the root stack to the next chunk, sized for at least `len` slots, and returns the
/// new frame's base address.
fn enter_next_chunk(len: usize, bytes: u32) -> u32 {
    let registers = unsafe { registers() };
    let stack = &mut unsafe { runtime() }.root_stack;
    let next = match stack.current {
        Some(current) => {
            stack.saved_tops.push(registers.root_top);
            current + 1
        }
        None => 0,
    };
    // Chunks after the active top frame hold no live frames, so an undersized one may be replaced.
    if stack.chunks.get(next).is_none_or(|slots| len > slots.len()) {
        let slots = alloc::vec![0; len.max(ROOT_CHUNK_SLOTS)].into_boxed_slice();
        if next < stack.chunks.len() {
            stack.chunks[next] = slots;
        } else {
            stack.chunks.push(slots);
        }
    }
    stack.current = Some(next);
    let (floor, limit) = chunk_bounds(&stack.chunks[next]);
    registers.root_floor = floor;
    registers.root_limit = limit;
    registers.root_top = floor + bytes;
    floor
}

/// Removes the innermost root frame identified by its base address.
///
/// Generated functions pop inline and only call this for the first frame of a chunk.
pub(crate) fn pop_root_frame(base: i32) {
    let Ok(base) = u32::try_from(base) else {
        crate::runtime::trap();
    };
    let registers = unsafe { registers() };
    if base < registers.root_floor || base > registers.root_top {
        crate::runtime::trap();
    }
    let stack = &mut unsafe { runtime() }.root_stack;
    match stack.current {
        Some(current) if base == registers.root_floor && current > 0 => {
            let Some(top) = stack.saved_tops.pop() else {
                crate::runtime::trap();
            };
            let previous = current - 1;
            stack.current = Some(previous);
            let (floor, limit) = chunk_bounds(&stack.chunks[previous]);
            registers.root_floor = floor;
            registers.root_limit = limit;
            registers.root_top = top;
        }
        _ => registers.root_top = base,
    }
}

/// Returns the start and end addresses of one root-stack chunk.
fn chunk_bounds(slots: &[u64]) -> (u32, u32) {
    let start = slots.as_ptr() as usize;
    let Some(end) = slots
        .len()
        .checked_mul(ROOT_SLOT_SIZE)
        .and_then(|bytes| start.checked_add(bytes))
    else {
        crate::runtime::trap();
    };
    match (u32::try_from(start), u32::try_from(end)) {
        (Ok(start), Ok(end)) => (start, end),
        _ => crate::runtime::trap(),
    }
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
    // The worklist keeps its capacity between collections; it starts with the root set.
    let mut worklist = core::mem::take(&mut unsafe { runtime() }.gc_worklist);
    worklist.clear();
    collect_roots(&mut worklist);
    while let Some(reference) = worklist.pop() {
        mark(reference, &mut worklist);
    }
    let live_values = sweep();
    let state = unsafe { runtime() };
    state.gc_worklist = worklist;
    state.next_gc_at = live_values
        .saturating_mul(COLLECTION_GROWTH_FACTOR)
        .max(INITIAL_COLLECTION_THRESHOLD);
}

/// Appends the current root set to the collection worklist.
///
/// Marks need no clearing pass here because the previous sweep reset them.
fn collect_roots(roots: &mut Vec<ValueRef>) {
    let state = unsafe { runtime() };
    if let Some(current) = state.root_stack.current {
        let top = unsafe { registers() }.root_top;
        for (index, slots) in state.root_stack.chunks[..=current].iter().enumerate() {
            let end = if index == current {
                top
            } else {
                state.root_stack.saved_tops[index]
            };
            let (floor, _) = chunk_bounds(slots);
            let used = (end.saturating_sub(floor) as usize) / ROOT_SLOT_SIZE;
            roots.extend(slots[..used].iter().copied().filter_map(raw_root));
        }
    }
    for frame in state.async_frames.iter().flatten() {
        roots.extend(frame.slots().iter().copied().filter_map(raw_root));
    }
    roots.extend(state.completed_async_result);
    if let Some(execution) = &state.execution {
        roots.extend(execution.roots());
    }
    roots.extend(state.temporary_roots.iter().copied());
    roots.extend(state.pinned_literals.iter().copied());
}

/// Marks one reachable slot and queues its owned child references.
fn mark(reference: ValueRef, worklist: &mut Vec<ValueRef>) {
    // Immediates own no heap slot and reference no children.
    let Some(index) = reference::heap_index(reference) else {
        return;
    };
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
        RtValue::Iterator(iterator) => worklist.extend(iterator.source()),
        RtValue::Object(object) => {
            worklist.extend(object.entries.iter().map(|(_, value)| *value));
            if let Some(enum_data) = &object.enum_data {
                worklist.extend(enum_data.fields.iter().copied());
            }
        }
        RtValue::Cell(cell) => worklist.push(cell.value),
        RtValue::Closure(closure) => worklist.extend(closure.captures.iter().copied()),
        RtValue::Int(_)
        | RtValue::Float(_)
        | RtValue::String(_)
        | RtValue::Bytes(_)
        | RtValue::FormatBuilder(_)
        | RtValue::BoxedFutureValue(_) => {}
    }
}

/// Releases all unreachable table entries, records their reusable indices and resets marks.
fn sweep() -> usize {
    let state = unsafe { runtime() };
    let mut live_values = 0;
    for (index, entry) in state.values.iter_mut().enumerate() {
        let Some(slot) = entry.as_mut() else {
            continue;
        };
        if slot.marked {
            // Resetting here lets the next collection start without a separate clearing pass.
            slot.marked = false;
            live_values += 1;
        } else {
            *entry = None;
            let Ok(index) = u32::try_from(index) else {
                crate::runtime::trap();
            };
            state.free_slots.push(index);
        }
    }
    live_values
}

/// Converts one raw shadow-stack slot to its rooted ValueRef, or `None` for an empty slot.
fn raw_root(slot: u64) -> Option<ValueRef> {
    // SAFETY: generated code only stores ValueRefs produced by this runtime, or 0.
    unsafe { ValueRef::from_raw(slot) }
}
