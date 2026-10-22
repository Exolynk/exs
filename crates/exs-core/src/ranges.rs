//! Native steps of prelude `Range` objects in `for` loops.
//!
//! A `for` loop over a Range object calls the prelude's `Range::next`. For a Range whose
//! fields are immediate Ints and Bools, the core takes the same step on the object's slots
//! instead; any other Range, and a step that would overflow or leave the immediate range,
//! falls back to `Range::next`, which then reports what the prelude reports.

use exs_layout::ValueRef;
use exs_layout::value::layout::{EXTRA_SHIFT, SMALL_INT_TAG};
use exs_layout::{RANGE_BEGIN_FALLBACK, RANGE_STATE_FINISHED, RANGE_STATE_LAST};

use crate::memory::Memory;
use crate::typed::{slot_address, slot_count, type_id, typed_object};

/// Result of a step that must call `Range::next`; no `ValueRef` has all bits set.
pub const RANGE_FALLBACK: u64 = u64::MAX;
/// Result of a step of an exhausted Range.
pub const RANGE_DONE: u64 = 0;

/// Slot of the prelude Range's `current` field, in declaration order.
const CURRENT: u32 = 0;
/// Slot of the `end` field.
const END: u32 = 1;
/// Slot of the `step` field.
const STEP: u32 = 2;
/// Slot of the `inclusive` field.
const INCLUSIVE: u32 = 3;
/// Slot of the `done` field.
const DONE: u32 = 4;

/// Advances the Range `raw` of type `range_type` like `Range::next` and returns the item,
/// [`RANGE_DONE`] once exhausted, or [`RANGE_FALLBACK`] without changing anything.
pub fn range_step(memory: &mut impl Memory, raw: u64, range_type: u32) -> u64 {
    step(memory, raw, range_type).unwrap_or(RANGE_FALLBACK)
}

/// Takes one native step, or returns `None` when `Range::next` must take it.
#[inline(always)]
fn step(memory: &mut impl Memory, raw: u64, range_type: u32) -> Option<u64> {
    let object = range_object(memory, raw, range_type)?;
    let item = load(memory, object, CURRENT);
    let (current, end, step) = (
        small_int(item)?,
        small_int(load(memory, object, END))?,
        small_int(load(memory, object, STEP))?,
    );
    let (inclusive, done) = (
        flag(load(memory, object, INCLUSIVE))?,
        flag(load(memory, object, DONE))?,
    );
    if done {
        return Some(RANGE_DONE);
    }
    let exhausted = exhausted(current, end, step, inclusive);
    if exhausted || current == end {
        memory.store_u64(slot_address(object, DONE), ValueRef::TRUE.raw());
        return Some(if exhausted { RANGE_DONE } else { item });
    }
    let next = ValueRef::small_int(current.checked_add(step)?)?;
    memory.store_u64(slot_address(object, CURRENT), next.raw());
    Some(item)
}

/// Starts counting an unobserved Range variable in a `for` loop.
///
/// For a well-formed unit-step Range of immediate fields that is neither done nor exhausted,
/// returns the current value in the low and the last item in the high 32 bits, both as `i32`.
/// Every other Range, including one whose current value is `i32::MIN`, returns
/// [`RANGE_BEGIN_FALLBACK`] and is iterated as an Object. Never changes the Range.
pub fn range_begin(memory: &impl Memory, raw: u64, range_type: u32) -> u64 {
    begin(memory, raw, range_type).unwrap_or(RANGE_BEGIN_FALLBACK.cast_unsigned())
}

/// Returns the packed bounds of [`range_begin`], or `None` for the fallback.
fn begin(memory: &impl Memory, raw: u64, range_type: u32) -> Option<u64> {
    let object = range_object(memory, raw, range_type)?;
    let current = small_int(load(memory, object, CURRENT))?;
    let end = small_int(load(memory, object, END))?;
    let step = small_int(load(memory, object, STEP))?;
    let inclusive = flag(load(memory, object, INCLUSIVE))?;
    if flag(load(memory, object, DONE))?
        || step.unsigned_abs() != 1
        || current == i64::from(i32::MIN)
        || exhausted(current, end, step, inclusive)
    {
        return None;
    }
    // Not being exhausted, the Range moves toward a last item between current and end.
    let last = i32::try_from(if inclusive { end } else { end - step }).ok()?;
    let current = i32::try_from(current).ok()?;
    Some(u64::from(current.cast_unsigned()) | u64::from(last.cast_unsigned()) << 32)
}

/// Writes the state of a counted Range variable back once its loop exits.
///
/// `cursor` and `state` are the loop's counter and progress: 0 before the last item,
/// [`RANGE_STATE_LAST`] right after it and [`RANGE_STATE_FINISHED`] once the loop asked for
/// another item. The fields then match what `Range::next` would have left.
pub fn range_finish(memory: &mut impl Memory, raw: u64, range_type: u32, cursor: i64, state: i64) {
    let Some(object) = range_object(memory, raw, range_type) else {
        return;
    };
    let (Some(inclusive), Some(end), Some(step)) = (
        flag(load(memory, object, INCLUSIVE)),
        small_int(load(memory, object, END)),
        small_int(load(memory, object, STEP)),
    ) else {
        return;
    };
    // An exclusive Range moves past its last item onto the end and finishes one call later.
    let (cursor, finished) = match state {
        RANGE_STATE_LAST | RANGE_STATE_FINISHED if inclusive => (cursor, true),
        RANGE_STATE_LAST => (cursor + step, false),
        RANGE_STATE_FINISHED => (end, true),
        _ => (cursor, false),
    };
    if let Some(cursor) = ValueRef::small_int(cursor) {
        memory.store_u64(slot_address(object, CURRENT), cursor.raw());
    }
    if finished {
        memory.store_u64(slot_address(object, DONE), ValueRef::TRUE.raw());
    }
}

/// Returns the address of the prelude Range `raw` of type `range_type` with all its slots.
#[inline(always)]
fn range_object(memory: &impl Memory, raw: u64, range_type: u32) -> Option<u32> {
    let object = typed_object(memory, raw)?;
    (type_id(memory, object) == range_type && slot_count(memory, object) > DONE).then_some(object)
}

/// Returns whether a Range at `current` has no further item, like `Range::next`.
#[inline(always)]
const fn exhausted(current: i64, end: i64, step: i64, inclusive: bool) -> bool {
    match (step > 0, inclusive) {
        (true, true) => current > end,
        (true, false) => current >= end,
        (false, true) => current < end,
        (false, false) => current <= end,
    }
}

/// Returns the value of an immediate Int, or `None` for any other value.
#[inline(always)]
fn small_int(raw: u64) -> Option<i64> {
    ((raw >> EXTRA_SHIFT) as u32 == SMALL_INT_TAG).then_some(i64::from(raw as u32 as i32))
}

/// Returns the value of an immediate Bool, or `None` for any other value.
#[inline(always)]
fn flag(raw: u64) -> Option<bool> {
    if raw == ValueRef::TRUE.raw() {
        Some(true)
    } else if raw == ValueRef::FALSE.raw() {
        Some(false)
    } else {
        None
    }
}

/// Loads slot `slot` of the typed Object at `object`.
fn load(memory: &impl Memory, object: u32, slot: u32) -> u64 {
    memory.load_u64(slot_address(object, slot))
}
