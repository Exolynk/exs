//! Trait protocol, nominal type, enum, and match integration tests.

mod support;

use exs_abi::{ErrorSeverity, ExsValue};
use exs_runner::{ExecutionCancellation, ExecutionLimits, ServerRunner};
use support::{block_on, compile_source, execute_source, execute_source_with_inputs};

/// Resolves Self in inherited trait methods to the concrete implementation target.
#[test]
fn executes_inherited_trait_methods_with_self_annotations() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                trait Keep {
                    fn keep(self, other: Self) -> Self { ret self; }
                }

                type Number { value: Int, }
                impl Keep for Number {}

                fn main() -> Int {
                    let first = Number { value: 42 };
                    let second = Number { value: 7 };
                    let result = first.keep(second);
                    ret result.value;
                }
            "#,
            &[],
        ),
        ExsValue::Int(42)
    );
}

/// Dispatches source `+` through the compiler-owned standard `Add` trait.
#[test]
fn dispatches_standard_add_for_nominal_types() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            type Number { value: Int }

            impl Add for Number {
                fn add(self, other: Any) -> Any {
                    ret Number { value: self.value + other.value };
                }
            }

            fn main() -> Int {
                let left = Number { value: 20 };
                let right = Number { value: 22 };
                ret (left + right).value;
            }
            "#,
            &[],
        ),
        ExsValue::Int(42)
    );
}

/// Dispatches source `+` through a standard `Add` implementation on an enum.
#[test]
fn dispatches_standard_add_for_enums() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            enum Score { Value(value: Int), }

            impl Add for Score {
                fn add(self, other: Any) -> Any {
                    ret match self {
                        Score::Value(left) => match other {
                            Score::Value(right) => Score::Value(left + right),
                        },
                    };
                }
            }

            fn main() -> Int {
                let left = Score::Value(20);
                let right = Score::Value(22);
                ret match left + right { Score::Value(value) => value, };
            }
            "#,
            &[],
        ),
        ExsValue::Int(42)
    );
}

/// Dispatches source arithmetic operators through their standard nominal trait implementations.
#[test]
fn dispatches_standard_sub_mul_and_div_for_nominal_types() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            type Number { value: Int }

            impl Sub for Number {
                fn sub(self, other: Any) -> Any {
                    ret Number { value: self.value - other.value };
                }
            }

            impl Mul for Number {
                fn mul(self, other: Any) -> Any {
                    ret Number { value: self.value * other.value };
                }
            }

            impl Div for Number {
                fn div(self, other: Any) -> Any {
                    ret Number { value: 42 };
                }
            }

            fn main() -> List {
                let left = Number { value: 84 };
                let right = Number { value: 2 };
                ret [
                    (left - right).value,
                    (left * right).value,
                    (left / right).value,
                ];
            }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::Int(82),
            ExsValue::Int(168),
            ExsValue::Int(42)
        ])
    );
}

/// Dispatches equality and ordering operators through one nominal Compare implementation.
#[test]
fn dispatches_standard_compare_for_nominal_types() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            type Version { value: Int }

            impl Compare for Version {
                fn compare(self, other: Any) -> Ordering {
                    if self.value < other.value { ret Ordering::Less; }
                    if self.value > other.value { ret Ordering::Greater; }
                    ret Ordering::Equal;
                }
            }

            fn main() -> List {
                let first = Version { value: 1 };
                let second = Version { value: 2 };
                let same = Version { value: 1 };
                ret [first < second, first <= same, second > first, second >= same, first == same, first != second];
            }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
        ])
    );
}

/// Keeps an inherent method named `add` separate from the source `+` protocol.
#[test]
fn does_not_select_inherent_add_methods_for_source_addition() {
    let result = execute_source_with_inputs(
        r#"
        type Number {}

        impl Number {
            fn add(self, other: Any) -> Any { ret 42; }
        }

        fn main() -> Error {
            let left = Number {};
            let right = Number {};
            ret left + right;
        }
        "#,
        &[],
    );
    let ExsValue::Error(error) = result else {
        panic!("inherent add method was selected by source +");
    };
    assert_eq!(error.kind, "TypeError");
}

