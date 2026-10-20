//! Mutable insertion-ordered runtime Object payloads.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use exs_value::ValueRef;

/// One shared, immutable Object key or enum name.
#[derive(Clone)]
pub(crate) enum SharedName {
    /// Text of a pinned compiler literal String, which lives for the whole execution.
    Literal(&'static str),
    /// Text owned by this Object or enum, for host-supplied and dynamically built names.
    Owned(Box<str>),
}

impl SharedName {
    /// Returns the name text.
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Literal(text) => text,
            Self::Owned(text) => text,
        }
    }
}

impl core::ops::Deref for SharedName {
    type Target = str;

    /// Returns the name text.
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for SharedName {
    /// Returns the name text.
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl core::fmt::Display for SharedName {
    /// Writes the name text.
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<&str> for SharedName {
    /// Copies one name into runtime-owned storage.
    fn from(text: &str) -> Self {
        Self::Owned(Box::from(text))
    }
}

impl From<String> for SharedName {
    /// Moves one name into runtime-owned storage without copying it.
    fn from(text: String) -> Self {
        Self::Owned(text.into_boxed_str())
    }
}

/// Returns whether one shared name holds the given text, checking identity first.
pub(crate) fn same_name(left: &SharedName, right: &str) -> bool {
    let left = left.as_str();
    (core::ptr::eq(left.as_ptr(), right.as_ptr()) && left.len() == right.len()) || left == right
}

/// Returns the entry position of one key, trying the key's last known position first.
pub(crate) fn find_hinted_entry(
    entries: &[(SharedName, ValueRef)],
    key: &str,
    hint: &core::cell::Cell<u32>,
) -> Option<usize> {
    let hinted = hint.get() as usize;
    if let Some((entry_key, _)) = entries.get(hinted)
        && same_name(entry_key, key)
    {
        return Some(hinted);
    }
    let position = entries
        .iter()
        .position(|(entry_key, _)| same_name(entry_key, key))?;
    if let Ok(position) = u32::try_from(position) {
        hint.set(position);
    }
    Some(position)
}

/// A mutable insertion-ordered mapping from string keys to runtime values.
pub(crate) struct RuntimeObject {
    /// Compiler-owned nominal type tag, if this Object was constructed through `Type { ... }`.
    pub(crate) type_id: Option<u32>,
    /// Key-value entries in insertion order.
    pub(crate) entries: Vec<(SharedName, ValueRef)>,
    /// Private enum metadata, present only for nominal enum values.
    pub(crate) enum_data: Option<RuntimeEnum>,
}

/// One tagged enum payload carried by a nominal Object allocation.
pub(crate) struct RuntimeEnum {
    /// Stable host-boundary identity for the owning enum declaration.
    pub(crate) type_identity: SharedName,
    /// Source-visible selected variant name.
    pub(crate) variant: SharedName,
    /// Payload values in declaration order.
    pub(crate) fields: Vec<ValueRef>,
}

impl RuntimeObject {
    /// Creates an empty runtime Object.
    pub(crate) const fn new() -> Self {
        Self {
            type_id: None,
            entries: Vec::new(),
            enum_data: None,
        }
    }

    /// Creates an empty Object carrying one compiler-owned nominal type tag.
    pub(crate) const fn typed(type_id: u32) -> Self {
        Self {
            type_id: Some(type_id),
            entries: Vec::new(),
            enum_data: None,
        }
    }

    /// Creates one nominal enum value with its private variant payload.
    pub(crate) fn enumeration(type_id: Option<u32>, enum_data: RuntimeEnum) -> Self {
        Self {
            type_id,
            entries: Vec::new(),
            enum_data: Some(enum_data),
        }
    }
}

pub(crate) mod operations {
    //! Wasm operations for mutable runtime Objects.

    use alloc::boxed::Box;
    use alloc::string::String;
    use alloc::vec::Vec;

    use exs_value::ValueRef;
    use exs_value::layout::MAX_INLINE_TYPE_ID;

    use crate::gc;
    use crate::runtime;
    use crate::value::{RtValue, RuntimeList, RuntimeObject, RuntimeString, Value, operations};

    /// Allocates an empty mutable runtime Object.
    pub(crate) fn new_value() -> ValueRef {
        runtime::allocate(RtValue::Object(Box::new(RuntimeObject::new())))
    }

    /// Allocates an empty nominal Object with one compiler-provided type tag.
    pub(crate) fn new_typed_value(type_id: u32) -> ValueRef {
        runtime::allocate(RtValue::Object(Box::new(RuntimeObject::typed(type_id))))
    }

    /// Returns whether one value is an Object carrying the requested nominal type tag.
    pub(crate) fn has_type(receiver: ValueRef, type_id: u32) -> bool {
        // Small ids are always encoded in the reference, so only large ids need the heap.
        if type_id <= MAX_INLINE_TYPE_ID {
            return receiver.inline_type_id() == Some(type_id);
        }
        matches!(runtime::value(receiver), Value::Object(object) if object.type_id == Some(type_id))
    }

    /// Reads one Object property or returns Null when it is absent.
    pub(crate) fn get(receiver: ValueRef, index: ValueRef) -> ValueRef {
        let key = match operations::string_str(index) {
            Ok(key) => key,
            Err(error) => return error,
        };
        let result = match runtime::value(receiver) {
            Value::Object(object) => object
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
            None => runtime::none_value(),
        }
    }

    /// Returns the text of one pinned compiler literal String used as a key.
    ///
    /// The compiler only passes literal Strings, which stay pinned for the whole execution,
    /// so their text can be shared by Object entries without copying.
    fn literal_key(
        key: ValueRef,
    ) -> Result<(&'static str, &'static core::cell::Cell<u32>), ValueRef> {
        match runtime::value(key) {
            Value::String(text) => Ok((text.as_str(), text.key_hint())),
            _ => Err(runtime::recoverable_error(
                "TypeError",
                "Object keys and method names require a String value",
                key,
            )),
        }
    }

    /// Reads one property named by a compiler literal, trying its last known position first.
    ///
    /// Non-Object receivers keep the generic indexing behavior and errors.
    pub(crate) fn get_literal(receiver: ValueRef, key: ValueRef) -> ValueRef {
        let Value::Object(object) = runtime::value(receiver) else {
            return operations::index_get(receiver, key);
        };
        let (text, hint) = match literal_key(key) {
            Ok(key) => key,
            Err(error) => return error,
        };
        match super::find_hinted_entry(&object.entries, text, hint) {
            Some(position) => object.entries[position].1,
            None => runtime::none_value(),
        }
    }

    /// Creates or replaces one property named by a compiler literal.
    ///
    /// Non-Object receivers keep the generic index assignment behavior and errors.
    pub(crate) fn set_literal(
        receiver: ValueRef,
        key: ValueRef,
        replacement: ValueRef,
    ) -> ValueRef {
        if !matches!(runtime::value(receiver), Value::Object(_)) {
            return operations::index_set(receiver, key, replacement);
        }
        let (text, hint) = match literal_key(key) {
            Ok(key) => key,
            Err(error) => return error,
        };
        let Some(RtValue::Object(object)) = runtime::value_mut(receiver) else {
            runtime::trap();
        };
        match super::find_hinted_entry(&object.entries, text, hint) {
            Some(position) => object.entries[position].1 = replacement,
            None => object
                .entries
                .push((super::SharedName::Literal(text), replacement)),
        }
        replacement
    }

    /// Allocates an empty nominal Object with room for its declared fields.
    pub(crate) fn new_typed_with_capacity(type_id: u32, capacity: usize) -> ValueRef {
        let mut object = RuntimeObject::typed(type_id);
        object.entries.reserve_exact(capacity);
        runtime::allocate(RtValue::Object(Box::new(object)))
    }

    /// Appends one compiler-validated field to an Object under construction without a key scan.
    pub(crate) fn push_literal_field(receiver: ValueRef, key: ValueRef, value: ValueRef) {
        let Ok((text, _)) = literal_key(key) else {
            runtime::trap();
        };
        let Some(RtValue::Object(object)) = runtime::value_mut(receiver) else {
            runtime::trap();
        };
        object
            .entries
            .push((super::SharedName::Literal(text), value));
    }

    /// Reads one Object property or returns a caller-provided default when it is absent.
    pub(crate) fn get_or(receiver: ValueRef, index: ValueRef, default: ValueRef) -> ValueRef {
        let key = match operations::string_str(index) {
            Ok(key) => key,
            Err(error) => return error,
        };
        match runtime::value(receiver) {
            Value::Object(object) => object
                .entries
                .iter()
                .find_map(|(entry_key, value)| (entry_key.as_ref() == key).then_some(*value))
                .unwrap_or(default),
            _ => runtime::recoverable_error(
                "TypeError",
                "get_or requires an Object receiver",
                receiver,
            ),
        }
    }

    /// Creates or replaces one Object property.
    pub(crate) fn set(receiver: ValueRef, index: ValueRef, replacement: ValueRef) -> ValueRef {
        let key = match operations::string_str(index) {
            Ok(key) => key,
            Err(error) => return error,
        };
        match runtime::value_mut(receiver) {
            Some(RtValue::Object(object)) => {
                if let Some((_, value)) = object
                    .entries
                    .iter_mut()
                    .find(|(entry_key, _)| entry_key.as_ref() == key)
                {
                    *value = replacement;
                } else {
                    object.entries.push((key.into(), replacement));
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
        let key = match operations::string_str(key) {
            Ok(key) => key,
            Err(error) => return error,
        };
        let contains = match runtime::value(receiver) {
            Value::Object(object) => object
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
        runtime::bool_value(contains)
    }

    /// Removes one Object property and returns its previous value or Null.
    pub(crate) fn delete(receiver: ValueRef, key: ValueRef) -> ValueRef {
        let key = match operations::string_str(key) {
            Ok(key) => key,
            Err(error) => return error,
        };
        let removed = match runtime::value_mut(receiver) {
            Some(RtValue::Object(object)) => object
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
            None => runtime::none_value(),
        }
    }

    /// Returns a new List containing Object keys in insertion order.
    pub(crate) fn keys(receiver: ValueRef) -> ValueRef {
        let checkpoint = gc::temporary_root_checkpoint();
        let keys = match runtime::value(receiver) {
            Value::Object(object) => object
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
                let value = runtime::allocate(RtValue::String(RuntimeString::from_string(key)));
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
            Value::Object(object) => object.entries.iter().map(|(_, value)| *value).collect(),
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

    /// Removes every Object property while retaining the Object identity.
    pub(crate) fn clear(receiver: ValueRef) -> ValueRef {
        match runtime::value_mut(receiver) {
            Some(RtValue::Object(object)) => object.entries.clear(),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "clear requires an Object receiver",
                    receiver,
                );
            }
        }
        runtime::none_value()
    }

    /// Returns key-value pair Lists in Object insertion order.
    pub(crate) fn entries(receiver: ValueRef) -> ValueRef {
        let entries = match runtime::value(receiver) {
            Value::Object(object) => object
                .entries
                .iter()
                .map(|(key, value)| (String::from(key.as_ref()), *value))
                .collect::<Vec<_>>(),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "entries requires an Object receiver",
                    receiver,
                );
            }
        };
        let checkpoint = gc::temporary_root_checkpoint();
        let mut pairs = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            let key = runtime::allocate(RtValue::String(RuntimeString::from_string(key)));
            gc::push_temporary_root(key);
            let pair = runtime::allocate(RtValue::List(Box::new(RuntimeList {
                elements: alloc::vec![key, value],
            })));
            gc::push_temporary_root(pair);
            pairs.push(pair);
        }
        let result = runtime::allocate(RtValue::List(Box::new(RuntimeList { elements: pairs })));
        gc::restore_temporary_roots(checkpoint);
        result
    }

    /// Returns a new Object whose right-hand properties overwrite matching left-hand keys.
    pub(crate) fn merge(receiver: ValueRef, other: ValueRef) -> ValueRef {
        let mut entries = match runtime::value(receiver) {
            Value::Object(object) => object.entries.clone(),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "merge requires an Object receiver",
                    receiver,
                );
            }
        };
        let other_entries = match runtime::value(other) {
            Value::Object(object) => object.entries.clone(),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "merge requires an Object argument",
                    other,
                );
            }
        };
        for (key, value) in other_entries {
            if let Some((_, existing)) = entries
                .iter_mut()
                .find(|(existing_key, _)| existing_key.as_ref() == key.as_ref())
            {
                *existing = value;
            } else {
                entries.push((key, value));
            }
        }
        runtime::allocate(RtValue::Object(Box::new(RuntimeObject {
            type_id: None,
            entries,
            enum_data: None,
        })))
    }

