//! List methods with the results, Errors and order of checks.
//!
//! Index arguments are checked before the receiver.

use exs_layout::ValueRef;

use crate::access::list_index;
use crate::errors::recoverable;
use crate::lists::{
    allocate_list, ensure_capacity, item_address, len, list_object, list_reference, set_len,
};
use crate::memory::Memory;
use crate::methods::numbers::type_error;
use crate::numbers::int_value;
use crate::strings::{DATA, byte_len, char_count, copy_string_object, string_object};
use crate::values::equal;

/// Returns the List behind `receiver` or the TypeError `message`.
fn receiver_list(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    message: &str,
) -> Result<u32, Option<u64>> {
    list_object(receiver).ok_or_else(|| type_error(memory, vm, message, receiver))
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

/// Evaluates `push`: appends `item` and returns the new length.
pub(crate) fn push(memory: &mut impl Memory, vm: u32, receiver: u64, item: u64) -> Option<u64> {
    let list = check!(receiver_list(
        memory,
        vm,
        receiver,
        "push requires a List receiver"
    ));
    let count = len(memory, list);
    ensure_capacity(memory, vm, list, count.checked_add(1)?)?;
    memory.store_u64(item_address(memory, list, count), item);
    set_len(memory, list, count + 1);
    int_value(memory, vm, i64::from(count) + 1)
}

/// Evaluates `pop`: removes and returns the last element, None when empty.
pub(crate) fn pop(memory: &mut impl Memory, vm: u32, receiver: u64) -> Option<u64> {
    let list = check!(receiver_list(
        memory,
        vm,
        receiver,
        "pop requires a List receiver"
    ));
    let count = len(memory, list);
    if count == 0 {
        return Some(ValueRef::NONE.raw());
    }
    let slot = item_address(memory, list, count - 1);
    let value = memory.load_u64(slot);
    memory.store_u64(slot, 0);
    set_len(memory, list, count - 1);
    Some(value)
}

/// Evaluates `get` on a List: the element, or None outside the bounds.
pub(crate) fn get(memory: &mut impl Memory, vm: u32, receiver: u64, index: u64) -> Option<u64> {
    let position = check!(list_index(memory, vm, index));
    let Some(list) = list_object(receiver) else {
        return type_error(memory, vm, "get requires a List receiver", receiver);
    };
    Some(if position < len(memory, list) {
        memory.load_u64(item_address(memory, list, position))
    } else {
        ValueRef::NONE.raw()
    })
}

/// Evaluates `first` or `last`: the end element, None when empty.
pub(crate) fn end(memory: &mut impl Memory, vm: u32, receiver: u64, last: bool) -> Option<u64> {
    let message = if last {
        "last requires a List receiver"
    } else {
        "first requires a List receiver"
    };
    let list = check!(receiver_list(memory, vm, receiver, message));
    let count = len(memory, list);
    Some(match count {
        0 => ValueRef::NONE.raw(),
        _ => memory.load_u64(item_address(memory, list, if last { count - 1 } else { 0 })),
    })
}

/// Evaluates `slice(start, end)` on a List: a new List of the elements in `start..end`.
pub(crate) fn slice(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    start: u64,
    stop: u64,
) -> Option<u64> {
    let start = check!(list_index(memory, vm, start));
    let stop = check!(list_index(memory, vm, stop));
    let list = check!(receiver_list(
        memory,
        vm,
        receiver,
        "slice requires a List receiver"
    ));
    if start > stop || stop > len(memory, list) {
        return recoverable(
            memory,
            vm,
            "IndexError",
            "List slice range is outside the List bounds",
            receiver,
        );
    }
    let result = allocate_list(memory, vm, stop - start)?;
    memory.copy(
        item_address(memory, result, 0),
        item_address(memory, list, start),
        (stop - start) * 8,
    );
    set_len(memory, result, stop - start);
    list_reference(result)
}

/// Evaluates `extend(other)`: appends the elements of another List and returns the new length.
pub(crate) fn extend(memory: &mut impl Memory, vm: u32, receiver: u64, other: u64) -> Option<u64> {
    let Some(source) = list_object(other) else {
        return type_error(memory, vm, "extend requires a List argument", other);
    };
    let list = check!(receiver_list(
        memory,
        vm,
        receiver,
        "extend requires a List receiver"
    ));
    let (count, added) = (len(memory, list), len(memory, source));
    let total = count.checked_add(added)?;
    ensure_capacity(memory, vm, list, total)?;
    // Copying before growing the length also covers extending a List with itself.
    memory.copy(
        item_address(memory, list, count),
        item_address(memory, source, 0),
        added * 8,
    );
    set_len(memory, list, total);
    int_value(memory, vm, i64::from(total))
}

/// Evaluates `reverse` (in place, returning None) or `reversed` (a reversed copy).
pub(crate) fn reverse(memory: &mut impl Memory, vm: u32, receiver: u64, copy: bool) -> Option<u64> {
    let message = if copy {
        "reversed requires a List receiver"
    } else {
        "reverse requires a List receiver"
    };
    let list = check!(receiver_list(memory, vm, receiver, message));
    let count = len(memory, list);
    let target = if copy {
        let result = allocate_list(memory, vm, count)?;
        memory.copy(
            item_address(memory, result, 0),
            item_address(memory, list, 0),
            count * 8,
        );
        set_len(memory, result, count);
        result
    } else {
        list
    };
    for index in 0..count / 2 {
        let (low, high) = (
            item_address(memory, target, index),
            item_address(memory, target, count - 1 - index),
        );
        let (first, second) = (memory.load_u64(low), memory.load_u64(high));
        memory.store_u64(low, second);
        memory.store_u64(high, first);
    }
    if copy {
        list_reference(target)
    } else {
        Some(ValueRef::UNIT.raw())
    }
}

/// Evaluates `contains` on a List with the language's equality.
pub(crate) fn contains(memory: &mut impl Memory, vm: u32, receiver: u64, item: u64) -> Option<u64> {
    let list = check!(receiver_list(
        memory,
        vm,
        receiver,
        "contains requires a List receiver"
    ));
    let found = (0..len(memory, list)).any(|index| {
        equal(
            memory,
            memory.load_u64(item_address(memory, list, index)),
            item,
        )
    });
    Some(ValueRef::boolean(found).raw())
}

/// Evaluates `index_of` or `last_index_of`: the matching index, None when absent.
pub(crate) fn index_of(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    item: u64,
    last: bool,
) -> Option<u64> {
    let message = if last {
        "last_index_of requires a List receiver"
    } else {
        "index_of requires a List receiver"
    };
    let list = check!(receiver_list(memory, vm, receiver, message));
    let count = len(memory, list);
    let matches = |index: &u32| {
        equal(
            memory,
            memory.load_u64(item_address(memory, list, *index)),
            item,
        )
    };
    let found = if last {
        (0..count).rev().find(matches)
    } else {
        (0..count).find(matches)
    };
    match found {
        Some(index) => int_value(memory, vm, i64::from(index)),
        None => Some(ValueRef::NONE.raw()),
    }
}

/// Evaluates `insert(index, value)`: inserts before `index` and returns Unit.
pub(crate) fn insert(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    index: u64,
    value: u64,
) -> Option<u64> {
    let position = check!(list_index(memory, vm, index));
    let list = check!(receiver_list(
        memory,
        vm,
        receiver,
        "insert requires a List receiver"
    ));
    let count = len(memory, list);
    if position > count {
        return recoverable(
            memory,
            vm,
            "IndexError",
            "List insertion index is outside the List bounds",
            index,
        );
    }
    ensure_capacity(memory, vm, list, count.checked_add(1)?)?;
    let slot = item_address(memory, list, position);
    memory.copy(slot + 8, slot, (count - position) * 8);
    memory.store_u64(slot, value);
    set_len(memory, list, count + 1);
    Some(ValueRef::UNIT.raw())
}

/// Evaluates `remove(index)`: removes and returns the element at `index`.
pub(crate) fn remove(memory: &mut impl Memory, vm: u32, receiver: u64, index: u64) -> Option<u64> {
    let position = check!(list_index(memory, vm, index));
    let list = check!(receiver_list(
        memory,
        vm,
        receiver,
        "remove requires a List receiver"
    ));
    let count = len(memory, list);
    if position >= count {
        return recoverable(
            memory,
            vm,
            "IndexError",
            "List index is outside the List bounds",
            index,
        );
    }
    let slot = item_address(memory, list, position);
    let value = memory.load_u64(slot);
    memory.copy(slot, slot + 8, (count - position - 1) * 8);
    memory.store_u64(item_address(memory, list, count - 1), 0);
    set_len(memory, list, count - 1);
    Some(value)
}

/// Removes every element of the List at `list`.
pub(crate) fn clear(memory: &mut impl Memory, list: u32) {
    let count = len(memory, list);
    memory.fill(item_address(memory, list, 0), count * 8, 0);
    set_len(memory, list, 0);
}

/// Evaluates `join(separator)` of a List of Strings.
pub(crate) fn join(
    memory: &mut impl Memory,
    vm: u32,
    receiver: u64,
    separator: u64,
) -> Option<u64> {
    let Some(separator_object) = string_object(separator) else {
        return type_error(memory, vm, "join requires a String separator", separator);
    };
    let list = check!(receiver_list(
        memory,
        vm,
        receiver,
        "join requires a List receiver"
    ));
    let count = len(memory, list);
    let (mut bytes, mut chars) = (0_u32, 0_u32);
    for index in 0..count {
        let item = memory.load_u64(item_address(memory, list, index));
        let Some(text) = string_object(item) else {
            return type_error(
                memory,
                vm,
                "join requires every List item to be a String",
                item,
            );
        };
        if index > 0 {
            bytes = bytes.checked_add(byte_len(memory, separator_object))?;
            chars = chars.checked_add(char_count(memory, separator_object))?;
        }
        bytes = bytes.checked_add(byte_len(memory, text))?;
        chars = chars.checked_add(char_count(memory, text))?;
    }
    copy_string_object(memory, vm, bytes, chars, |memory, mut position| {
        for index in 0..count {
            if index > 0 {
                let separator_len = byte_len(memory, separator_object);
                memory.copy(position, separator_object + DATA, separator_len);
                position += separator_len;
            }
            let text =
                string_object(memory.load_u64(item_address(memory, list, index))).unwrap_or(0);
            let text_len = byte_len(memory, text);
            memory.copy(position, text + DATA, text_len);
            position += text_len;
        }
    })
}