/// Routes a suspending standard `Add` implementation through its continuation child frame.
#[test]
fn suspends_through_standard_add_implementations() {
    let compiled = compile_source(
        r#"
        type Number { value: Int }

        impl Add for Number {
            fn add(self, other: Any) -> Any {
                let bonus = Host::call("bonus");
                ret Number { value: self.value + other.value + bonus };
            }
        }

        fn main() -> Int {
            let left = Number { value: 20 };
            let right = Number { value: 21 };
            ret (left + right).value;
        }
        "#,
    );
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    assert!(
        runner
            .registry_mut()
            .fn_async_raw("bonus", |_arguments: Vec<ExsValue>| async move {
                ExsValue::Int(1)
            })
            .is_ok()
    );
    let result = match block_on(runner.execute(
        &compiled.wasm,
        "main",
        &[],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(42));
}

/// Routes a suspending standard `Div` implementation through its continuation child frame.
#[test]
fn suspends_through_standard_div_implementations() {
    let compiled = compile_source(
        r#"
        type Number { value: Float }

        impl Div for Number {
            fn div(self, other: Any) -> Any {
                let divisor = Host::call("divisor");
                ret Number { value: self.value / divisor };
            }
        }

        fn main() -> Float {
            let value = Number { value: 84.0 } / Number { value: 2.0 };
            ret value.value;
        }
        "#,
    );
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    assert!(
        runner
            .registry_mut()
            .fn_async_raw("divisor", |_arguments: Vec<ExsValue>| async move {
                ExsValue::Int(2)
            })
            .is_ok()
    );
    let result = match block_on(runner.execute(
        &compiled.wasm,
        "main",
        &[],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Float(42.0));
}

/// Routes a suspending standard Compare implementation through its continuation child frame.
#[test]
fn suspends_through_standard_compare_implementations() {
    let compiled = compile_source(
        r#"
        type Version { value: Int }

        impl Compare for Version {
            fn compare(self, other: Any) -> Ordering {
                let result = Host::call("ordering");
                if result == 0 { ret Ordering::Less; }
                ret Ordering::Greater;
            }
        }

        fn main() -> Bool {
            ret Version { value: 1 } < Version { value: 2 };
        }
        "#,
    );
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    assert!(
        runner
            .registry_mut()
            .fn_async_raw("ordering", |_arguments: Vec<ExsValue>| async move {
                ExsValue::Int(0)
            })
            .is_ok()
    );
    let result = match block_on(runner.execute(
        &compiled.wasm,
        "main",
        &[],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Bool(true));
}

/// Retains root and child language frames when a child host call returns an Error value.
#[test]
fn traces_errors_through_suspendable_child_frames() {
    let compiled = compile_source(
        r#"
        fn child(value) -> Error { ret Host::call("echo", value) + "invalid"; }
        fn main(input) -> Error { ret child(input)?; }
        "#,
    );
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    assert!(
        runner
            .registry_mut()
            .fn_async_raw("echo", |arguments: Vec<ExsValue>| async move {
                arguments.into_iter().next().unwrap_or(ExsValue::None)
            })
            .is_ok()
    );
    let result = match block_on(runner.execute(
        &compiled.wasm,
        "main",
        &[ExsValue::Int(7)],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    let ExsValue::Error(error) = result else {
        panic!("expected a TypeError");
    };
    assert_eq!(error.kind, "TypeError");
    assert!(error.trace.len() >= 2);
}

/// Constructs nominal Objects, fills omitted optional fields, and dispatches implementation methods.
#[test]
fn executes_nominal_object_construction_and_implementation_methods() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            type User {
                name: String,
                nickname: String | None,
                metadata,
            }
            impl User {
                fn display(self) -> String { ret self.name; }
                fn named(name: String) -> User { ret User { name: name }; }
            }
            fn main() -> String {
                let user = User::named("Ada");
                ret user.display();
            }
        "#,
            &[],
        ),
        ExsValue::String("Ada".to_owned())
    );
}

/// Constructs tagged enum values, dispatches their methods, and returns them through CBOR.
#[test]
fn executes_enum_constructors_implementations_and_cbor_results() {
    let source = r#"
        enum Color {
            Rgb(red: Int, green: Int, blue: Int),
            Transparent,
        }
        trait Rank { fn rank(self) -> Int; }
        impl Color { fn channels(self) -> Int { ret 3; } }
        impl Rank for Color { fn rank(self) -> Int { ret self.channels(); } }
        fn main() -> Color {
            let color = Color::Rgb(255, 0, 128);
            let transparent = Color::Transparent;
            let count = color.rank() + transparent.channels();
            if count == 6 { ret color; }
            ret transparent;
        }
    "#;
    assert_eq!(
        execute_source_with_inputs(source, &[]),
        ExsValue::Enum {
            type_id: "test.exs::Color".to_owned(),
            variant: "Rgb".to_owned(),
            fields: vec![ExsValue::Int(255), ExsValue::Int(0), ExsValue::Int(128)],
        }
    );
}

/// Accepts a tagged enum supplied by a runner for its matching enum contract.
#[test]
fn accepts_cbor_enum_input_for_enum_contract() {
    assert_eq!(
        execute_source(
            "enum Color { Transparent, } fn main(value: Color) -> Color { ret value; }",
            ExsValue::Enum {
                type_id: "test.exs::Color".to_owned(),
                variant: "Transparent".to_owned(),
                fields: vec![],
            },
        ),
        ExsValue::Enum {
            type_id: "test.exs::Color".to_owned(),
            variant: "Transparent".to_owned(),
            fields: vec![],
        }
    );
}

/// Constructs an enum after a host suspension through the continuation lowerer.
#[test]
fn constructs_enum_after_a_host_call() {
    let compiled = compile_source(
        "enum Color { Gray(value: Int), } fn main() -> Color { let value = Host::call(\"value\"); ret Color::Gray(value); }",
    );
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    assert!(
        runner
            .registry_mut()
            .fn_sync_raw("value", |_| ExsValue::Int(42))
            .is_ok()
    );
    let result = match block_on(runner.execute(
        &compiled.wasm,
        "main",
        &[],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(
        result,
        ExsValue::Enum {
            type_id: "test.exs::Color".to_owned(),
            variant: "Gray".to_owned(),
            fields: vec![ExsValue::Int(42)],
        }
    );
}

/// Dispatches enum matches and exposes ordered payload values only to the selected arm.
#[test]
fn executes_exhaustive_enum_match_expressions() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                enum Color {
                    Rgb(red: Int, green: Int, blue: Int),
                    Transparent,
                }
                fn main() -> Int {
                    let color = Color::Rgb(255, 0, 128);
                    ret match color {
                        Color::Rgb(red, green, blue) => red + green + blue,
                        Color::Transparent => 0,
                    };
                }
            "#,
            &[],
        ),
        ExsValue::Int(383)
    );
}

/// Returns directly from a selected statement-block enum match arm.
#[test]
fn returns_from_a_block_enum_match_arm() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                enum Color {
                    Rgb(r: Int, g: Int, b: Int),
                    Name(value),
                    Transparent,
                }
                impl Color {
                    fn as_number(self) -> Int {
                        ret match self {
                            Color::Rgb(r, g, b) => r + g + b,
                            Color::Name(_) => 0,
                            Color::Transparent => { ret -1; },
                        };
                    }
                }
                fn main() -> Int {
                    let color = Color::Transparent;
                    ret color.as_number();
                }
            "#,
            &[],
        ),
        ExsValue::Int(-1)
    );
}

