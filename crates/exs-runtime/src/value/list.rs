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
    use alloc::string::String;

    use exs_value::ValueRef;

    use crate::runtime;
    use crate::value::{RtValue, RuntimeList, operations::values_equal};

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

    /// Reads one List element or returns None when the index is outside its bounds.
    pub(crate) fn get_or_none(receiver: ValueRef, index: ValueRef) -> ValueRef {
        let index = match list_index(index) {
            Ok(index) => index,
            Err(error) => return error,
        };
        match runtime::value(receiver) {
            RtValue::List(list) => list
                .elements
                .get(index)
                .copied()
                .unwrap_or_else(|| runtime::allocate(RtValue::None)),
            _ => runtime::recoverable_error("TypeError", "get requires a List receiver", receiver),
        }
    }

    /// Returns the first List element or None when the List is empty.
    pub(crate) fn first(receiver: ValueRef) -> ValueRef {
        optional_end(receiver, false, "first")
    }

    /// Returns the final List element or None when the List is empty.
    pub(crate) fn last(receiver: ValueRef) -> ValueRef {
        optional_end(receiver, true, "last")
    }

    /// Returns a shallow List covering one validated half-open index range.
    pub(crate) fn slice(receiver: ValueRef, start: ValueRef, end: ValueRef) -> ValueRef {
        let start = match list_index(start) {
            Ok(index) => index,
            Err(error) => return error,
        };
        let end = match list_index(end) {
            Ok(index) => index,
            Err(error) => return error,
        };
        let elements = match runtime::value(receiver) {
            RtValue::List(list) if start <= end && end <= list.elements.len() => {
                list.elements[start..end].to_vec()
            }
            RtValue::List(_) => {
                return runtime::recoverable_error(
                    "IndexError",
                    "List slice range is outside the List bounds",
                    receiver,
                );
            }
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "slice requires a List receiver",
                    receiver,
                );
            }
        };
        runtime::allocate(RtValue::List(Box::new(RuntimeList { elements })))
    }

    /// Appends a shallow copy of another List and returns the new receiver length.
    pub(crate) fn extend(receiver: ValueRef, other: ValueRef) -> ValueRef {
        let additions = match runtime::value(other) {
            RtValue::List(list) => list.elements.clone(),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "extend requires a List argument",
                    other,
                );
            }
        };
        let length = match runtime::value_mut(receiver) {
            RtValue::List(list) => {
                list.elements.extend(additions);
                list.elements.len()
            }
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "extend requires a List receiver",
                    receiver,
                );
            }
        };
        length_value(length)
    }

    /// Reverses one List in place and returns None.
    pub(crate) fn reverse(receiver: ValueRef) -> ValueRef {
        match runtime::value_mut(receiver) {
            RtValue::List(list) => list.elements.reverse(),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "reverse requires a List receiver",
                    receiver,
                );
            }
        }
        runtime::allocate(RtValue::None)
    }

    /// Returns a new shallow List in reverse element order.
    pub(crate) fn reversed(receiver: ValueRef) -> ValueRef {
        let mut elements = match runtime::value(receiver) {
            RtValue::List(list) => list.elements.clone(),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "reversed requires a List receiver",
                    receiver,
                );
            }
        };
        elements.reverse();
        runtime::allocate(RtValue::List(Box::new(RuntimeList { elements })))
    }

    /// Returns whether a List contains one value under ExS equality semantics.
    pub(crate) fn contains(receiver: ValueRef, item: ValueRef) -> ValueRef {
        let contains = match runtime::value(receiver) {
            RtValue::List(list) => list.elements.iter().any(|value| values_equal(*value, item)),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "contains requires a List receiver",
                    receiver,
                );
            }
        };
        runtime::allocate(RtValue::Bool(contains))
    }

    /// Returns the first index of a matching List value, or None when it is absent.
    pub(crate) fn index_of(receiver: ValueRef, item: ValueRef) -> ValueRef {
        matching_index(receiver, item, false, "index_of")
    }

    /// Returns the final index of a matching List value, or None when it is absent.
    pub(crate) fn last_index_of(receiver: ValueRef, item: ValueRef) -> ValueRef {
        matching_index(receiver, item, true, "last_index_of")
    }

    /// Joins String List entries with a String separator.
    pub(crate) fn join(receiver: ValueRef, separator: ValueRef) -> ValueRef {
        let separator = match runtime::value(separator) {
            RtValue::String(value) => String::from(value.as_str()),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "join requires a String separator",
                    separator,
                );
            }
        };
        let elements = match runtime::value(receiver) {
            RtValue::List(list) => list.elements.clone(),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "join requires a List receiver",
                    receiver,
                );
            }
        };
        let mut result = String::new();
        for (index, value) in elements.iter().enumerate() {
            let RtValue::String(value) = runtime::value(*value) else {
                return runtime::recoverable_error(
                    "TypeError",
                    "join requires every List item to be a String",
                    *value,
                );
            };
            if index > 0 {
                result.push_str(&separator);
            }
            result.push_str(value.as_str());
        }
        runtime::allocate(RtValue::String(Box::new(
            crate::value::RuntimeString::from_string(result),
        )))
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

    /// Returns an optional List endpoint without exposing an index error for emptiness.
    fn optional_end(receiver: ValueRef, final_item: bool, name: &str) -> ValueRef {
        match runtime::value(receiver) {
            RtValue::List(list) => {
                let value = if final_item {
                    list.elements.last()
                } else {
                    list.elements.first()
                };
                value
                    .copied()
                    .unwrap_or_else(|| runtime::allocate(RtValue::None))
            }
            _ => runtime::recoverable_error(
                "TypeError",
                &alloc::format!("{name} requires a List receiver"),
                receiver,
            ),
        }
    }

    /// Returns an optional matching List index using ExS equality semantics.
    fn matching_index(receiver: ValueRef, item: ValueRef, reverse: bool, name: &str) -> ValueRef {
        let index = match runtime::value(receiver) {
            RtValue::List(list) if reverse => list
                .elements
                .iter()
                .rposition(|value| values_equal(*value, item)),
            RtValue::List(list) => list
                .elements
                .iter()
                .position(|value| values_equal(*value, item)),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    &alloc::format!("{name} requires a List receiver"),
                    receiver,
                );
            }
        };
        match index {
            Some(index) => length_value(index),
            None => runtime::allocate(RtValue::None),
        }
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

    /// Allocates one signed 64-bit ExS integer containing one collection length.
    pub(crate) fn length_value(length: usize) -> ValueRef {
        let Ok(length) = i64::try_from(length) else {
            runtime::trap();
        };
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
