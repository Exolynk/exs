use super::*;

/// Builds one host record Object with the given key order.
fn record(entries: &[(&str, i64)]) -> ExsValue {
    ExsValue::Object(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), ExsValue::Int(*value)))
            .collect(),
    )
}

/// Keeps typed Object fields in literal order followed by omitted declared fields.
#[test]
fn keeps_typed_object_field_order() {
    assert_eq!(
        execute_source(
            r#"
            type Point { x: Int, y: Int | None, z: Int | None }
            fn main(input) -> List {
                let point = Point { z: 3, x: 1 };
                point.y = 2;
                point.extra = 4;
                ret [point.keys(), point.y, point.extra];
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::List(vec![
            ExsValue::List(vec![
                ExsValue::String("z".to_owned()),
                ExsValue::String("x".to_owned()),
                ExsValue::String("y".to_owned()),
                ExsValue::String("extra".to_owned()),
            ]),
            ExsValue::Int(2),
            ExsValue::Int(4),
        ]),
    );
}

/// Reads and writes the same property on host records with differing key orders.
#[test]
fn reads_properties_across_differently_ordered_records() {
    assert_eq!(
        execute_source(
            r#"
            fn main(records) -> List {
                let values = [];
                for record in records {
                    values.push(record.b);
                    record.b = record.b + 10;
                    values.push(record.b);
                    values.push(record.missing);
                }
                ret values;
            }
        "#,
            ExsValue::List(vec![
                record(&[("a", 1), ("b", 2)]),
                record(&[("b", 3), ("a", 4)]),
                record(&[("c", 5), ("a", 6), ("b", 7)]),
            ]),
        ),
        ExsValue::List(vec![
            ExsValue::Int(2),
            ExsValue::Int(12),
            ExsValue::None,
            ExsValue::Int(3),
            ExsValue::Int(13),
            ExsValue::None,
            ExsValue::Int(7),
            ExsValue::Int(17),
            ExsValue::None,
        ]),
    );
}

/// Matches enums built by the compiler and supplied by the host alike.
#[test]
fn matches_compiler_and_host_enums() {
    let source = r#"
        enum Shape { Circle(radius: Int), Square(side: Int), Empty }
        fn area(shape: Shape) -> Int {
            ret match shape {
                Shape::Circle(radius) => radius * 3,
                Shape::Square(side) => side * side,
                Shape::Empty => 0,
            };
        }
        fn main(input: Shape) -> List {
            ret [area(input), area(Shape::Square(4)), area(Shape::Empty), Shape::Circle(2)];
        }
    "#;
    assert_eq!(
        execute_source(
            source,
            ExsValue::Enum {
                type_id: "test.exs::Shape".to_owned(),
                variant: "Circle".to_owned(),
                fields: vec![ExsValue::Int(5)],
            },
        ),
        ExsValue::List(vec![
            ExsValue::Int(15),
            ExsValue::Int(16),
            ExsValue::Int(0),
            ExsValue::Enum {
                type_id: "test.exs::Shape".to_owned(),
                variant: "Circle".to_owned(),
                fields: vec![ExsValue::Int(2)],
            },
        ]),
    );
}

/// Lowers `match` identically in direct functions and in resumable continuations.
#[test]
fn matches_identically_in_direct_and_resumable_functions() {
    let source = r#"
        enum Shape { Circle(radius: Int), Square(side: Int), Empty }
        enum Pair { Both(left: Int, right: Int) }
        fn area(shape) -> Int {
            ret match shape {
                Shape::Circle(radius) => radius * 3,
                Shape::Square(side) => side * side,
                Shape::Empty => 0,
            };
        }
        fn early(shape) -> Int {
            let unused = match shape {
                Shape::Square(side) => {
                    if side > 10 { ret -1; }
                },
                _ => 5,
            };
            ret 7;
        }
        fn loop_control(limit: Int) -> List {
            let seen = [];
            let index = 0;
            while index < limit {
                index = index + 1;
                let unused = match Shape::Square(index) {
                    Shape::Square(side) => {
                        if side == 2 { continue; }
                        if side == 5 { break; }
                        seen.push(side);
                    },
                    _ => 0,
                };
            }
            ret seen;
        }
        fn nested(pair: Pair, shape) -> Int {
            ret 100 + match pair {
                Pair::Both(left, right) => left * 10 + right + match shape {
                    Shape::Empty => 1,
                    _ => 2,
                },
            };
        }
        fn main(input) -> List {
            ret [
                area(Shape::Circle(2)),
                area(Shape::Square(4)),
                area(Shape::Empty),
                early(Shape::Square(11)),
                early(Shape::Square(3)),
                early(Shape::Empty),
                loop_control(10),
                nested(Pair::Both(3, 4), Shape::Empty),
                nested(Pair::Both(1, 2), Shape::Circle(1)),
            ];
        }
    "#;
    let ints = |values: &[i64]| ExsValue::List(values.iter().copied().map(ExsValue::Int).collect());
    let expected = ExsValue::List(vec![
        ExsValue::Int(6),
        ExsValue::Int(16),
        ExsValue::Int(0),
        ExsValue::Int(-1),
        ExsValue::Int(7),
        ExsValue::Int(7),
        ints(&[1, 3, 4]),
        ExsValue::Int(135),
        ExsValue::Int(114),
    ]);
    // Any closure makes every function resumable, which selects the continuation lowering.
    let resumable = format!(
        "{source} fn force_resumable() {{ let callback = () => {{ ret 1; }}; ret callback; }}"
    );
    assert_eq!(execute_source(source, ExsValue::None), expected);
    assert_eq!(execute_source(&resumable, ExsValue::None), expected);
    let mismatch = r#"
        enum Shape { Circle(radius: Int), Empty }
        fn main(input) {
            ret match input {
                Shape::Circle(radius) => radius,
                Shape::Empty => 0,
            };
        }
    "#;
    let resumable = format!(
        "{mismatch} fn force_resumable() {{ let callback = () => {{ ret 1; }}; ret callback; }}"
    );
    for source in [mismatch, resumable.as_str()] {
        let ExsValue::Error(error) = execute_source(source, ExsValue::Int(3)) else {
            panic!("matching an Int against enum arms did not return an Error");
        };
        assert_eq!(error.kind, "MatchError");
    }
}

/// Keeps generic indexing behavior for property access on non-Object receivers.
#[test]
fn keeps_indexing_behavior_for_non_object_receivers() {
    let ExsValue::Error(error) =
        execute_source("fn main(input) { ret [1, 2].name; }", ExsValue::None)
    else {
        panic!("property read on a List did not return an Error");
    };
    assert_eq!(error.kind, "IndexError");
    assert_eq!(
        execute_source(
            "fn main(input) { let list = [1]; list.name = 2; ret list; }",
            ExsValue::None,
        ),
        ExsValue::List(vec![ExsValue::Int(1)]),
    );
}

/// Builds Object literals identically whether their keys are distinct or repeated.
#[test]
fn builds_object_literals_with_distinct_and_repeated_keys() {
    let source = r#"
        fn tick(log, value) { log.push(value); ret value; }
        fn main(input) -> List {
            let log = [];
            let unique = { b: tick(log, 1), a: tick(log, 2), c: { inner: tick(log, 3) } };
            let repeated = { a: tick(log, 4), b: tick(log, 5), a: tick(log, 6) };
            let empty = {};
            unique.d = 7;
            unique.a = 8;
            empty.x = 9;
            ret [
                unique.keys(),
                unique.values(),
                unique.c.inner,
                repeated.keys(),
                repeated.values(),
                empty,
                log,
            ];
        }
    "#;
    let strings = |values: &[&str]| {
        ExsValue::List(
            values
                .iter()
                .map(|value| ExsValue::String((*value).to_owned()))
                .collect(),
        )
    };
    let ints = |values: &[i64]| ExsValue::List(values.iter().copied().map(ExsValue::Int).collect());
    let expected = ExsValue::List(vec![
        strings(&["b", "a", "c", "d"]),
        ExsValue::List(vec![
            ExsValue::Int(1),
            ExsValue::Int(8),
            ExsValue::Object(vec![("inner".to_owned(), ExsValue::Int(3))]),
            ExsValue::Int(7),
        ]),
        ExsValue::Int(3),
        strings(&["a", "b"]),
        ints(&[6, 5]),
        ExsValue::Object(vec![("x".to_owned(), ExsValue::Int(9))]),
        ints(&[1, 2, 3, 4, 5, 6]),
    ]);
    // Any closure makes every function resumable, which selects the continuation lowering.
    let resumable = format!("{source} fn force_resumable() {{ ret () => {{ ret 1; }}; }}");
    for source in [source, resumable.as_str()] {
        assert_eq!(execute_source(source, ExsValue::None), expected);
    }
}