/// Selects a wildcard match arm when no preceding variant arm accepts the value.
#[test]
fn executes_enum_match_wildcard_fallback() {
    assert_eq!(
        execute_source_with_inputs(
            "enum Color { Red, Blue, } fn main() -> Int { let color = Color::Blue; ret match color { Color::Red => 1, _ => 2, }; }",
            &[],
        ),
        ExsValue::Int(2)
    );
}

/// Returns MatchError when a host enum has the expected type identity but an unknown variant.
#[test]
fn returns_match_error_for_unknown_host_enum_variant() {
    let result = execute_source(
        "enum Color { Red, Blue, } fn main(value: Color) -> Int | Error { ret match value { Color::Red => 1, Color::Blue => 2, }; }",
        ExsValue::Enum {
            type_id: "test.exs::Color".to_owned(),
            variant: "Green".to_owned(),
            fields: vec![],
        },
    );
    let ExsValue::Error(error) = result else {
        panic!("unknown enum variant did not return an Error");
    };
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
    assert_eq!(error.kind, "MatchError");
}

/// Resumes a host call performed by the selected enum match arm.
#[test]
fn resumes_host_call_inside_enum_match_arm() {
    let compiled = compile_source(
        "enum Color { Red, Blue, } fn main() -> Int { let color = Color::Blue; ret match color { Color::Red => 1, Color::Blue => Host::call(\"value\"), }; }",
    );
    let mut runner = ServerRunner::new(ExecutionLimits::default());
    assert!(
        runner
            .registry_mut()
            .fn_sync_raw("value", |_| ExsValue::Int(42))
            .is_ok()
    );
    let result = match block_on(runner.execute(
        &compiled.wasm,
        "main",
        &[],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(result, ExsValue::Int(42));
}

/// Executes a nominal Object construction without invoking an implementation method.
#[test]
fn executes_nominal_object_construction() {
    assert_eq!(
        execute_source(
            "type User { name: String, nickname: String | None, } fn main(input) -> String { let user = User { name: \"Ada\" }; ret user.name; }",
            ExsValue::None,
        ),
        ExsValue::String("Ada".to_owned())
    );
}

/// Inserts explicit None entries for omitted `Any` and None-permitting nominal Object fields.
#[test]
fn fills_omitted_nominal_object_fields_with_none() {
    assert_eq!(
        execute_source(
            "type User { name: String, nickname: String | None, metadata, } fn main(input) { let user = User { name: \"Ada\" }; ret user.has(\"nickname\") && user.nickname == None && user.has(\"metadata\"); }",
            ExsValue::None,
        ),
        ExsValue::Bool(true)
    );
}

/// Returns a language TypeError when a nominal Object field violates its declared contract.
#[test]
fn returns_type_error_for_invalid_nominal_object_field() {
    let result = execute_source_with_inputs(
        "type User { name: String, } fn main() -> Error { ret User { name: 1 }; }",
        &[],
    );
    let ExsValue::Error(error) = result else {
        panic!("invalid nominal field did not return an Error");
    };
    assert_eq!(error.kind, "TypeError");
    assert_eq!(error.severity, ErrorSeverity::Recoverable);
}

/// Dispatches a required trait instance method through a trait-typed function parameter.
#[test]
fn dispatches_trait_instance_methods_and_validates_trait_contracts() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            trait Label { fn label(self) -> String; }
            type User { name: String, }
            impl Label for User { fn label(self) -> String { ret self.name; } }
            fn render(value: Label) -> String { ret value.label(); }
            fn main() -> String { ret render(User { name: "Ada" }); }
        "#,
            &[],
        ),
        ExsValue::String("Ada".to_owned())
    );
}

