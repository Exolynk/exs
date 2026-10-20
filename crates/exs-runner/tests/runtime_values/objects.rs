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
