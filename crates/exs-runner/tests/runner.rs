//! Integration tests for executing linked Phase-1 ExS modules.

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

/// Executes an arithmetic result through the linked runtime.
#[test]
fn executes_compiled_integer_program() {
    let compiled = compile_source("fn main() { let value = 40 + 2; ret value; }");
    assert_eq!(
        execute(&compiled.wasm)
            .ok()
            .and_then(|value| value.as_int()),
        Some(42)
    );
}

/// Executes calls, assignments, conditionals, and boolean operators.
#[test]
fn executes_calls_assignments_conditionals_and_booleans() {
    let compiled = compile_source(
        r#"
            fn double(value) { ret value * 2; }
            fn main() {
                let value = 20;
                value = double(value);
                if value == 40 && true {
                    ret value + 2;
                } else {
                    ret 0;
                }
            }
        "#,
    );
    assert_eq!(
        execute(&compiled.wasm)
            .ok()
            .and_then(|value| value.as_int()),
        Some(42)
    );
}

/// Preserves the inclusive lower integer bound in compiled code.
#[test]
fn executes_the_minimum_exs_integer_literal() {
    let compiled = compile_source("fn main() { ret -36028797018963968; }");
    assert_eq!(
        execute(&compiled.wasm)
            .ok()
            .and_then(|value| value.as_int()),
        Some(exs_value::MIN_INT)
    );
}

/// Links the compiler's committed runtime template into an executable module.
#[test]
fn links_against_the_committed_runtime_template() {
    let compiled = compile_source("fn main() { ret 7 * 6; }");
    assert_eq!(
        execute(&compiled.wasm)
            .ok()
            .and_then(|value| value.as_int()),
        Some(42)
    );
}

/// Evaluates boolean equality inside the runtime rather than as a compiler shortcut.
#[test]
fn evaluates_boolean_equality_in_the_runtime() {
    let compiled = compile_source("fn main() { ret true == false; }");
    assert_eq!(
        execute(&compiled.wasm)
            .ok()
            .and_then(|value| value.as_bool()),
        Some(false)
    );
}
