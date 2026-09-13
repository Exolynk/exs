/// Maps every iterable item through a callback, stopping at the first callback Error.
fn __exs_list_map(iterable: Any, callback: Fn) -> List | Error {
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
fn __exs_list_filter(iterable: Any, callback: Fn) -> List | Error {
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
fn __exs_list_find(iterable: Any, callback: Fn) -> Any | None | Error {
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
fn __exs_list_any(iterable: Any, callback: Fn) -> Bool | Error {
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
fn __exs_list_all(iterable: Any, callback: Fn) -> Bool | Error {
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
fn __exs_list_each(iterable: Any, callback: Fn) -> None | Error {
    for item in iterable {
        let result = callback(item);
        if result is Error {
            ret result;
        }
    }
    ret None;
}

/// Folds iterable items from an explicit initial accumulator, stopping at callback Errors.
fn __exs_list_reduce(iterable: Any, initial: Any, callback: Fn) -> Any | Error {
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
fn __exs_iterator_collect(iterable: Any) -> List | Error {
    let result = [];
    for item in iterable {
        result.push(item);
    }
    ret result;
}

/// Counts every item produced by an iterable.
fn __exs_iterator_count(iterable: Any) -> Int | Error {
    let count = 0;
    for item in iterable {
        count = (count + 1)?;
    }
    ret count;
}

/// Collects every iterable item into a new List.
fn __exs_iterator_to_list(iterable: Any) -> List | Error {
    ret __exs_iterator_collect(iterable);
}
