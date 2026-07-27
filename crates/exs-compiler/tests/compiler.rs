//! Integration tests for the public Phase-1 compiler API.

use exs_compiler::{CompileOptions, SourceInput, compile};

/// Compiles the required minimal entry point.
#[test]
fn compiles_a_minimal_main_function() {
    let source = "fn main() { ret 42; }";
    let module = compile(
        SourceInput {
            source_id: "test.exs",
            text: source,
        },
        CompileOptions,
    );
    assert!(module.is_ok());
}

/// Reports a missing statement terminator at the source level.
#[test]
fn reports_a_missing_statement_semicolon() {
    let source = "fn main() { let value = 1 ret value; }";
    let result = compile(
        SourceInput {
            source_id: "test.exs",
            text: source,
        },
        CompileOptions,
    );
    let error = match result {
        Ok(_) => panic!("compilation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.diagnostics[0].code, "E0103");
}
