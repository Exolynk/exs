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
