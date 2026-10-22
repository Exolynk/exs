//! Object methods with the results, Errors and order of checks.
//!
//! An Object's entries are, in order, the present declared fields of a typed Object followed
//! by its extra keys, or the keys of an untyped Object. A declared field removed by `delete`
//! or `clear` holds an empty slot and is no entry until it is set again.

use exs_layout::ValueRef;

use crate::lists::{allocate_list, item_address, len, list_object, list_reference, set_len};
use crate::memory::Memory;
use crate::methods::numbers::type_error;
use crate::objects::{
    empty_object, object_clear, object_reference, object_remove, object_set, shape, untyped_object,
    value_address,
};
use crate::shapes::{key, key_count};
use crate::strings::string_object;
use crate::typed::{
    extra, ordered_slot, set_extra, slot_address, slot_count, type_entry, type_id, typed_object,
};
use crate::vm::Vm;

/// Message of the TypeError for an Object key that is no String.
const KEY_NOT_STRING: &str = "Object keys and method names require a String value";

/// An Object receiver.
#[derive(Clone, Copy)]
enum Receiver {
    /// A typed Object at this address.
    Typed(u32),
    /// An untyped Object at this address.
    Untyped(u32),
    /// An enum, an Object without keys.
    Enum,
}

/// Classifies an Object receiver.
fn object(memory: &impl Memory, raw: u64) -> Option<Receiver> {
    typed_object(memory, raw)
        .map(Receiver::Typed)
        .or_else(|| untyped_object(memory, raw).map(Receiver::Untyped))
        .or_else(|| crate::enums::enum_object(memory, raw).map(|_| Receiver::Enum))
}

/// Returns the number of entries of `receiver`.
fn entry_count(memory: &impl Memory, vm: u32, receiver: Receiver) -> u32 {
    match receiver {
        Receiver::Enum => 0,
        Receiver::Untyped(object) => key_count(memory, shape(memory, object)),
        Receiver::Typed(object) => {
            let declared = declared_count(memory, vm, object);
            let present = (0..declared)
                .filter(|index| memory.load_u64(slot_address(object, *index)) != 0)
                .count() as u32;
            present
                + match extra(memory, object) {
                    0 => 0,
                    keys => key_count(memory, shape(memory, keys)),
                }
        }
    }
}

/// Returns the number of declared field slots the type of a typed Object names.
fn declared_count(memory: &impl Memory, vm: u32, object: u32) -> u32 {
    type_entry(memory, vm, type_id(memory, object))
        .map_or(0, |entry| entry.field_count.min(slot_count(memory, object)))
}

/// Returns the key and value of entry `index` of `receiver`.
fn entry(memory: &impl Memory, vm: u32, receiver: Receiver, index: u32) -> (u64, u64) {
    match receiver {
        Receiver::Enum => (0, 0),
        Receiver::Untyped(object) => (
            key(memory, shape(memory, object), index),
            memory.load_u64(value_address(memory, object, index)),
        ),
        Receiver::Typed(object) => {
            let mut remaining = index;
            if let Some(entry) = type_entry(memory, vm, type_id(memory, object)) {
                for position in 0..declared_count(memory, vm, object) {
                    let field = ordered_slot(memory, object, position);
                    let value = memory.load_u64(slot_address(object, field));
                    if value == 0 {
                        continue;
                    }
                    if remaining == 0 {
                        return (memory.load_u64(entry.field_names + field * 8), value);
                    }
                    remaining -= 1;
                }
            }
            let keys = extra(memory, object);
            (
                key(memory, shape(memory, keys), remaining),
                memory.load_u64(value_address(memory, keys, remaining)),
            )
        }
    }
}

/// Returns the value of the key named by the String at `name`, if `receiver` has it.
fn lookup(memory: &impl Memory, vm: u32, receiver: Receiver, name: u32) -> Option<u64> {
    (0..entry_count(memory, vm, receiver))
        .map(|index| entry(memory, vm, receiver, index))
        .find(|(key, _)| {
            string_object(*key).is_some_and(|key| crate::strings::strings_equal(memory, key, name))
        })
        .map(|(_, value)| value)
}

