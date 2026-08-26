//! Function-contract, propagation, and language-error integration tests.

mod support;

use exs_abi::{ErrorSeverity, ExsError, ExsValue};
use support::{execute_source, execute_source_with_inputs};

/// Enforces annotated argument and return types at direct function boundaries.
#[test]
fn validates_function_type_contracts() {
    assert_eq!(
        execute_source(
            r#"
            fn convert(value: Int, offset: Float) -> Float | Error {
                ret value + offset;
            }
            fn main(input) {
                ret convert(input, 0.5);
            }
            "#,
            ExsValue::Int(2),
        ),
        ExsValue::Float(2.5),
    );
    assert_eq!(
        execute_source(
            r#"
            fn echo(value: Any) -> Any {
                ret value;
            }
            fn main(input) {
                ret echo(input);
            }
            "#,
            ExsValue::Object(vec![("enabled".to_owned(), ExsValue::Bool(true))]),
        ),
        ExsValue::Object(vec![("enabled".to_owned(), ExsValue::Bool(true))]),
    );
    assert_error_kind_with_input(
        r#"
        fn identity(value: Int) -> Int | Error {
            ret value;
        }
        fn main(input) {
            ret identity(input);
        }
        "#,
        ExsValue::String("invalid".to_owned()),
        "TypeError",
    );
    assert_error_kind(
        r#"
        fn wrong() -> Int | Error {
            ret "invalid";
        }
        fn main(input) {
            ret wrong();
        }
        "#,
        "TypeError",
    );
}

/// Validates each supplied value against a typed variadic parameter contract.
#[test]
fn validates_variadic_function_type_contracts() {
    let result = execute_source_with_inputs(
        r#"
        fn select(values: Int...) -> Int | Error {
            ret values[1];
        }
        fn main(input) {
            ret select(1, input);
        }
        "#,
        &[ExsValue::String("invalid".to_owned())],
    );
    let ExsValue::Error(error) = result else {
        panic!("source did not return an Error");
    };
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
    assert_eq!(error.kind, "TypeError");
}

/// Preserves direct Error values that are explicitly accepted by a return union.
#[test]
fn accepts_error_values_in_function_type_contracts() {
    let result = execute_source(
        r#"
        fn fail(value: Error) -> Int | Error {
            ret value;
        }
        fn main(input) {
            ret fail(input);
        }
        "#,
        ExsValue::Error(ExsError {
            severity: ErrorSeverity::Recoverable,
            kind: "Expected".to_owned(),
            message: "expected failure".to_owned(),
            data: Box::new(ExsValue::None),
            origin: None,
            trace: Vec::new(),
            cause: None,
        }),
    );
    let ExsValue::Error(error) = result else {
        panic!("typed Error value was not returned");
    };
    assert_eq!(error.kind, "Expected");
}

/// Accepts None when a return union explicitly includes it.
#[test]
fn accepts_none_in_function_type_contracts() {
    assert_eq!(
        execute_source(
            "fn missing() -> None | Int { ret None; } fn main(input) { ret missing(); }",
            ExsValue::None,
        ),
        ExsValue::None,
    );
}

/// Returns a fatal Error when a strict function contract rejects a value.
#[test]
fn returns_a_fatal_error_for_a_strict_function_type_contract_violation() {
    let result = execute_source(
        "fn wrong() -> Int { ret \"invalid\"; } fn main(input) { ret wrong(); }",
        ExsValue::None,
    );
    let ExsValue::Error(error) = result else {
        panic!("strict type contract did not return an Error");
    };
    assert_eq!(error.severity, ErrorSeverity::Fatal);
    assert_eq!(error.kind, "TypeError");
    assert!(error.origin.is_some());
    assert_eq!(error.trace.len(), 2);
}

/// Does not allow a fatal strict-contract Error to be discarded by the caller.
#[test]
fn terminates_after_a_discarded_strict_contract_failure() {
    let result = execute_source(
        r#"
        fn wrong() -> Int { ret "invalid"; }
        fn main(input) -> Int {
            wrong();
            ret 42;
        }
        "#,
        ExsValue::None,
    );
    let ExsValue::Error(error) = result else {
        panic!("discarded strict contract failure did not return an Error");
    };
    assert_eq!(error.severity, ErrorSeverity::Fatal);
    assert_eq!(error.kind, "TypeError");
}

/// Preserves direct values and propagates Error values unchanged with question mark.
#[test]
fn propagates_option_and_result_values() {
    assert_eq!(
        execute_source(
            "fn main(input) { let value = input?; ret value; }",
            ExsValue::Int(42),
        ),
        ExsValue::Int(42),
    );
    let error = ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: "Example".to_owned(),
        message: "example error".to_owned(),
        data: Box::new(ExsValue::None),
        origin: None,
        trace: Vec::new(),
        cause: None,
    });
    assert_eq!(
        execute_source(
            "fn main(input) { let value = input?; ret value; }",
            error.clone()
        ),
        error,
    );
}

