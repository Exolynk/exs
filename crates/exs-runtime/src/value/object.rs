//! Mutable insertion-ordered runtime Object payloads.

use alloc::boxed::Box;
use alloc::vec::Vec;

use exs_value::ValueRef;

/// A mutable insertion-ordered mapping from string keys to runtime values.
pub(crate) struct RuntimeObject {
    /// Key-value entries in insertion order.
    pub(crate) entries: Vec<(Box<str>, ValueRef)>,
}

impl RuntimeObject {
    /// Creates an empty runtime Object.
    pub(crate) const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

pub(crate) mod operations {
    //! Wasm operations for mutable runtime Objects.

    use alloc::boxed::Box;
    use alloc::string::String;
    use alloc::vec::Vec;

    use exs_value::ValueRef;

    use crate::gc;
    use crate::runtime;
    use crate::value::{RtValue, RuntimeList, RuntimeObject, RuntimeString, operations};

    /// Allocates an empty mutable runtime Object.
    pub(crate) fn new_value() -> ValueRef {
        runtime::allocate(RtValue::Object(Box::new(RuntimeObject::new())))
    }

    /// Reads one Object property or returns Null when it is absent.
    pub(crate) fn get(receiver: ValueRef, index: ValueRef) -> ValueRef {
        let key = match operations::string_value(index) {
            Ok(key) => key,
            Err(error) => return error,
        };
        let result = match runtime::value(receiver) {
            RtValue::Object(object) => object
                .entries
                .iter()
                .find_map(|(entry_key, value)| (entry_key.as_ref() == key).then_some(*value)),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "index access requires an Object receiver",
                    receiver,
                );
            }
        };
        match result {
            Some(value) => value,
            None => runtime::allocate(RtValue::None),
        }
    }

    /// Creates or replaces one Object property.
    pub(crate) fn set(receiver: ValueRef, index: ValueRef, replacement: ValueRef) -> ValueRef {
        let key = match operations::string_value(index) {
            Ok(key) => key,
            Err(error) => return error,
        };
        match runtime::value_mut(receiver) {
            RtValue::Object(object) => {
                if let Some((_, value)) = object
                    .entries
                    .iter_mut()
                    .find(|(entry_key, _)| entry_key.as_ref() == key)
                {
                    *value = replacement;
                } else {
                    object.entries.push((key.into_boxed_str(), replacement));
                }
                replacement
            }
            _ => runtime::recoverable_error(
                "TypeError",
                "index assignment requires an Object receiver",
                receiver,
            ),
        }
    }

    /// Tests whether an Object contains one string key.
    pub(crate) fn has(receiver: ValueRef, key: ValueRef) -> ValueRef {
        let key = match operations::string_value(key) {
            Ok(key) => key,
            Err(error) => return error,
        };
        let contains = match runtime::value(receiver) {
            RtValue::Object(object) => object
                .entries
                .iter()
                .any(|(entry_key, _)| entry_key.as_ref() == key),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "has requires an Object receiver",
                    receiver,
                );
            }
        };
        runtime::allocate(RtValue::Bool(contains))
    }

    /// Removes one Object property and returns its previous value or Null.
    pub(crate) fn delete(receiver: ValueRef, key: ValueRef) -> ValueRef {
        let key = match operations::string_value(key) {
            Ok(key) => key,
            Err(error) => return error,
        };
        let removed = match runtime::value_mut(receiver) {
            RtValue::Object(object) => object
                .entries
                .iter()
                .position(|(entry_key, _)| entry_key.as_ref() == key)
                .map(|index| object.entries.remove(index).1),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "delete requires an Object receiver",
                    receiver,
                );
            }
        };
        match removed {
            Some(value) => value,
            None => runtime::allocate(RtValue::None),
        }
    }

    /// Returns a new List containing Object keys in insertion order.
    pub(crate) fn keys(receiver: ValueRef) -> ValueRef {
        let checkpoint = gc::temporary_root_checkpoint();
        let keys = match runtime::value(receiver) {
            RtValue::Object(object) => object
                .entries
                .iter()
                .map(|(key, _)| String::from(key.as_ref()))
                .collect::<Vec<_>>(),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "keys requires an Object receiver",
                    receiver,
                );
            }
        };
        let elements = keys
            .into_iter()
            .map(|key| {
                let value =
                    runtime::allocate(RtValue::String(Box::new(RuntimeString::from_string(key))));
                gc::push_temporary_root(value);
                value
            })
            .collect();
        let result = runtime::allocate(RtValue::List(Box::new(RuntimeList { elements })));
        gc::restore_temporary_roots(checkpoint);
        result
    }

    /// Returns a new shallow List containing Object values in insertion order.
    pub(crate) fn values(receiver: ValueRef) -> ValueRef {
        let elements = match runtime::value(receiver) {
            RtValue::Object(object) => object.entries.iter().map(|(_, value)| *value).collect(),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "values requires an Object receiver",
                    receiver,
                );
            }
        };
        runtime::allocate(RtValue::List(Box::new(RuntimeList { elements })))
    }
}
