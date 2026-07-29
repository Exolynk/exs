//! Integration tests for executing linked Phase-1 ExS modules.

use exs_abi::ExsValue;
use exs_compiler::{CompileOptions, SourceInput, compile};
use exs_runner::execute;

/// Compiles source text for runner tests.
fn compile_source(source: &str) -> exs_compiler::CompiledModule {
    match compile(
        SourceInput {
            source_id: "test.exs",
            text: source,
        },
        CompileOptions,
    ) {
        Ok(module) => module,
        Err(error) => panic!("compilation failed: {error}"),
    }
}

/// Executes source text and unwraps a successful runner result for assertions.
fn execute_source(source: &str, input: ExsValue) -> ExsValue {
    let compiled = compile_source(source);
    match execute(&compiled.wasm, input) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    }
}

/// Executes an arithmetic result through the linked runtime.
#[test]
fn executes_compiled_integer_program() {
    assert_eq!(
        execute_source(
            "fn main(input) { let value = 40 + 2; ret value; }",
            ExsValue::Null
        ),
        ExsValue::Int(42)
    );
}

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
            ExsValue::Null,
        ),
        ExsValue::Int(42)
    );
}

/// Preserves the inclusive lower integer bound in compiled code.
#[test]
fn executes_the_minimum_exs_integer_literal() {
    assert_eq!(
        execute_source("fn main(input) { ret -36028797018963968; }", ExsValue::Null,),
        ExsValue::Int(exs_value::MIN_INT)
    );
}

/// Links the compiler's committed runtime template into an executable module.
#[test]
fn links_against_the_committed_runtime_template() {
    assert_eq!(
        execute_source("fn main(input) { ret 7 * 6; }", ExsValue::Null),
        ExsValue::Int(42)
    );
}

/// Evaluates boolean equality inside the runtime rather than as a compiler shortcut.
#[test]
fn evaluates_boolean_equality_in_the_runtime() {
    assert_eq!(
        execute_source("fn main(input) { ret true == false; }", ExsValue::Null),
        ExsValue::Bool(false)
    );
}

/// Promotes mixed arithmetic to Float and treats Bool as its numeric 0 or 1 value.
#[test]
fn executes_mixed_bool_integer_and_float_arithmetic() {
    assert_eq!(
        execute_source("fn main(input) { ret true + 2 + 0.5; }", ExsValue::Null),
        ExsValue::Float(3.5)
    );
}

/// Compares Bool, Int, and Float values through the runtime numeric dispatch.
#[test]
fn compares_mixed_numeric_values() {
    assert_eq!(
        execute_source(
            "fn main(input) { ret true == 1.0 && false < 1; }",
            ExsValue::Null,
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
            ExsValue::Null,
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
            ExsValue::Null,
        ),
        ExsValue::List(vec![
            ExsValue::Int(1),
            ExsValue::Int(3),
            ExsValue::Null,
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
            ExsValue::Null,
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
            ExsValue::Null,
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
            ExsValue::Null,
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
            ExsValue::Null,
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
