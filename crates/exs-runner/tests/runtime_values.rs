//! Built-in value, collection, iteration, garbage-collection, and clone integration tests.

mod support;

use exs_abi::{ErrorSeverity, ExsValue};
use support::{execute_source, execute_source_with_inputs};

/// Executes calls, assignments, conditionals, and boolean operators.
#[test]
fn executes_calls_assignments_conditionals_and_booleans() {
    assert_eq!(
        execute_source(
            r#"
            fn double(value) { ret value * 2; }
            fn main(input) {
                let value = 20;
                value = double(value);
                if value == 40 && true {
                    ret value + 2;
                } else {
                    ret 0;
                }
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::Int(42)
    );
}

/// Preserves both signed 64-bit integer bounds in compiled source literals.
#[test]
fn executes_signed_64_bit_integer_literals() {
    assert_eq!(
        execute_source(
            "fn main(input) { ret 9223372036854775807; }",
            ExsValue::None,
        ),
        ExsValue::Int(i64::MAX)
    );
    assert_eq!(
        execute_source(
            "fn main(input) { ret -9223372036854775808; }",
            ExsValue::None,
        ),
        ExsValue::Int(i64::MIN)
    );
}

/// Round-trips signed 64-bit runner input and result values.
#[test]
fn round_trips_signed_64_bit_runner_values() {
    for value in [i64::MIN, i64::MAX] {
        assert_eq!(
            execute_source(
                "fn main(input: Int) -> Int { ret input; }",
                ExsValue::Int(value)
            ),
            ExsValue::Int(value)
        );
    }
}

/// Reports overflow only when integer arithmetic exceeds the signed 64-bit range.
#[test]
fn reports_signed_64_bit_integer_overflow() {
    let result = execute_source(
        "fn main(input) -> Error { ret 9223372036854775807 + 1; }",
        ExsValue::None,
    );
    let ExsValue::Error(error) = result else {
        panic!("signed 64-bit overflow did not return an Error");
    };
    assert_eq!(error.kind, "IntOverflowError");
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
}

/// Links the compiler's committed runtime template into an executable module.
#[test]
fn links_against_the_committed_runtime_template() {
    assert_eq!(
        execute_source("fn main(input) { ret 7 * 6; }", ExsValue::None),
        ExsValue::Int(42)
    );
}

/// Returns a fatal Error when the final value cannot cross the runner CBOR boundary.
#[test]
fn returns_fatal_serialization_errors_for_unserializable_final_values() {
    for source in [
        "fn main(input) -> Fn { ret () => { ret 1; }; }",
        r#"
        fn main(input) -> List {
            let cycle = [];
            cycle.push(cycle);
            ret cycle;
        }
        "#,
    ] {
        let result = execute_source(source, ExsValue::None);
        let ExsValue::Error(error) = result else {
            panic!("unserializable final value did not return an Error");
        };
        assert_eq!(error.severity, ErrorSeverity::Fatal);
        assert_eq!(error.kind, "SerializationError");
        assert_eq!(error.data, Box::new(ExsValue::None));
    }
}

/// Evaluates boolean equality inside the runtime rather than as a compiler shortcut.
#[test]
fn evaluates_boolean_equality_in_the_runtime() {
    assert_eq!(
        execute_source("fn main(input) { ret true == false; }", ExsValue::None),
        ExsValue::Bool(false)
    );
}

/// Promotes mixed arithmetic to Float and treats Bool as its numeric 0 or 1 value.
#[test]
fn executes_mixed_bool_integer_and_float_arithmetic() {
    assert_eq!(
        execute_source("fn main(input) { ret true + 2 + 0.5; }", ExsValue::None),
        ExsValue::Float(3.5)
    );
}

/// Compares Bool, Int, and Float values through the runtime numeric dispatch.
#[test]
fn compares_mixed_numeric_values() {
    assert_eq!(
        execute_source(
            "fn main(input) { ret true == 1.0 && false < 1; }",
            ExsValue::None,
        ),
        ExsValue::Bool(true)
    );
}

/// Decodes CBOR input in the runtime and passes it to the one main parameter.
#[test]
fn passes_cbor_input_to_main() {
    assert_eq!(
        execute_source("fn main(input) { ret input + 1; }", ExsValue::Int(41)),
        ExsValue::Int(42)
    );
}

/// Passes ordered CBOR values into typed multi-parameter main declarations.
#[test]
fn passes_multiple_cbor_inputs_to_main() {
    assert_eq!(
        execute_source_with_inputs(
            "fn main(number: Int, offset: Float, name: String) -> String { ret name; }",
            &[
                ExsValue::Int(1),
                ExsValue::Float(0.5),
                ExsValue::String("Ada".to_owned()),
            ],
        ),
        ExsValue::String("Ada".to_owned()),
    );
}

/// Substitutes None for missing main arguments before applying their contracts.
#[test]
fn substitutes_none_for_missing_main_inputs() {
    assert_eq!(
        execute_source_with_inputs("fn main(value: None) -> None { ret value; }", &[]),
        ExsValue::None,
    );
}

/// Rejects entry input arrays that contain more values than main declares.
#[test]
fn rejects_excess_main_inputs_with_a_fatal_arity_error() {
    let result = execute_source_with_inputs("fn main() { ret None; }", &[ExsValue::Int(1)]);
    let ExsValue::Error(error) = result else {
        panic!("excess main input did not return an Error");
    };
    assert_eq!(error.severity, ErrorSeverity::Fatal);
    assert_eq!(error.kind, "ArityError");
    assert_eq!(error.data, Box::new(ExsValue::List(vec![ExsValue::Int(1)])));
}

/// Constructs a UTF-8 literal from a passive Wasm data segment and compares its contents.
#[test]
fn executes_string_literals_and_content_equality() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let name = "Ada\nLovelace \u{1f642}";
                if name == input {
                    ret name;
                } else {
                    ret "unexpected";
                }
            }
        "#,
            ExsValue::String("Ada\nLovelace 🙂".to_owned()),
        ),
        ExsValue::String("Ada\nLovelace 🙂".to_owned())
    );
}

