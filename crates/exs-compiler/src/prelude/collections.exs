/// Maps every List item through a callback, stopping at the first callback Error.
fn __exs_list_map(values: List, callback: Fn) -> List | Error {
    let result = [];
    for item in values {
        let mapped = callback(item);
        if mapped is Error {
            ret mapped;
        }
        result.push(mapped);
    }
    ret result;
}

/// Keeps every List item whose callback result is true, stopping at callback Errors.
fn __exs_list_filter(values: List, callback: Fn) -> List | Error {
    let result = [];
    for item in values {
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

/// Returns the first List item whose callback result is true, or None when absent.
fn __exs_list_find(values: List, callback: Fn) -> Any | None | Error {
    for item in values {
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

/// Returns whether any List item produces true, stopping at callback Errors.
fn __exs_list_any(values: List, callback: Fn) -> Bool | Error {
    for item in values {
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

/// Returns whether every List item produces true, stopping at callback Errors.
fn __exs_list_all(values: List, callback: Fn) -> Bool | Error {
    for item in values {
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

/// Invokes a callback for every List item, stopping at the first callback Error.
fn __exs_list_each(values: List, callback: Fn) -> None | Error {
    for item in values {
        let result = callback(item);
        if result is Error {
            ret result;
        }
    }
    ret None;
}

/// Folds List items from an explicit initial accumulator, stopping at callback Errors.
fn __exs_list_reduce(values: List, initial: Any, callback: Fn) -> Any | Error {
    let accumulator = initial;
    for item in values {
        let result = callback(accumulator, item);
        if result is Error {
            ret result;
        }
        accumulator = result;
    }
    ret accumulator;
}
