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

/// Iterates half-open and inclusive Range syntax in either direction.
#[test]
fn iterates_range_expressions() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) -> List {
                let values = [];
                for value in 0..3 {
                    values.push(value);
                }
                for value in 3..=1 {
                    values.push(value);
                }
                ret values;
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::List(vec![
            ExsValue::Int(0),
            ExsValue::Int(1),
            ExsValue::Int(2),
            ExsValue::Int(3),
            ExsValue::Int(2),
            ExsValue::Int(1),
        ]),
    );
}

/// Iterates literal Range edge cases through the native Range iterator.
#[test]
fn iterates_literal_range_edge_cases() {
    assert_eq!(
        execute_source(
            r#"
            fn collect_closure() -> List {
                let values = [];
                for value in -2..=-4 {
                    values.push(value);
                }
                ret values;
            }

            fn main(input) -> List {
                let empty = [];
                for value in 5..5 {
                    empty.push(value);
                }
                let single = [];
                for value in 0..=0 {
                    single.push(value);
                }
                let descending = [];
                for value in 3..1 {
                    descending.push(value);
                }
                let bottom = [];
                for value in -9223372036854775807..=-9223372036854775808 {
                    bottom.push(value);
                }
                let broken = [];
                for value in 0..10 {
                    if value == 2 {
                        break;
                    }
                    broken.push(value);
                }
                ret [empty, single, descending, bottom, broken, collect_closure()];
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::List(vec![
            ExsValue::List(vec![]),
            ExsValue::List(vec![ExsValue::Int(0)]),
            ExsValue::List(vec![ExsValue::Int(3), ExsValue::Int(2)]),
            ExsValue::List(vec![
                ExsValue::Int(-9_223_372_036_854_775_807),
                ExsValue::Int(i64::MIN),
            ]),
            ExsValue::List(vec![ExsValue::Int(0), ExsValue::Int(1)]),
            ExsValue::List(vec![
                ExsValue::Int(-2),
                ExsValue::Int(-3),
                ExsValue::Int(-4)
            ]),
        ]),
    );
}

/// Observes Range field mutations made during a loop, including reordered Range objects.
#[test]
fn observes_range_mutations_inside_for_loops() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) -> List {
                let range = 0..3;
                let values = [];
                for value in range {
                    values.push(value);
                    if value == 1 {
                        range.end = 5;
                    }
                }
                let reordered = Range { done: false, inclusive: true, step: 2, end: 4, current: 0 };
                let stepped = [];
                for value in reordered {
                    stepped.push(value);
                }
                ret [values, range.current, range.done, stepped, reordered.done];
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::List(vec![
            ExsValue::List(vec![
                ExsValue::Int(0),
                ExsValue::Int(1),
                ExsValue::Int(2),
                ExsValue::Int(3),
                ExsValue::Int(4),
            ]),
            ExsValue::Int(5),
            ExsValue::Bool(true),
            ExsValue::List(vec![ExsValue::Int(0), ExsValue::Int(2), ExsValue::Int(4)]),
            ExsValue::Bool(true),
        ]),
    );
}

/// Reports the same Error for an invalid literal Range bound inside and outside a for loop.
#[test]
fn reports_invalid_literal_range_bounds_inside_for_loops() {
    let looped = execute_source(
        r#"
        fn main(input) -> Int | Error {
            let total = 0;
            for value in 0..input {
                total = total + 1;
            }
            ret total;
        }
    "#,
        ExsValue::String("3".to_owned()),
    );
    let direct = execute_source(
        "fn main(input) -> Range | Error { ret 0..input; }",
        ExsValue::String("3".to_owned()),
    );
    let (ExsValue::Error(looped), ExsValue::Error(direct)) = (looped, direct) else {
        panic!("invalid Range bound did not return an Error");
    };
    assert_eq!(looped.kind, direct.kind);
    assert_eq!(looped.message, direct.message);
}