/// Executes the direct Int and Float methods supplied by the standard runtime.
#[test]
fn executes_standard_numeric_methods() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                fn main() -> Float {
                    let integer = -42;
                    let float = -1.5;
                    ret integer.abs() + float.abs().ceil() + float.floor() + float.round();
                }
            "#,
            &[],
        ),
        ExsValue::Float(40.0)
    );
}

/// Executes standard length and emptiness methods for String, List, and Object values.
#[test]
fn executes_standard_collection_methods() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                fn main() -> Int {
                    let text = "🙂";
                    let list = [1, 2];
                    let object = { value: 3 };
                    if text.length() == 1
                        && !text.is_empty()
                        && list.length() == 2
                        && !list.is_empty()
                        && object.length() == 1
                        && !object.is_empty() {
                        ret 4;
                    }
                    ret 0;
                }
            "#,
            &[],
        ),
        ExsValue::Int(4)
    );
}

/// Reads every source-visible Error field through its standard methods.
#[test]
fn executes_standard_error_methods() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                fn main() -> Int {
                    let error = Error("Example", "example message", 7);
                    if error.kind() == "Example"
                        && error.message() == "example message"
                        && error.cause() == None {
                        ret error.data();
                    }
                    ret 0;
                }
            "#,
            &[],
        ),
        ExsValue::Int(7)
    );
}

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

/// Preserves list reference semantics through dynamic index and member dispatch.
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

/// Keeps aliased runtime Objects alive while repeated helper allocations trigger collection.
#[test]
fn preserves_live_aliases_across_allocation_triggered_collection() {
    assert_eq!(
        execute_source(
            r#"
            fn churn(value) {
                let discarded = [value, { value: value }, [value, value]];
                ret 0;
            }
            fn main(input) {
                let object = { value: input };
                let alias = object;
                churn(1);
                churn(2);
                churn(3);
                if alias == object && alias.value == input {
                    ret object;
                }
                ret {};
            }
        "#,
            ExsValue::Int(42),
        ),
        ExsValue::Object(vec![("value".to_owned(), ExsValue::Int(42))]),
    );
}

/// Traces a self-referential List without losing its identity or looping during collection.
#[test]
fn traces_cycles_during_allocation_triggered_collection() {
    assert_eq!(
        execute_source(
            r#"
            fn churn(value) {
                let discarded = [value, value, { value: value }];
                ret 0;
            }
            fn main(input) {
                let cycle = [];
                cycle.push(cycle);
                churn(1);
                churn(2);
                churn(3);
                ret cycle[0] == cycle;
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::Bool(true),
    );
}

/// Evaluates while conditions repeatedly and branches to the nearest loop targets.
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

/// Preserves rooted values while loop allocations repeatedly trigger collection.
#[test]
fn preserves_live_values_during_allocation_heavy_loops() {
    assert_eq!(
        execute_source(
            r#"
            fn main(input) {
                let stable = { value: [input] };
                let alias = stable;
                let count = 0;
                while count < 64 {
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