/// Returns the value of the own key whose text is `key` of the Object `raw`, or `None` when
/// `raw` is no Object or lacks the key.
pub(crate) fn field_value(memory: &impl Memory, vm: u32, raw: u64, key: &[u8]) -> Option<u64> {
    let receiver = object(memory, raw)?;
    (0..entry_count(memory, vm, receiver))
        .map(|index| entry(memory, vm, receiver, index))
        .find(|(name, _)| {
            string_object(*name)
                .is_some_and(|name| crate::strings::text(memory, name).as_bytes() == key)
        })
        .map(|(_, value)| value)
}

/// Reads the String key of an Object method, or creates its TypeError.
fn key_argument(memory: &mut impl Memory, vm: u32, raw: u64) -> Result<u32, Option<u64>> {
    string_object(raw).ok_or_else(|| type_error(memory, vm, KEY_NOT_STRING, raw))
}

/// Unwraps a check, returning its Error value from the enclosing method.
macro_rules! check {
    ($result:expr) => {
        match $result {
            Ok(value) => value,
            Err(error) => return error,
        }
    };
}

/// Reads the Object receiver of a method, or creates the TypeError `message`.
fn receiver(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
    message: &str,
) -> Result<Receiver, Option<u64>> {
    object(memory, raw).ok_or_else(|| type_error(memory, vm, message, raw))
}

/// Evaluates `get(key)` on an Object: the value, None when absent.
pub(crate) fn get(memory: &mut impl Memory, vm: u32, raw: u64, name: u64) -> Option<u64> {
    let name = check!(key_argument(memory, vm, name));
    let receiver = check!(receiver(
        memory,
        vm,
        raw,
        "index access requires an Object receiver"
    ));
    Some(lookup(memory, vm, receiver, name).unwrap_or(ValueRef::NONE.raw()))
}

/// Evaluates `get_or(key, default)`.
pub(crate) fn get_or(
    memory: &mut impl Memory,
    vm: u32,
    raw: u64,
    name: u64,
    default: u64,
) -> Option<u64> {
    let name = check!(key_argument(memory, vm, name));
    let receiver = check!(receiver(
        memory,
        vm,
        raw,
        "get_or requires an Object receiver"
    ));
    Some(lookup(memory, vm, receiver, name).unwrap_or(default))
}

/// Evaluates `has(key)`.
pub(crate) fn has(memory: &mut impl Memory, vm: u32, raw: u64, name: u64) -> Option<u64> {
    let name = check!(key_argument(memory, vm, name));
    let receiver = check!(receiver(memory, vm, raw, "has requires an Object receiver"));
    Some(ValueRef::boolean(lookup(memory, vm, receiver, name).is_some()).raw())
}

/// Evaluates `delete(key)`: removes the key and returns its value, None when absent.
pub(crate) fn delete(memory: &mut impl Memory, vm: u32, raw: u64, name_raw: u64) -> Option<u64> {
    let name = check!(key_argument(memory, vm, name_raw));
    let removed = match check!(receiver(
        memory,
        vm,
        raw,
        "delete requires an Object receiver"
    )) {
        Receiver::Enum => None,
        Receiver::Untyped(object) => object_remove(memory, vm, object, name)?,
        Receiver::Typed(object) => {
            match crate::typed::field_address(memory, vm, object, name_raw) {
                Some(slot) => {
                    let value = memory.load_u64(slot);
                    memory.store_u64(slot, 0);
                    (value != 0).then_some(value)
                }
                None => match extra(memory, object) {
                    0 => None,
                    keys => object_remove(memory, vm, keys, name)?,
                },
            }
        }
    };
    Some(removed.unwrap_or(ValueRef::NONE.raw()))
}

/// Removes every entry of an Object receiver.
pub(crate) fn clear(memory: &mut impl Memory, vm: u32, raw: u64) -> Option<()> {
    match object(memory, raw)? {
        Receiver::Enum => Some(()),
        Receiver::Untyped(object) => object_clear(memory, vm, object),
        Receiver::Typed(object) => {
            let declared = slot_count(memory, object);
            memory.fill(slot_address(object, 0), declared * 8, 0);
            set_extra(memory, object, 0);
            Some(())
        }
    }
}

/// Returns whether `raw` is an Object.
pub(crate) fn is_object(memory: &impl Memory, raw: u64) -> bool {
    object(memory, raw).is_some()
}

