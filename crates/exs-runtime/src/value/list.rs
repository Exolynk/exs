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
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "push requires a List receiver",
                    receiver,
                );
            }
        };
        length_value(length)
    }

    /// Reads one List element at a zero-based integer index.
    pub(crate) fn get(receiver: ValueRef, index: ValueRef) -> ValueRef {
        let index_reference = index;
        let index = match list_index(index) {
            Ok(index) => index,
            Err(error) => return error,
        };
        match runtime::value(receiver) {
            RtValue::List(list) => match list.elements.get(index) {
                Some(value) => *value,
                None => runtime::recoverable_error(
                    "IndexError",
                    "List index is outside the List bounds",
                    index_reference,
                ),
            },
            _ => runtime::recoverable_error(
                "TypeError",
                "index access requires a List receiver",
                receiver,
            ),
        }
    }

    /// Replaces one List element at a zero-based integer index.
    pub(crate) fn set(receiver: ValueRef, index: ValueRef, replacement: ValueRef) -> ValueRef {
        let index_reference = index;
        let index = match list_index(index) {
            Ok(index) => index,
            Err(error) => return error,
        };
        match runtime::value_mut(receiver) {
            RtValue::List(list) => match list.elements.get_mut(index) {
                Some(value) => {
                    *value = replacement;
                    replacement
                }
                None => runtime::recoverable_error(
                    "IndexError",
                    "List index is outside the List bounds",
                    index_reference,
                ),
            },
            _ => runtime::recoverable_error(
                "TypeError",
                "index assignment requires a List receiver",
                receiver,
            ),
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
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "List addition requires a List receiver",
                    left,
                );
            }
        };
        runtime::allocate(RtValue::List(Box::new(RuntimeList { elements })))
    }

    /// Removes and returns the final List value, or None for an empty List.
    pub(crate) fn pop(receiver: ValueRef) -> ValueRef {
        let value = match runtime::value_mut(receiver) {
            RtValue::List(list) => list.elements.pop(),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "pop requires a List receiver",
                    receiver,
                );
            }
        };
        match value {
            Some(value) => value,
            None => runtime::allocate(RtValue::None),
        }
    }

    /// Inserts one value into a List while preserving element order.
    pub(crate) fn insert(receiver: ValueRef, index: ValueRef, value: ValueRef) -> ValueRef {
        let index_reference = index;
        let index = match list_index(index) {
            Ok(index) => index,
            Err(error) => return error,
        };
        match runtime::value_mut(receiver) {
            RtValue::List(list) if index <= list.elements.len() => {
                list.elements.insert(index, value);
            }
            RtValue::List(_) => {
                return runtime::recoverable_error(
                    "IndexError",
                    "List insertion index is outside the List bounds",
                    index_reference,
                );
            }
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "insert requires a List receiver",
                    receiver,
                );
            }
        };
        runtime::allocate(RtValue::None)
    }

    /// Removes and returns one List value at a zero-based index.
    pub(crate) fn remove(receiver: ValueRef, index: ValueRef) -> ValueRef {
        let index_reference = index;
        let index = match list_index(index) {
            Ok(index) => index,
            Err(error) => return error,
        };
        match runtime::value_mut(receiver) {
            RtValue::List(list) if index < list.elements.len() => list.elements.remove(index),
            RtValue::List(_) => runtime::recoverable_error(
                "IndexError",
                "List index is outside the List bounds",
                index_reference,
            ),
            _ => {
                runtime::recoverable_error("TypeError", "remove requires a List receiver", receiver)
            }
        }
    }

    /// Clears one List and returns None.
    pub(crate) fn clear(receiver: ValueRef) -> ValueRef {
        match runtime::value_mut(receiver) {
            RtValue::List(list) => list.elements.clear(),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "clear requires a List receiver",
                    receiver,
                );
            }
        };
        runtime::allocate(RtValue::None)
    }

    /// Reads one non-negative runtime integer as a List index.
    fn list_index(reference: ValueRef) -> Result<usize, ValueRef> {
        match runtime::value(reference) {
            RtValue::Int(index) if *index >= 0 => match usize::try_from(*index) {
                Ok(index) => Ok(index),
                Err(_) => Err(runtime::recoverable_error(
                    "IndexError",
                    "List index is outside the supported range",
                    reference,
                )),
            },
            _ => Err(runtime::recoverable_error(
                "IndexError",
                "List index requires a non-negative Int value",
                reference,
            )),
        }
    }

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
    pub(crate) fn single_argument(arguments: ValueRef) -> Result<ValueRef, ValueRef> {
        match runtime::value(arguments) {
            RtValue::List(list) if list.elements.len() == 1 => Ok(list.elements[0]),
            _ => Err(runtime::recoverable_error(
                "ArityError",
                "method expects exactly one argument",
                arguments,
            )),
        }
    }

    /// Reads the two values in one runtime-provided argument List.
    pub(crate) fn two_arguments(arguments: ValueRef) -> Result<(ValueRef, ValueRef), ValueRef> {
        match runtime::value(arguments) {
            RtValue::List(list) if list.elements.len() == 2 => {
                Ok((list.elements[0], list.elements[1]))
            }
            _ => Err(runtime::recoverable_error(
                "ArityError",
                "method expects exactly two arguments",
                arguments,
            )),
        }
    }

    /// Verifies that one runtime-provided argument List is empty.
    pub(crate) fn require_no_arguments(arguments: ValueRef) -> Result<(), ValueRef> {
        match runtime::value(arguments) {
            RtValue::List(list) if list.elements.is_empty() => Ok(()),
            _ => Err(runtime::recoverable_error(
                "ArityError",
                "method expects no arguments",
                arguments,
            )),
        }
    }
}