/// Accepts built-in and nominal implementations through the unified `Add` trait contract.
#[test]
fn dispatches_builtin_and_nominal_add_trait_contracts() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            enum Marker { Value, }

            impl Add for Marker {
                fn add(self, value: Any) -> Any { ret self; }
            }

            fn test(input: Add) -> Any {
                ret input.add(2);
            }

            fn main() -> List {
                ret [test(1), test(Marker::Value)];
            }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::Int(3),
            ExsValue::Enum {
                type_id: "test.exs::Marker".to_owned(),
                variant: "Value".to_owned(),
                fields: Vec::new(),
            },
        ])
    );
}

/// Accepts built-in and nominal values through every standard arithmetic trait contract.
#[test]
fn dispatches_builtin_and_nominal_arithmetic_trait_contracts() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            enum Marker { Value, }

            impl Sub for Marker { fn sub(self, other: Any) -> Any { ret self; } }
            impl Mul for Marker { fn mul(self, other: Any) -> Any { ret self; } }
            impl Div for Marker { fn div(self, other: Any) -> Any { ret self; } }

            fn subtract(value: Sub) -> Any { ret value.sub(2); }
            fn multiply(value: Mul) -> Any { ret value.mul(2); }
            fn divide(value: Div) -> Any { ret value.div(2); }

            fn main() -> List {
                ret [subtract(8), multiply(21), divide(84), subtract(Marker::Value)];
            }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::Int(6),
            ExsValue::Int(42),
            ExsValue::Float(42.0),
            ExsValue::Enum {
                type_id: "test.exs::Marker".to_owned(),
                variant: "Value".to_owned(),
                fields: Vec::new(),
            },
        ])
    );
}

/// Preserves universal equality and built-in comparison through the Compare contract.
#[test]
fn dispatches_builtin_compare_trait_contracts() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            fn compare_self(value: Compare) -> Ordering {
                ret value.compare(value);
            }

            fn main() -> List {
                let values = [1];
                ret [
                    20 == 20,
                    20 != 22,
                    "Ada" < "Lin",
                    values == values,
                    match compare_self(20) { Ordering::Equal => true, Ordering::Less => false, Ordering::Greater => false, Ordering::Unordered => false, },
                    match [1].compare([2]) { Ordering::Unordered => true, Ordering::Less => false, Ordering::Equal => false, Ordering::Greater => false, },
                ];
            }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
        ])
    );
}

/// Dispatches a static trait default method through its implementing nominal type.
#[test]
fn dispatches_inherited_static_trait_default_methods() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
            trait Category { fn category() -> String { ret "person"; } }
            type User {}
            impl Category for User {}
            fn main() -> String { ret User::category(); }
        "#,
            &[],
        ),
        ExsValue::String("person".to_owned())
    );
}