/// Iterates explicit stepped Ranges and applies eager helpers to every iterable kind.
#[test]
fn iterates_stepped_ranges_and_eager_iterable_helpers() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) -> List | Error {
                let stepped = Range::step(0, 8, 2)?;
                let through = Range::step_through(6, 0, -3)?;
                let doubled = stepped.map((value) => { ret value * 2; })?;
                ret [doubled, through.to_list()?, "A🙂".count()?, b"AB".collect()?];
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::List(vec![
            ExsValue::List(vec![
                ExsValue::Int(0),
                ExsValue::Int(4),
                ExsValue::Int(8),
                ExsValue::Int(12),
            ]),
            ExsValue::List(vec![ExsValue::Int(6), ExsValue::Int(3), ExsValue::Int(0)]),
            ExsValue::Int(2),
            ExsValue::List(vec![ExsValue::Int(65), ExsValue::Int(66)]),
        ]),
    );
}

/// Advances a Range object in place across loops, matching the prelude `Range::next`.
#[test]
fn resumes_and_exhausts_range_objects_across_for_loops() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) -> List | Error {
                let range = Range::step_through(0, 9, 3)?;
                let values = [];
                for value in range {
                    values.push(value);
                    if value == 3 {
                        break;
                    }
                }
                for value in range {
                    values.push(value);
                }
                for value in range {
                    values.push(-1);
                }
                let top = [];
                for value in 9223372036854775806..=9223372036854775807 {
                    top.push(value);
                }
                ret [values, range.done, range.current, top];
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::List(vec![
            ExsValue::List(vec![
                ExsValue::Int(0),
                ExsValue::Int(3),
                ExsValue::Int(6),
                ExsValue::Int(9),
            ]),
            ExsValue::Bool(true),
            ExsValue::Int(9),
            ExsValue::List(vec![
                ExsValue::Int(9_223_372_036_854_775_806),
                ExsValue::Int(9_223_372_036_854_775_807),
            ]),
        ]),
    );
}

/// Reports the prelude overflow Error when a Range step would leave the Int domain.
#[test]
fn reports_range_step_overflow_inside_for_loops() {
    let ExsValue::Error(error) = execute_source(
        r#"
        fn main(input) -> Int | Error {
            let total = 0;
            for value in Range::step(9223372036854775805, 9223372036854775807, 3)? {
                total = total + 1;
            }
            ret total;
        }
    "#,
        ExsValue::None,
    ) else {
        panic!("overflowing Range step did not return an Error");
    };
    assert!(!error.kind.is_empty());
}