    /// Builds an Object from two-item List entries whose keys are Strings.
    pub(crate) fn from_entries(entries: ValueRef) -> ValueRef {
        let entries = match runtime::value(entries) {
            Value::List(entries) => entries.elements.clone(),
            _ => {
                return runtime::recoverable_error(
                    "TypeError",
                    "Object::from_entries requires a List",
                    entries,
                );
            }
        };
        let mut result = RuntimeObject::new();
        for entry in entries {
            let (key, value) = match runtime::value(entry) {
                Value::List(pair) if pair.elements.len() == 2 => {
                    let key = match runtime::value(pair.elements[0]) {
                        Value::String(key) => String::from(key.as_str()),
                        _ => {
                            return runtime::recoverable_error(
                                "TypeError",
                                "Object::from_entries requires String keys",
                                pair.elements[0],
                            );
                        }
                    };
                    (key, pair.elements[1])
                }
                _ => {
                    return runtime::recoverable_error(
                        "TypeError",
                        "Object::from_entries requires two-item List entries",
                        entry,
                    );
                }
            };
            if let Some((_, existing)) = result
                .entries
                .iter_mut()
                .find(|(existing_key, _)| existing_key.as_ref() == key)
            {
                *existing = value;
            } else {
                result.entries.push((key.into(), value));
            }
        }
        runtime::allocate(RtValue::Object(Box::new(result)))
    }
}
