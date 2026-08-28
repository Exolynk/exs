use super::*;

/// Deeply clones mutable Lists while preserving aliases within the clone only.
#[test]
fn clones_mutable_graphs_without_mutating_the_source() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                fn main() -> List {
                    let shared = [1];
                    let original = [shared, shared];
                    let copy = original.clone();
                    copy[0].push(2);
                    ret [original[0].length(), copy[0].length(), copy[1].length()];
                }
            "#,
            &[],
        ),
        ExsValue::List(vec![ExsValue::Int(1), ExsValue::Int(2), ExsValue::Int(2)])
    );
}

/// Preserves a List cycle while making the clone independent of its source graph.
#[test]
fn clones_cyclic_lists() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                fn main() -> List {
                    let original = [];
                    original.push(original);
                    let copy = original.clone();
                    copy.push(1);
                    ret [original.length(), copy.length(), copy[0].length()];
                }
            "#,
            &[],
        ),
        ExsValue::List(vec![ExsValue::Int(1), ExsValue::Int(2), ExsValue::Int(2)])
    );
}

/// Deeply clones Error data rather than retaining mutable source data by reference.
#[test]
fn clones_error_data_graphs() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                fn main() -> List {
                    let original = Error("Invalid", "example", [1]);
                    let copy = original.clone();
                    let copied_data = copy.data();
                    copied_data.push(2);
                    ret [original.data().length(), copied_data.length()];
                }
            "#,
            &[],
        ),
        ExsValue::List(vec![ExsValue::Int(1), ExsValue::Int(2)])
    );
}

/// Clones Objects and enum payloads without retaining mutable source fields.
#[test]
fn clones_objects_and_enum_payloads() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                enum Container {
                    Items(values),
                }

                impl Container {
                    fn items(self) -> List {
                        ret match self {
                            Container::Items(values) => values,
                        };
                    }
                }

                fn main() -> List {
                    let object = { values: [1] };
                    let object_copy = object.clone();
                    let object_values = object["values"];
                    let copied_object_values = object_copy["values"];
                    copied_object_values.push(2);

                    let enum_value = Container::Items([1]);
                    let enum_copy = enum_value.clone();
                    let enum_values = enum_value.items();
                    let copied_enum_values = enum_copy.items();
                    copied_enum_values.push(2);

                    ret [
                        object_values[0],
                        copied_object_values[1],
                        enum_values[0],
                        copied_enum_values[1],
                    ];
                }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::Int(1),
            ExsValue::Int(2),
            ExsValue::Int(1),
            ExsValue::Int(2),
        ])
    );
}

/// Clones closures into independent capture Cells while retaining their function identity.
#[test]
fn clones_closures_with_independent_captured_cells() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                fn main() -> List {
                    let count = 0;
                    let original = () => {
                        count = count + 1;
                        ret count;
                    };
                    let copy = original.clone();
                    let original_first = original();
                    let copy_first = copy();
                    let original_second = original();
                    let copy_second = copy();
                    ret [original_first, copy_first, original_second, copy_second];
                }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::Int(1),
            ExsValue::Int(1),
            ExsValue::Int(2),
            ExsValue::Int(2),
        ])
    );
}