/// Rejects explicit Range steps that cannot produce a valid progression.
#[test]
fn rejects_invalid_explicit_range_steps() {
    for source in [
        "fn main(input) -> Error { ret Range::step(0, 1, 0); }",
        "fn main(input) -> Error { ret Range::step(0, 1, -1); }",
        "fn main(input) -> Error { ret Range::step(1, 0, 1); }",
    ] {
        let ExsValue::Error(error) = execute_source(source, ExsValue::None) else {
            panic!("invalid Range step did not return an Error");
        };
        assert_eq!(error.kind, "ValueError");
    }
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

/// Counts literal Ranges inline for immediate Ints and falls back for other bounds.
#[test]
fn iterates_literal_ranges_inline_and_through_the_fallback() {
    let ints = |values: &[i64]| ExsValue::List(values.iter().copied().map(ExsValue::Int).collect());
    assert_eq!(
        execute_source(
            r#"
            fn tick(log, value) { log.push(value); ret value; }
            fn collect_range(start: Int, end: Int, inclusive: Bool) -> List {
                let values = [];
                if inclusive {
                    for value in start..=end { values.push(value); }
                } else {
                    for value in start..end { values.push(value); }
                }
                ret values;
            }
            fn main(input) {
                let skipped = [];
                for value in 0..10 {
                    if value == 2 { continue; }
                    if value == 6 { break; }
                    skipped.push(value);
                }
                let log = [];
                let rebound = [];
                for value in tick(log, 1)..tick(log, 4) {
                    value = value + 100;
                    rebound.push(value);
                }
                ret [
                    collect_range(5, 0, false),
                    collect_range(3, 1, true),
                    collect_range(0, 0, false),
                    collect_range(2, 2, true),
                    collect_range(2147483645, 2147483647, true),
                    collect_range(-2147483648, -2147483646, false),
                    collect_range(2147483646, 2147483649, false),
                    skipped,
                    log,
                    rebound,
                ];
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::List(vec![
            ints(&[5, 4, 3, 2, 1]),
            ints(&[3, 2, 1]),
            ints(&[]),
            ints(&[2]),
            ints(&[2_147_483_645, 2_147_483_646, 2_147_483_647]),
            ints(&[-2_147_483_648, -2_147_483_647]),
            ints(&[2_147_483_646, 2_147_483_647, 2_147_483_648]),
            ints(&[0, 1, 3, 4, 5]),
            ints(&[1, 4]),
            ints(&[101, 102, 103]),
        ]),
    );
    for source in [
        "fn main(input) { for value in \"a\"..3 { } ret 0; }",
        "fn main(input) { for value in 1.5..3 { } ret 0; }",
    ] {
        let ExsValue::Error(error) = execute_source(source, ExsValue::None) else {
            panic!("a non-Int Range bound did not return an Error");
        };
        assert_eq!(error.kind, "TypeError");
    }
}

/// Keeps Range variable fields identical whether a loop counts inline or advances the Object.
#[test]
fn iterates_range_variables_with_prelude_field_semantics() {
    let source = r#"
        fn exclusive(start: Int, end: Int, stop: Int) -> List {
            let range = start..end;
            let items = [];
            for value in range {
                items.push(value);
                if value == stop { break; }
            }
            let between = [range.current, range.done];
            let rest = [];
            for value in range { rest.push(value); }
            ret [items, between, rest, range.current, range.done];
        }
        fn inclusive(start: Int, end: Int, stop: Int) -> List {
            let range = start..=end;
            let items = [];
            for value in range {
                items.push(value);
                if value == stop { break; }
            }
            let between = [range.current, range.done];
            let rest = [];
            for value in range { rest.push(value); }
            ret [items, between, rest, range.current, range.done];
        }
        fn first_over(limit: Int) -> Int {
            let range = 0..limit;
            for value in range {
                if value > 2 { ret value; }
            }
            ret -1;
        }
        fn shadowed() -> List {
            let range = 0..3;
            let items = [];
            {
                let range = [7, 8];
                for value in range { items.push(value); }
            }
            for value in range { items.push(value); }
            ret [items, range.current, range.done];
        }
        fn observed() -> List {
            let range = 0..4;
            let seen = [];
            for value in range { seen.push(range.current); }
            ret [seen, range.done];
        }
        fn assigned() -> List {
            let range = 0..5;
            range.current = 3;
            let items = [];
            for value in range { items.push(value); }
            ret [items, range.current, range.done];
        }
        fn nested() -> List {
            let range = 0..3;
            let pairs = [];
            for outer in range {
                for inner in range { pairs.push([outer, inner]); }
            }
            ret [pairs, range.current, range.done];
        }
        fn repeated(rounds: Int) -> Int {
            let total = 0;
            let round = 0;
            while round < rounds {
                let range = 0..3;
                for value in range { total = total + value; }
                round = round + 1;
            }
            ret total;
        }
        fn main(input) {
            ret [
                exclusive(0, 3, 99),
                exclusive(0, 3, 2),
                exclusive(0, 3, 1),
                exclusive(3, 0, 99),
                exclusive(3, 0, 1),
                exclusive(0, 0, 99),
                inclusive(0, 2, 99),
                inclusive(0, 2, 2),
                inclusive(0, 2, 0),
                inclusive(2, 0, 99),
                inclusive(2, 2, 99),
                inclusive(2, 2, 2),
                exclusive(2147483645, 2147483647, 99),
                inclusive(2147483645, 2147483647, 2147483647),
                exclusive(-2147483648, -2147483646, 99),
                inclusive(-2147483646, -2147483648, 99),
                exclusive(2147483646, 2147483649, 2147483647),
                first_over(10),
                shadowed(),
                observed(),
                assigned(),
                nested(),
                repeated(4),
            ];
        }
    "#;
    let ExsValue::List(results) = execute_source(source, ExsValue::None) else {
        panic!("Range variable script did not return a List");
    };
    let int = ExsValue::Int;
    let ints = |values: &[i64]| ExsValue::List(values.iter().copied().map(ExsValue::Int).collect());
    let run = |items: &[i64], between: (i64, bool), rest: &[i64], current: i64, done: bool| {
        ExsValue::List(vec![
            ints(items),
            ExsValue::List(vec![int(between.0), ExsValue::Bool(between.1)]),
            ints(rest),
            int(current),
            ExsValue::Bool(done),
        ])
    };
    let expected = vec![
        run(&[0, 1, 2], (3, true), &[], 3, true),
        run(&[0, 1, 2], (3, false), &[], 3, true),
        run(&[0, 1], (2, false), &[2], 3, true),
        run(&[3, 2, 1], (0, true), &[], 0, true),
        run(&[3, 2, 1], (0, false), &[], 0, true),
        run(&[], (0, true), &[], 0, true),
        run(&[0, 1, 2], (2, true), &[], 2, true),
        run(&[0, 1, 2], (2, true), &[], 2, true),
        run(&[0], (1, false), &[1, 2], 2, true),
        run(&[2, 1, 0], (0, true), &[], 0, true),
        run(&[2], (2, true), &[], 2, true),
        run(&[2], (2, true), &[], 2, true),
        run(
            &[2_147_483_645, 2_147_483_646],
            (2_147_483_647, true),
            &[],
            2_147_483_647,
            true,
        ),
        run(
            &[2_147_483_645, 2_147_483_646, 2_147_483_647],
            (2_147_483_647, true),
            &[],
            2_147_483_647,
            true,
        ),
        run(
            &[-2_147_483_648, -2_147_483_647],
            (-2_147_483_646, true),
            &[],
            -2_147_483_646,
            true,
        ),
        run(
            &[-2_147_483_646, -2_147_483_647, -2_147_483_648],
            (-2_147_483_648, true),
            &[],
            -2_147_483_648,
            true,
        ),
        run(
            &[2_147_483_646, 2_147_483_647],
            (2_147_483_648, false),
            &[2_147_483_648],
            2_147_483_649,
            true,
        ),
        int(3),
        ExsValue::List(vec![ints(&[7, 8, 0, 1, 2]), int(3), ExsValue::Bool(true)]),
        ExsValue::List(vec![ints(&[1, 2, 3, 4]), ExsValue::Bool(true)]),
        ExsValue::List(vec![ints(&[3, 4]), int(5), ExsValue::Bool(true)]),
        ExsValue::List(vec![
            ExsValue::List(vec![ints(&[0, 1]), ints(&[0, 2])]),
            int(3),
            ExsValue::Bool(true),
        ]),
        int(12),
    ];
    assert_eq!(results.len(), expected.len());
    for (index, (result, expected)) in results.iter().zip(&expected).enumerate() {
        assert_eq!(result, expected, "Range variable case {index}");
    }
    let ExsValue::Error(error) = execute_source(
        "fn main(input) { let range = \"a\"..3; for value in range { } ret range.current; }",
        ExsValue::None,
    ) else {
        panic!("a non-Int Range bound did not return an Error");
    };
    assert_eq!(error.kind, "TypeError");
}

/// Iterates the `[key, value]` entries of untyped Objects over a snapshot in insertion order.
#[test]
fn iterates_object_entries() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) -> List {
                let object = { b: 2, a: 1 };
                let keys = [];
                for entry in object {
                    keys.push(entry[0]);
                    object.c = 3;
                }
                let total = 0;
                for entry in { x: 1, y: 2 } {
                    total = total + entry[1];
                }
                ret [keys, total, { a: 1 }.map((entry) => { ret entry[0]; })];
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::List(vec![
            ExsValue::List(vec![
                ExsValue::String("b".to_owned()),
                ExsValue::String("a".to_owned()),
            ]),
            ExsValue::Int(3),
            ExsValue::List(vec![ExsValue::String("a".to_owned())]),
        ]),
    );
}