/// Converts None propagation into a MissingValue Error.
#[test]
fn converts_none_propagation_to_missing_value_error() {
    let result = execute_source("fn main(input) { ret None?; }", ExsValue::None);
    let ExsValue::Error(error) = result else {
        panic!("None propagation did not return an Error");
    };
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
    assert_eq!(error.kind, "MissingValue");
    assert_eq!(error.data, Box::new(ExsValue::None));
    assert!(error.origin.is_some());
    assert_eq!(error.trace.len(), 1);
}

/// Captures direct generated function frames when an Error is created.
#[test]
fn captures_direct_function_error_trace() {
    let result = execute_source(
        "fn inner(value) { ret None?; } fn main(input) { ret inner(input); }",
        ExsValue::None,
    );
    let ExsValue::Error(error) = result else {
        panic!("missing Error result");
    };
    assert_eq!(error.trace.len(), 2);
    assert_eq!(error.trace[0].function_id, 0);
    assert_eq!(error.trace[1].function_id, 1);
}

/// Tests host-provided Error values through source-level is Error.
#[test]
fn tests_error_values_in_source() {
    let error = ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: "Example".to_owned(),
        message: "example error".to_owned(),
        data: Box::new(ExsValue::None),
        origin: None,
        trace: Vec::new(),
        cause: None,
    });
    assert_eq!(
        execute_source("fn main(input) { ret input is Error; }", error),
        ExsValue::Bool(true),
    );
}

/// Constructs a source-level recoverable Error with its data and source trace intact.
#[test]
fn constructs_errors_with_the_error_builtin() {
    let result = execute_source(
        r#"
        fn main(input) {
            ret Error("ValidationError", "invalid input", { value: input });
        }
        "#,
        ExsValue::Int(42),
    );
    let ExsValue::Error(error) = result else {
        panic!("error builtin did not return an Error");
    };
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
    assert_eq!(error.kind, "ValidationError");
    assert_eq!(error.message, "invalid input");
    assert_eq!(
        error.data,
        Box::new(ExsValue::Object(vec![(
            "value".to_owned(),
            ExsValue::Int(42)
        )]))
    );
    assert!(error.origin.is_some());
    assert_eq!(error.trace.len(), 1);
}

/// Validates the kind and message arguments accepted by the Error builtin.
#[test]
fn validates_error_builtin_string_arguments() {
    assert_error_kind(
        "fn main(input) { ret Error(1, \"message\", input); }",
        "TypeError",
    );
    assert_error_kind(
        "fn main(input) { ret Error(\"Kind\", 1, input); }",
        "TypeError",
    );
}

/// Returns a recoverable Error instead of trapping for invalid dynamic source operations.
#[test]
fn returns_recoverable_errors_for_invalid_dynamic_operations() {
    assert_error_kind("fn main(input) { ret [] - 1; }", "TypeError");
    assert_error_kind("fn main(input) { ret [][0]; }", "IndexError");
    assert_error_kind(
        "fn main(input) { let value = 1; ret value.push(2); }",
        "TypeError",
    );
    assert_error_kind(
        "fn main(input) { for item in 1 { ret item; } ret 0; }",
        "NotIterable",
    );
    assert_error_kind(
        "fn main(input) { let value = {}; for item in value { ret item; } ret 0; }",
        "NotIterable",
    );
}

/// Returns a recoverable Error when a non-Boolean value is used as a condition.
#[test]
fn returns_a_recoverable_error_for_an_invalid_condition() {
    let result = execute_source("fn main(input) { if 1 { ret 2; } ret 3; }", ExsValue::None);
    let ExsValue::Error(error) = result else {
        panic!("invalid condition did not return an Error");
    };
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
    assert_eq!(error.kind, "TypeError");
    assert!(error.origin.is_some());
    assert_eq!(error.trace.len(), 1);
}

/// Executes source and verifies that it returns a recoverable Error of the requested kind.
fn assert_error_kind(source: &str, kind: &str) {
    assert_error_kind_with_input(source, ExsValue::None, kind);
}

/// Executes source and verifies an input-specific recoverable Error result.
fn assert_error_kind_with_input(source: &str, input: ExsValue, kind: &str) {
    let result = execute_source(source, input);
    let ExsValue::Error(error) = result else {
        panic!("source did not return an Error");
    };
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
    assert_eq!(error.kind, kind);
}
