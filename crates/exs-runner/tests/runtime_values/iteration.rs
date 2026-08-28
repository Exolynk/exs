use super::*;

/// Executes while loops with break and continue.
#[test]
fn executes_while_with_break_and_continue() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let value = 0;
                let sum = 0;
                while value < 10 {
                    value = value + 1;
                    if value == 2 {
                        continue;
                    }
                    if value == 6 {
                        break;
                    }
                    sum = sum + value;
                }
                ret sum;
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::Int(13),
    );
}

/// Iterates a List snapshot even when the source List mutates during the loop.
#[test]
fn iterates_a_shallow_list_snapshot() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let values = [1, 2, 3];
                let sum = 0;
                for item in values {
                    if item == 1 {
                        values.push(4);
                    }
                    if item == 2 {
                        continue;
                    }
                    sum = sum + item;
                }
                ret [sum, values];
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::List(vec![
            ExsValue::Int(4),
            ExsValue::List(vec![
                ExsValue::Int(1),
                ExsValue::Int(2),
                ExsValue::Int(3),
                ExsValue::Int(4),
            ]),
        ]),
    );
}

/// Gives closures created in separate for-loop iterations distinct captured bindings.
#[test]
fn preserves_for_loop_closure_captures_per_iteration() {
    assert_eq!(
        execute_source(
            r#"
                fn main(input) -> List {
                    let callbacks = [];
                    for item in [1, 2] {
                        callbacks.push(() => {
                            ret item;
                        });
                    }
                    let first = callbacks[0];
                    let second = callbacks[1];
                    ret [first(), second()];
                }
            "#,
            ExsValue::None,
        ),
        ExsValue::List(vec![ExsValue::Int(1), ExsValue::Int(2)])
    );
}

/// Iterates UTF-8 strings as individual Unicode scalar runtime Strings.
#[test]
fn iterates_string_unicode_scalars() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let scalars = [];
                for scalar in "A🙂B" {
                    scalars.push(scalar);
                }
                ret scalars;
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::List(vec![
            ExsValue::String("A".to_owned()),
            ExsValue::String("🙂".to_owned()),
            ExsValue::String("B".to_owned()),
        ]),
    );
}

/// Preserves rooted values while loop allocations cross the collection threshold.
#[test]
fn preserves_live_values_during_allocation_heavy_loops() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let stable = { value: [input] };
                let alias = stable;
                let count = 0;
                while count < 512 {
                    let discarded = [{ count: count }, [count, count], "discarded"];
                    count = count + 1;
                }
                ret alias.value[0];
            }
        "#,
            ExsValue::Int(42),
        ),
        ExsValue::Int(42),
    );
}

/// Executes direct Option values through the linked runtime.
#[test]
fn executes_direct_option_values() {
    assert_eq!(
        execute_source("fn main(input) { ret input; }", ExsValue::Int(42)),
        ExsValue::Int(42),
    );
    assert_eq!(
        execute_source("fn main(input) { ret None; }", ExsValue::None),
        ExsValue::None,
    );
}
