//! Integration tests for the public Phase-1 compiler API.

use exs_compiler::{CompileOptions, SourceInput, compile};

/// Compiles the required minimal entry point.
#[test]
fn compiles_a_minimal_main_function() {
    let source = "fn main(input) { ret 42; }";
    let module = compile(
        SourceInput {
            source_id: "test.exs",
            text: source,
        },
        CompileOptions,
    );
    assert!(module.is_ok());
}

/// Compiles decimal and exponent floating-point literals.
#[test]
fn compiles_floating_point_literals() {
    let module = compile(
        SourceInput {
            source_id: "float.exs",
            text: "fn main(input) { ret 1.0 + 0.25 + 1e2 + 2.5e-3; }",
        },
        CompileOptions,
    );
    assert!(module.is_ok());
}

/// Compiles decoded string escapes into compiler-owned passive data segments.
#[test]
fn compiles_utf8_string_literals() {
    let module = compile(
        SourceInput {
            source_id: "string.exs",
            text: r#"fn main(input) { ret "Hi \u{1f642}\n"; }"#,
        },
        CompileOptions,
    );
    assert!(module.is_ok());
}

/// Compiles list literals, dynamic index expressions, and a member call.
#[test]
fn compiles_list_syntax() {
    let module = compile(
        SourceInput {
            source_id: "list.exs",
            text: "fn main(input) { let values = [input, 2]; values.push(3); values[1] = 4; ret values[0]; }",
        },
        CompileOptions,
    );
    assert!(module.is_ok());
}

/// Compiles object literals, dot properties, dynamic keys, and member calls.
#[test]
fn compiles_object_syntax() {
    let module = compile(
        SourceInput {
            source_id: "object.exs",
            text: "fn main(input) { let key = \"name\"; let value = { name: input, \"role\": \"admin\" }; value[key] = \"Ada\"; value.score = 42; ret value.has(\"score\"); }",
        },
        CompileOptions,
    );
    assert!(module.is_ok());
}

/// Reports a missing statement terminator at the source level.
#[test]
fn reports_a_missing_statement_semicolon() {
    let source = "fn main(input) { let value = 1 ret value; }";
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

/// Rejects the obsolete zero-parameter Phase-1 entry point.
#[test]
fn requires_one_main_parameter() {
    let result = compile(
        SourceInput {
            source_id: "entry.exs",
            text: "fn main() { ret 42; }",
        },
        CompileOptions,
    );
    let error = match result {
        Ok(_) => panic!("compilation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.diagnostics[0].code, "E0203");
}
