//! Results that cannot cross the runner boundary, reported like v1 as a fatal
//! SerializationError.
//!
//! The encoder only reports that a result cannot be encoded. Like v1, which converts the result
//! depth first, a second walk finds whether the first obstacle is a reference cycle or a value
//! of a type the boundary lacks, such as a closure.

use exs_value::ValueRef;

use crate::closures::{cell_object, closure_object};
use crate::enums::{enum_object, field_address as enum_field, field_count};
use crate::errors::{error_object, error_parts, new_error};
use crate::lists::{item_address, len, list_object};
use crate::memory::Memory;
use crate::objects::{shape, untyped_object, value_address};
use crate::shapes::key_count;
use crate::typed::{extra, slot_address, slot_count, typed_object};

/// Deepest nesting the walk follows, like the encoder.
const MAX_DEPTH: usize = 128;

/// Why a result cannot cross the runner boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Obstacle {
    /// The value graph contains a reference cycle.
    Cycle,
    /// A value has a type the boundary does not carry.
    UnsupportedValue,
}

/// Creates v1's SerializationError, fatal for a result and recoverable for host-call
/// arguments, for the value `raw` the encoder rejected.
pub(crate) fn serialization_error(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
    fatal: bool,
) -> Option<u64> {
    let mut path = [0_u32; MAX_DEPTH];
    let message = match obstacle(memory, raw, &mut path, 0) {
        Some(Obstacle::Cycle) => {
            "value graph contains a cycle and cannot cross the runner boundary"
        }
        Some(Obstacle::UnsupportedValue) => "value type cannot cross the runner boundary",
        None => "value cannot be encoded for the runner boundary",
    };
    let error = new_error(
        memory,
        vm,
        fatal,
        "SerializationError",
        message,
        ValueRef::NONE.raw(),
    )?;
    // Like v1, only the Error of a result has no origin and trace.
    if fatal {
        crate::errors::clear_origin_and_trace(memory, error as u32);
    }
    Some(error)
}

/// Returns the first obstacle of `raw` in depth-first order, with `path` holding the
/// containers being visited.
fn obstacle(
    memory: &impl Memory,
    raw: u64,
    path: &mut [u32; MAX_DEPTH],
    depth: usize,
) -> Option<Obstacle> {
    if closure_object(raw).is_some() || cell_object(raw).is_some() {
        return Some(Obstacle::UnsupportedValue);
    }
    let container = Container::of(memory, raw)?;
    let address = container.address();
    if path[..depth].contains(&address) {
        return Some(Obstacle::Cycle);
    }
    if depth >= MAX_DEPTH {
        return None;
    }
    path[depth] = address;
    (0..container.count(memory))
        .filter_map(|index| container.child(memory, index))
        .find_map(|child| obstacle(memory, child, path, depth + 1))
}

/// A value with child values, in the encoder's order.
#[derive(Clone, Copy)]
enum Container {
    /// A List at this address.
    List(u32),
    /// A typed Object at this address, its declared slots then its extra keys.
    Typed(u32),
    /// An Error at this address: its data, then its cause.
    Error(u32),
    /// An enum at this address.
    Enum(u32),
    /// An untyped Object at this address.
    Untyped(u32),
}

impl Container {
    /// Classifies `raw`, or returns `None` for values without children.
    fn of(memory: &impl Memory, raw: u64) -> Option<Self> {
        list_object(raw)
            .map(Self::List)
            .or_else(|| typed_object(memory, raw).map(Self::Typed))
            .or_else(|| error_object(raw).map(Self::Error))
            .or_else(|| enum_object(memory, raw).map(Self::Enum))
            .or_else(|| untyped_object(memory, raw).map(Self::Untyped))
    }

    /// Returns the container's address.
    const fn address(self) -> u32 {
        match self {
            Self::List(address)
            | Self::Typed(address)
            | Self::Error(address)
            | Self::Enum(address)
            | Self::Untyped(address) => address,
        }
    }

    /// Returns the number of child positions.
    fn count(self, memory: &impl Memory) -> u32 {
        match self {
            Self::List(list) => len(memory, list),
            Self::Typed(object) => {
                slot_count(memory, object)
                    + match extra(memory, object) {
                        0 => 0,
                        keys => key_count(memory, shape(memory, keys)),
                    }
            }
            Self::Error(_) => 2,
            Self::Enum(object) => field_count(memory, object),
            Self::Untyped(object) => key_count(memory, shape(memory, object)),
        }
    }

    /// Returns the child value at position `index`, or `None` for a removed typed field.
    fn child(self, memory: &impl Memory, index: u32) -> Option<u64> {
        match self {
            Self::List(list) => Some(memory.load_u64(item_address(memory, list, index))),
            Self::Typed(object) => {
                let slots = slot_count(memory, object);
                if index < slots {
                    Some(memory.load_u64(slot_address(object, index))).filter(|value| *value != 0)
                } else {
                    let keys = extra(memory, object);
                    Some(memory.load_u64(value_address(memory, keys, index - slots)))
                }
            }
            Self::Error(object) => {
                let parts = error_parts(memory, object);
                Some(if index == 0 { parts.data } else { parts.cause })
            }
            Self::Enum(object) => Some(memory.load_u64(enum_field(object, index))),
            Self::Untyped(object) => Some(memory.load_u64(value_address(memory, object, index))),
        }
    }
}
