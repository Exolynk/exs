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

/// Appends a closure, which makes every function resumable and selects the continuation
/// lowering.
fn resumable(source: &str) -> String {
    format!("{source} fn force_resumable() {{ ret () => {{ ret 1; }}; }}")
}

/// Converts None propagation into a MissingValue Error when the return type admits Errors but
/// not None.
#[test]
fn converts_none_propagation_to_missing_value_error() {
    let source = "fn main(input) -> Int | Error { ret None?; }";
    for source in [source.to_owned(), resumable(source)] {
        let result = execute_source(&source, ExsValue::None);
        let ExsValue::Error(error) = result else {
            panic!("None propagation did not return an Error");
        };
        assert_eq!(error.severity, ErrorSeverity::Recoverable);
        assert_eq!(error.kind, "MissingValue");
        assert_eq!(error.data, Box::new(ExsValue::None));
        assert!(error.origin.is_some());
        assert_eq!(error.trace.len(), 1);
    }
}

/// Returns None from `?` on None when the return type admits None or is omitted, and passes
/// Unit and other values through.
#[test]
fn returns_none_from_none_propagation() {
    let source = r#"
        fn missing(value) -> Int | None { let found = value?; ret found + 1; }
        fn any(value) { let found = value?; ret found; }
        fn done() -> Unit | Error { ret; }
        fn main(input) -> List | Error {
            ret [missing(None), missing(1), any(None), any(2), done()?, done()];
        }
    "#;
    let expected = ExsValue::List(vec![
        ExsValue::None,
        ExsValue::Int(2),
        ExsValue::None,
        ExsValue::Int(2),
        ExsValue::Unit,
        ExsValue::Unit,
    ]);
    for source in [source.to_owned(), resumable(source)] {
        assert_eq!(execute_source(&source, ExsValue::None), expected);
    }
}

/// Fails fatally when `?` meets an Error in a function whose return type admits None but no
/// Errors, keeping the Error as the data of the fatal TypeError.
#[test]
fn escalates_error_propagation_without_error_return_type() {
    let source = r#"
        fn missing(value) -> Int | None { ret value?; }
        fn main(input) -> Int | None | Error { ret missing(input); }
    "#;
    let input = ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: "Example".to_owned(),
        message: "example error".to_owned(),
        data: Box::new(ExsValue::None),
        origin: None,
        trace: Vec::new(),
        cause: None,
    });
    for source in [source.to_owned(), resumable(source)] {
        let ExsValue::Error(error) = execute_source(&source, input.clone()) else {
            panic!("Error propagation did not fail");
        };
        assert_eq!(error.severity, ErrorSeverity::Fatal);
        assert_eq!(error.kind, "TypeError");
        assert_eq!(error.data, Box::new(input.clone()));
    }
}

/// Returns Unit from functions that complete without a value, from `ret;`, and from match
/// blocks.
#[test]
fn returns_unit_without_a_value() {
    let source = r#"
        fn empty() {}
        fn early(input) { if input { ret; } ret 1; }
        fn main(input) -> List {
            ret [empty(), early(true), match input { _ => { let x = 1; } }, Unit is Unit, Unit == None];
        }
    "#;
    let expected = ExsValue::List(vec![
        ExsValue::Unit,
        ExsValue::Unit,
        ExsValue::Unit,
        ExsValue::Bool(true),
        ExsValue::Bool(false),
    ]);
    for source in [source.to_owned(), resumable(source)] {
        assert_eq!(execute_source(&source, ExsValue::None), expected);
    }
}

/// Captures direct generated function frames when an Error is created.
#[test]
fn captures_direct_function_error_trace() {
    let result = execute_source(
        "fn inner(value) -> Int | Error { ret None?; } fn main(input) { ret inner(input); }",
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

/// Tests built-in, nominal, enum, and union contracts without raising a type error.
#[test]
fn tests_every_source_visible_type_category() {
    assert_eq!(
        execute_source(
            r#"
            type Label { text: String }
            enum Outcome { Ready(Int) }

            fn main(input) {
                let label = Label {text: "ready"};
                let outcome = Outcome::Ready(1);
                ret [
                    input is Int,
                    input is String | Int,
                    input is Label,
                    label is Label,
                    label is Object,
                    outcome is Outcome,
                    outcome is Label,
                ];
            }
            "#,
            ExsValue::Int(42),
        ),
        ExsValue::List(vec![
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bool(false),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bool(false),
        ]),
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
        "enum Color { Red } fn main(input) { let value = Color::Red; for item in value { ret item; } ret 0; }",
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

/// Fails `assert` and `assert_eq` with fatal AssertionFailed Errors, in synchronous code and
/// in code that suspends on a host call, and passes when they hold.
#[test]
fn reports_failed_assertions() {
    let assertion = |source: &str| match execute_source(source, ExsValue::None) {
        ExsValue::Error(error) => (error.severity, error.kind, error.message, *error.data),
        other => panic!("expected an assertion Error, received {other:?}"),
    };
    let expected = |message: &str, data: ExsValue| {
        (
            ErrorSeverity::Fatal,
            "AssertionFailed".to_owned(),
            message.to_owned(),
            data,
        )
    };
    assert_eq!(
        assertion("fn main(input) { assert(1 > 2); ret 1; }"),
        expected("assert failed", ExsValue::Bool(false))
    );
    let pair = ExsValue::List(vec![ExsValue::Int(3), ExsValue::String("3".to_owned())]);
    assert_eq!(
        assertion(r#"fn main(input) { assert_eq(1 + 2, "3", "sum"); ret 1; }"#),
        expected("sum", pair.clone())
    );
    assert_eq!(
        assertion(
            r#"fn main(input) { let ignored = Host::call("missing"); assert_eq(1 + 2, "3", "sum"); ret 1; }"#
        ),
        expected("sum", pair)
    );
    assert_eq!(
        execute_source(
            r#"fn main(input) { assert(true); assert_eq("ab", "a" + "b"); let ignored = Host::call("missing"); assert(true, "ok"); ret 7; }"#,
            ExsValue::None
        ),
        ExsValue::Int(7)
    );
}

/// Returns the Error of a failed index or property assignment from the current function,
/// while an Error value can still be stored.
#[test]
fn returns_errors_of_failed_assignments() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            type Point { x: Int }

            fn index_out() { let items = [1]; items[5] = 2; ret "after"; }
            fn field() { let point = Point { x: 1 }; point.y = 2; ret "after"; }
            fn stores_error() { let items = [1]; items[0] = Error("Kind", "message", 1); ret items[0].kind(); }
            fn resumable() {
                Host::sleep(Duration::milliseconds(0));
                let items = [1];
                items[3] = 1;
                ret "after";
            }

            fn main() -> List {
                ret [index_out().kind(), field().kind(), stores_error(), resumable().kind()];
            }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::String("IndexError".to_owned()),
            ExsValue::String("TypeError".to_owned()),
            ExsValue::String("Kind".to_owned()),
            ExsValue::String("IndexError".to_owned()),
        ])
    );
}
