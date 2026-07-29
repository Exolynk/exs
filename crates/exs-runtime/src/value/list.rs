//! Mutable runtime List payloads and operations.

use alloc::vec::Vec;

use exs_value::ValueRef;

/// A mutable ordered sequence of runtime value references.
pub(crate) struct RuntimeList {
    /// Elements in source-visible order.
    pub(crate) elements: Vec<ValueRef>,
}

impl RuntimeList {
    /// Creates an empty runtime list.
    pub(crate) const fn new() -> Self {
        Self {
            elements: Vec::new(),
        }
    }
}

pub(crate) mod operations {
    //! Wasm operations for mutable runtime Lists.

    use alloc::boxed::Box;

    use exs_value::{ValueRef, is_valid_int};

    use crate::runtime;
    use crate::value::{RtValue, RuntimeList};

    /// Allocates an empty mutable runtime List.
    pub(crate) fn new_value() -> ValueRef {
        runtime::allocate(RtValue::List(Box::new(RuntimeList::new())))
    }

    /// Appends one value and returns the new List length.
    pub(crate) fn append(receiver: ValueRef, item: ValueRef) -> ValueRef {
        let length = match runtime::value_mut(receiver) {
            RtValue::List(list) => {
                list.elements.push(item);
                list.elements.len()
            }
            _ => runtime::trap(),
        };
        length_value(length)
    }

    /// Reads one List element at a zero-based integer index.
    pub(crate) fn get(receiver: ValueRef, index: ValueRef) -> ValueRef {
        let index = list_index(index);
        match runtime::value(receiver) {
            RtValue::List(list) => match list.elements.get(index) {
                Some(value) => *value,
                None => runtime::trap(),
            },
            _ => runtime::trap(),
        }
    }

    /// Replaces one List element at a zero-based integer index.
    pub(crate) fn set(receiver: ValueRef, index: ValueRef, replacement: ValueRef) -> ValueRef {
        let index = list_index(index);
        match runtime::value_mut(receiver) {
            RtValue::List(list) => match list.elements.get_mut(index) {
                Some(value) => {
                    *value = replacement;
                    replacement
                }
                None => runtime::trap(),
            },
            _ => runtime::trap(),
        }
    }

    /// Creates a shallow List by appending a value or another List's elements.
    pub(crate) fn add(left: ValueRef, right: ValueRef) -> ValueRef {
        let elements = match runtime::value(left) {
            RtValue::List(list) => {
                let mut elements = list.elements.clone();
                match runtime::value(right) {
                    RtValue::List(right) => elements.extend_from_slice(&right.elements),
                    _ => elements.push(right),
                }
                elements
            }
            _ => runtime::trap(),
        };
        runtime::allocate(RtValue::List(Box::new(RuntimeList { elements })))
    }

    /// Removes and returns the final List value, or Null for an empty List.
    pub(crate) fn pop(receiver: ValueRef) -> ValueRef {
        let value = match runtime::value_mut(receiver) {
            RtValue::List(list) => list.elements.pop(),
            _ => runtime::trap(),
        };
        match value {
            Some(value) => value,
            None => runtime::allocate(RtValue::Null),
        }
    }

    /// Inserts one value into a List while preserving element order.
    pub(crate) fn insert(receiver: ValueRef, index: ValueRef, value: ValueRef) -> ValueRef {
        let index = list_index(index);
        match runtime::value_mut(receiver) {
            RtValue::List(list) if index <= list.elements.len() => {
                list.elements.insert(index, value);
            }
            RtValue::List(_) => runtime::trap(),
            _ => runtime::trap(),
        };
        runtime::allocate(RtValue::Null)
    }

    /// Removes and returns one List value at a zero-based index.
    pub(crate) fn remove(receiver: ValueRef, index: ValueRef) -> ValueRef {
        let index = list_index(index);
        match runtime::value_mut(receiver) {
            RtValue::List(list) if index < list.elements.len() => list.elements.remove(index),
            RtValue::List(_) => runtime::trap(),
            _ => runtime::trap(),
        }
    }

    /// Clears one List and returns Null.
    pub(crate) fn clear(receiver: ValueRef) -> ValueRef {
        match runtime::value_mut(receiver) {
            RtValue::List(list) => list.elements.clear(),
            _ => runtime::trap(),
        };
        runtime::allocate(RtValue::Null)
    }

    /// Reads one non-negative runtime integer as a List index.
    fn list_index(reference: ValueRef) -> usize {
        match runtime::value(reference) {
            RtValue::Int(index) if *index >= 0 => match usize::try_from(*index) {
                Ok(index) => index,
                Err(_) => runtime::trap(),
            },
            _ => runtime::trap(),
        }
    }

    /// Returns a checked ExS integer containing a collection length.
    /// Allocates a checked ExS integer containing one collection length.
    pub(crate) fn length_value(length: usize) -> ValueRef {
        let Ok(length) = i64::try_from(length) else {
            runtime::trap();
        };
        if !is_valid_int(length) {
            runtime::trap();
        }
        runtime::allocate(RtValue::Int(length))
    }

    /// Reads the sole value in one runtime-provided argument List.
    pub(crate) fn single_argument(arguments: ValueRef) -> ValueRef {
        match runtime::value(arguments) {
            RtValue::List(list) if list.elements.len() == 1 => list.elements[0],
            _ => runtime::trap(),
        }
    }

    /// Reads the two values in one runtime-provided argument List.
    pub(crate) fn two_arguments(arguments: ValueRef) -> (ValueRef, ValueRef) {
        match runtime::value(arguments) {
            RtValue::List(list) if list.elements.len() == 2 => (list.elements[0], list.elements[1]),
            _ => runtime::trap(),
        }
    }

    /// Verifies that one runtime-provided argument List is empty.
    pub(crate) fn require_no_arguments(arguments: ValueRef) {
        match runtime::value(arguments) {
            RtValue::List(list) if list.elements.is_empty() => {}
            _ => runtime::trap(),
        }
    }
}