/// Returns the number of entries of an Object, or `None` for other values.
pub(crate) fn object_len(memory: &impl Memory, vm: u32, raw: u64) -> Option<u32> {
    Some(entry_count(memory, vm, object(memory, raw)?))
}

/// The entry part `keys`, `values` and `entries` collect.
#[derive(Clone, Copy)]
pub(crate) enum Collect {
    /// The key Strings.
    Keys,
    /// The values.
    Values,
    /// Two-element `[key, value]` Lists.
    Entries,
}

/// Evaluates `keys`, `values` or `entries` into a new List in entry order.
pub(crate) fn collect(memory: &mut impl Memory, vm: u32, raw: u64, part: Collect) -> Option<u64> {
    let message = match part {
        Collect::Keys => "keys requires an Object receiver",
        Collect::Values => "values requires an Object receiver",
        Collect::Entries => "entries requires an Object receiver",
    };
    let receiver = check!(receiver(memory, vm, raw, message));
    let count = entry_count(memory, vm, receiver);
    let list = allocate_list(memory, vm, count)?;
    let state = Vm(vm);
    let root = state.push_root(memory, list_reference(list)?)?;
    for index in 0..count {
        let (key, value) = entry(memory, vm, receiver, index);
        let item = match part {
            Collect::Keys => key,
            Collect::Values => value,
            Collect::Entries => {
                let pair = allocate_list(memory, vm, 2);
                let Some(pair) = pair else {
                    state.pop_roots(memory, root);
                    return None;
                };
                memory.store_u64(item_address(memory, pair, 0), key);
                memory.store_u64(item_address(memory, pair, 1), value);
                set_len(memory, pair, 2);
                list_reference(pair)?
            }
        };
        memory.store_u64(item_address(memory, list, index), item);
        set_len(memory, list, index + 1);
    }
    state.pop_roots(memory, root);
    list_reference(list)
}

/// Evaluates `merge(other)`: a new untyped Object of the receiver's entries, overwritten and
/// extended by the other Object's entries.
pub(crate) fn merge(memory: &mut impl Memory, vm: u32, raw: u64, other: u64) -> Option<u64> {
    let left = check!(receiver(
        memory,
        vm,
        raw,
        "merge requires an Object receiver"
    ));
    let Some(right) = object(memory, other) else {
        return type_error(memory, vm, "merge requires an Object argument", other);
    };
    let total = entry_count(memory, vm, left) + entry_count(memory, vm, right);
    let result = empty_object(memory, vm, total)?;
    let reference = object_reference(result)?;
    let state = Vm(vm);
    let root = state.push_root(memory, reference)?;
    let mut copied = Some(());
    for source in [left, right] {
        for index in 0..entry_count(memory, vm, source) {
            let (key, value) = entry(memory, vm, source, index);
            copied = copied.and_then(|()| object_set(memory, vm, result, key, value));
        }
    }
    state.pop_roots(memory, root);
    copied.map(|()| reference)
}

/// Evaluates `Object::from_entries(entries)`: an untyped Object of `[key, value]` Lists, later
/// keys overwriting earlier ones.
pub(crate) fn from_entries(memory: &mut impl Memory, vm: u32, entries: u64) -> Option<u64> {
    let Some(list) = list_object(entries) else {
        return type_error(memory, vm, "Object::from_entries requires a List", entries);
    };
    let count = len(memory, list);
    let result = empty_object(memory, vm, count)?;
    let reference = object_reference(result)?;
    let state = Vm(vm);
    let root = state.push_root(memory, reference)?;
    let mut outcome = Some(reference);
    for index in 0..count {
        let entry = memory.load_u64(item_address(memory, list, index));
        let pair = list_object(entry).filter(|pair| len(memory, *pair) == 2);
        let Some(pair) = pair else {
            outcome = type_error(
                memory,
                vm,
                "Object::from_entries requires two-item List entries",
                entry,
            );
            break;
        };
        let key = memory.load_u64(item_address(memory, pair, 0));
        if string_object(key).is_none() {
            outcome = type_error(memory, vm, "Object::from_entries requires String keys", key);
            break;
        }
        let value = memory.load_u64(item_address(memory, pair, 1));
        if object_set(memory, vm, result, key, value).is_none() {
            outcome = None;
            break;
        }
    }
    state.pop_roots(memory, root);
    outcome
}
