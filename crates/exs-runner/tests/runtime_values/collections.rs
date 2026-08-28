use super::*;

/// Evaluates list literals, indexed assignments, and mutations.
#[test]
fn executes_list_literals_index_assignment_and_push() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let first = [input, 2];
                let second = first;
                second.push(3);
                first[1] = first[1] + 40;
                ret first;
            }
        "#,
            ExsValue::Int(1),
        ),
        ExsValue::List(vec![ExsValue::Int(1), ExsValue::Int(42), ExsValue::Int(3)]),
    );
}

/// Uses identity equality for Lists and exposes the new length from `push`.
#[test]
fn preserves_list_identity_and_returns_push_length() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let first = [1];
                let alias = first;
                let length = alias.push(2);
                if first == alias && first != [1, 2] {
                    ret length;
                }
                ret 0;
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::Int(2),
    );
}

/// Uses identity equality for Errors.
#[test]
fn preserves_error_identity_equality() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                fn main() -> List {
                    let first_error = Error("First", "first", None);
                    let second_error = Error("Second", "second", None);
                    ret [
                        first_error == first_error,
                        first_error != first_error,
                        first_error == second_error,
                        first_error != second_error,
                    ];
                }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::Bool(true),
            ExsValue::Bool(false),
            ExsValue::Bool(false),
            ExsValue::Bool(true),
        ])
    );
}

/// Uses identity equality for closures.
#[test]
fn preserves_closure_identity_equality() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                fn main() -> List {
                    let first = () => { ret None; };
                    let second = () => { ret None; };
                    ret [
                        first == first,
                        first != first,
                        first == second,
                        first != second,
                    ];
                }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::Bool(true),
            ExsValue::Bool(false),
            ExsValue::Bool(false),
            ExsValue::Bool(true),
        ])
    );
}

/// Decodes a host list for the root input and returns a nested list result.
#[test]
fn passes_list_cbor_input_to_main() {
    assert_eq!(
        execute_source(
            "fn main(input) { input.push([3]); ret input; }",
            ExsValue::List(vec![ExsValue::Int(1), ExsValue::Int(2)]),
        ),
        ExsValue::List(vec![
            ExsValue::Int(1),
            ExsValue::Int(2),
            ExsValue::List(vec![ExsValue::Int(3)]),
        ]),
    );
}

/// Implements every remaining List mutation method with its specified return value.
#[test]
fn executes_remaining_list_operations() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let values = [1, 3];
                values.insert(1, 2);
                let removed = values.remove(0);
                let last = values.pop();
                values.clear();
                let empty = values.pop();
                ret [removed, last, empty, values];
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::List(vec![
            ExsValue::Int(1),
            ExsValue::Int(3),
            ExsValue::None,
            ExsValue::List(vec![]),
        ]),
    );
}

/// Appends one value or chains two Lists without mutating either source List.
#[test]
fn adds_lists_to_values_and_other_lists() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let base = [1];
                let appended = base + input;
                let chained = appended + [3, 4];
                ret [base, appended, chained];
            }
        "#,
            ExsValue::Int(2),
        ),
        ExsValue::List(vec![
            ExsValue::List(vec![ExsValue::Int(1)]),
            ExsValue::List(vec![ExsValue::Int(1), ExsValue::Int(2)]),
            ExsValue::List(vec![
                ExsValue::Int(1),
                ExsValue::Int(2),
                ExsValue::Int(3),
                ExsValue::Int(4),
            ]),
        ]),
    );
}

/// Uses the built-in `Add` method with the same behavior as the `+` operator.
#[test]
fn executes_builtin_add_methods_and_string_scalar_concatenation() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            fn main() -> List {
                let integer = 20;
                let float = 1.5;
                let identifier = "id=";
                let enabled = "enabled=";
                let values = [1];
                ret [
                    integer.add(22),
                    float.add(0.5),
                    identifier.add(42),
                    enabled.add(true),
                    values.add([2, 3]),
                    "left" + "right",
                ];
            }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::Int(42),
            ExsValue::Float(2.0),
            ExsValue::String("id=42".to_owned()),
            ExsValue::String("enabled=true".to_owned()),
            ExsValue::List(vec![ExsValue::Int(1), ExsValue::Int(2), ExsValue::Int(3)]),
            ExsValue::String("leftright".to_owned()),
        ])
    );
}

/// Uses built-in arithmetic methods with the same behavior as their source operators.
#[test]
fn executes_builtin_sub_mul_and_div_methods() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            fn main() -> List {
                let integer = 84;
                let float = 1.5;
                ret [
                    integer.sub(42),
                    integer.mul(2),
                    integer.div(2),
                    float.sub(0.5),
                    float.mul(2.0),
                    float.div(2.0),
                    84 / 2,
                ];
            }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::Int(42),
            ExsValue::Int(168),
            ExsValue::Float(42.0),
            ExsValue::Float(1.0),
            ExsValue::Float(3.0),
            ExsValue::Float(0.75),
            ExsValue::Float(42.0),
        ])
    );
}

/// Preserves object insertion order through literal construction and mutations.
#[test]
fn executes_object_literals_properties_dynamic_keys_and_methods() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let key = "name";
                let profile = { name: input, "role": "admin" };
                let alias = profile;
                alias.score = 42;
                profile[key] = "Ada";
                let keys = profile.keys();
                let values = profile.values();
                if profile.has("score") && keys[0] == "name" && keys[1] == "role" && keys[2] == "score" && values[2] == 42 {
                    ret profile;
                }
                ret {};
            }
        "#,
            ExsValue::Int(1),
        ),
        ExsValue::Object(vec![
            ("name".to_owned(), ExsValue::String("Ada".to_owned())),
            ("role".to_owned(), ExsValue::String("admin".to_owned())),
            ("score".to_owned(), ExsValue::Int(42)),
        ]),
    );
}

/// Uses identity equality and deletion semantics for Objects.
#[test]
fn preserves_object_identity_and_deletion_behavior() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let first = { value: input };
                let alias = first;
                let fresh = { value: input };
                let removed = alias.delete("value");
                if first == alias && first != fresh && removed == input && !first.has("value") {
                    ret first;
                }
                ret fresh;
            }
        "#,
            ExsValue::Int(42),
        ),
        ExsValue::Object(vec![]),
    );
}

/// Decodes a host object for the root input and returns it as an ordered CBOR map.
#[test]
fn passes_object_cbor_input_to_main() {
    assert_eq!(
        execute_source(
            "fn main(input) { input.updated = true; ret input; }",
            ExsValue::Object(vec![(
                "name".to_owned(),
                ExsValue::String("Ada".to_owned())
            )]),
        ),
        ExsValue::Object(vec![
            ("name".to_owned(), ExsValue::String("Ada".to_owned())),
            ("updated".to_owned(), ExsValue::Bool(true)),
        ]),
    );
}
