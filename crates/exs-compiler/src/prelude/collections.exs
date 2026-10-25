/// Maps every iterable item through a callback, stopping at the first callback Error.
private fn __exs_list_map(iterable: Any, callback: Fn) -> List | Error {
    let result = [];
    for item in iterable {
        let mapped = callback(item);
        if mapped is Error {
            ret mapped;
        }
        result.push(mapped);
    }
    ret result;
}

/// Keeps every iterable item whose callback result is true, stopping at callback Errors.
private fn __exs_list_filter(iterable: Any, callback: Fn) -> List | Error {
    let result = [];
    for item in iterable {
        let accepted = callback(item);
        if accepted is Error {
            ret accepted;
        }
        if accepted {
            result.push(item);
        }
    }
    ret result;
}

/// Returns the first iterable item whose callback result is true, or None when absent.
private fn __exs_list_find(iterable: Any, callback: Fn) -> Any | None | Error {
    for item in iterable {
        let accepted = callback(item);
        if accepted is Error {
            ret accepted;
        }
        if accepted {
            ret item;
        }
    }
    ret None;
}

/// Returns whether any iterable item produces true, stopping at callback Errors.
private fn __exs_list_any(iterable: Any, callback: Fn) -> Bool | Error {
    for item in iterable {
        let accepted = callback(item);
        if accepted is Error {
            ret accepted;
        }
        if accepted {
            ret true;
        }
    }
    ret false;
}

/// Returns whether every iterable item produces true, stopping at callback Errors.
private fn __exs_list_all(iterable: Any, callback: Fn) -> Bool | Error {
    for item in iterable {
        let accepted = callback(item);
        if accepted is Error {
            ret accepted;
        }
        if !accepted {
            ret false;
        }
    }
    ret true;
}

/// Invokes a callback for every iterable item, stopping at the first callback Error.
private fn __exs_list_each(iterable: Any, callback: Fn) -> Unit | Error {
    for item in iterable {
        let result = callback(item);
        if result is Error {
            ret result;
        }
    }
    ret Unit;
}

/// Folds iterable items from an explicit initial accumulator, stopping at callback Errors.
private fn __exs_list_reduce(iterable: Any, initial: Any, callback: Fn) -> Any | Error {
    let accumulator = initial;
    for item in iterable {
        let result = callback(accumulator, item);
        if result is Error {
            ret result;
        }
        accumulator = result;
    }
    ret accumulator;
}

/// Collects every item from an iterable into a new List.
private fn __exs_iterator_collect(iterable: Any) -> List | Error {
    let result = [];
    for item in iterable {
        result.push(item);
    }
    ret result;
}

/// Counts every item produced by an iterable.
private fn __exs_iterator_count(iterable: Any) -> Int | Error {
    let count = 0;
    for item in iterable {
        count = (count + 1)?;
    }
    ret count;
}

/// Collects every iterable item into a new List.
private fn __exs_iterator_to_list(iterable: Any) -> List | Error {
    ret __exs_iterator_collect(iterable);
}

/// Returns whether a List holds an item equal to `value` under `==`.
private fn __exs_list_contains(list: List, value: Any) -> Bool | Error {
    for item in list {
        if item == value {
            ret true;
        }
    }
    ret false;
}

/// Returns the index of the first List item equal to `value` under `==`, or None.
private fn __exs_list_index_of(list: List, value: Any) -> Int | None | Error {
    let index = 0;
    for item in list {
        if item == value {
            ret index;
        }
        index = index + 1;
    }
    ret None;
}

/// Returns the index of the last List item equal to `value` under `==`, or None.
private fn __exs_list_last_index_of(list: List, value: Any) -> Int | None | Error {
    let index = list.length() - 1;
    while index >= 0 {
        if list[index] == value {
            ret index;
        }
        index = index - 1;
    }
    ret None;
}

/// Sorts a List in place in ascending order under `<`; equal items keep their order.
private fn __exs_list_sort(list: List) -> Unit | Error {
    let sorted = __exs_merge_sort(list, None)?;
    list.clear();
    list.extend(sorted);
    ret Unit;
}

/// Sorts a List in place by the Ordering `compare(a, b)` returns; equal items keep their order.
private fn __exs_list_sort_by(list: List, compare: Fn) -> Unit | Error {
    let sorted = __exs_merge_sort(list, compare)?;
    list.clear();
    list.extend(sorted);
    ret Unit;
}

/// Returns a new List of the items in ascending order under `<`; equal items keep their order.
private fn __exs_list_sorted(list: List) -> List | Error {
    ret __exs_merge_sort(list, None);
}

/// Returns a new List of the items ordered by the Ordering `compare(a, b)` returns.
private fn __exs_list_sorted_by(list: List, compare: Fn) -> List | Error {
    ret __exs_merge_sort(list, compare);
}

/// Returns a new, stably merge-sorted List of `items`, ordered by `compare` or else by `<`.
private fn __exs_merge_sort(items: List, compare: Fn | None) -> List | Error {
    let length = items.length();
    if length <= 1 {
        ret items.slice(0, length);
    }
    let middle = length.div_euclid(2);
    let left = __exs_merge_sort(items.slice(0, middle), compare)?;
    let right = __exs_merge_sort(items.slice(middle, length), compare)?;
    let merged = [];
    let i = 0;
    let j = 0;
    while i < left.length() && j < right.length() {
        if __exs_sorts_before(right[j], left[i], compare)? {
            merged.push(right[j]);
            j = j + 1;
        } else {
            merged.push(left[i]);
            i = i + 1;
        }
    }
    merged.extend(left.slice(i, left.length()));
    merged.extend(right.slice(j, right.length()));
    ret merged;
}

/// Returns whether `first` sorts before `second` under `compare` or else `<`.
private fn __exs_sorts_before(first: Any, second: Any, compare: Fn | None) -> Bool | Error {
    if compare is None {
        ret first < second;
    }
    let order = compare(first, second)?;
    if !(order is Ordering) {
        ret Error("TypeError", "a sort comparison must return an Ordering", order);
    }
    ret order == Ordering::Less;
}

/// Returns the sum of a List's items under `+`, 0 for an empty List.
private fn __exs_list_sum(list: List) -> Any | Error {
    let total = 0;
    for item in list {
        total = (total + item)?;
    }
    ret total;
}

/// Returns the smallest item of a List under `<`, the first of equal ones, or None.
private fn __exs_list_min(list: List) -> Any | Error {
    let result = None;
    let first = true;
    for item in list {
        if first || (item < result)? {
            result = item;
            first = false;
        }
    }
    ret result;
}

/// Returns the largest item of a List under `>`, the first of equal ones, or None.
private fn __exs_list_max(list: List) -> Any | Error {
    let result = None;
    let first = true;
    for item in list {
        if first || (item > result)? {
            result = item;
            first = false;
        }
    }
    ret result;
}

/// Returns a new List of the items with nested Lists replaced by their items, one level deep.
private fn __exs_list_flatten(list: List) -> List {
    let result = [];
    for item in list {
        if item is List {
            result.extend(item);
        } else {
            result.push(item);
        }
    }
    ret result;
}

/// Returns a new List of the items without repetitions under `==`, keeping first occurrences.
private fn __exs_list_unique(list: List) -> List | Error {
    let result = [];
    for item in list {
        if !result.contains(item)? {
            result.push(item);
        }
    }
    ret result;
}
