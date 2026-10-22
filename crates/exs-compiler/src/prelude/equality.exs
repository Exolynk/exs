/// Compares two composites of one kind part by part for the built-in `==`: Lists by their
/// elements, Objects by their entries, nominal values by their fields, enums by their payload
/// and Errors by kind, message, data and cause. Parts whose type implements `Eq` or `Compare`
/// compare through it; cycles compare equal where both sides repeat the same pair.
private fn __exs_equal(left: Any, right: Any) -> Bool | Error {
    ret __exs_equal_parts(left, right, []);
}

/// Compares the parts of two composites that the core could not decide, unless the pair is
/// already being compared further up.
private fn __exs_equal_parts(left: Any, right: Any, seen: List) -> Bool | Error {
    for pair in seen {
        if __exs_same(pair[0], left) && __exs_same(pair[1], right) {
            ret true;
        }
    }
    seen.push([left, right]);
    let result = __exs_equal_children(left, right, seen);
    seen.pop();
    ret result;
}

/// Compares the parts of two composites of one kind.
private fn __exs_equal_children(left: Any, right: Any, seen: List) -> Bool | Error {
    let left_parts = __exs_parts(left);
    if left_parts is None {
        for key in left.keys() {
            if !right.has(key) {
                ret false;
            }
            if !__exs_equal_value(left[key], right[key], seen)? {
                ret false;
            }
        }
        ret true;
    }
    let right_parts = __exs_parts(right);
    let index = 0;
    while index < left_parts.length() {
        if !__exs_equal_value(left_parts[index], right_parts[index], seen)? {
            ret false;
        }
        index = index + 1;
    }
    ret true;
}

/// Compares two parts: through the core where it decides, through the left part's `Eq` or
/// `Compare` implementation, or part by part.
private fn __exs_equal_value(left: Any, right: Any, seen: List) -> Bool | Error {
    let equality = __exs_equality(left, right);
    if equality != 2 {
        ret equality == 1;
    }
    if __exs_has_eq(left) {
        ret left == right;
    }
    ret __exs_equal_parts(left, right, seen);
}
