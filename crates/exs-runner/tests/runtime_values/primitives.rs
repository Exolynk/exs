use super::*;

/// Evaluates signed 64-bit integer literals.
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

/// Preserves native Bytes across the runner boundary and typed ExS function contracts.
#[test]
fn round_trips_native_bytes() {
    assert_eq!(
        execute_source(
            "fn main(value: Bytes) -> Bytes { ret value; }",
            ExsValue::Bytes(vec![0, 127, 255]),
        ),
        ExsValue::Bytes(vec![0, 127, 255])
    );
}

/// Dispatches a nominal method before the built-in runtime method with the same name.
#[test]
fn dispatches_nominal_is_empty_before_the_runtime_fallback() {
    assert_eq!(
        execute_source(
            "type Toggle { value: Bool } impl Toggle { fn is_empty(self) -> Bool { ret self.value; } } fn main(input) -> Bool { let toggle = Toggle { value: true }; ret toggle.is_empty(); }",
            ExsValue::None,
        ),
        ExsValue::Bool(true)
    );
}

/// Evaluates Bytes literals, construction, methods, indexing, and Iterator-backed for loops.
#[test]
fn evaluates_native_bytes_operations() {
    let source = r#"
        fn main() -> List | Error {
            let prefix = b"A";
            let suffix = Bytes::from_list([0, 255])?;
            let value = prefix.concat(suffix)?;
            let sum = 0;
            for byte in value {
                sum = sum + byte;
            }
            ret [
                value[0],
                value.length(),
                value.to_list(),
                value.slice(1, 3)?,
                b"ok".decode_utf8()?,
                sum,
                Bytes::from_utf8("go")?,
            ];
        }
    "#;
    assert_eq!(
        execute_source_with_inputs(source, &[]),
        ExsValue::List(vec![
            ExsValue::Int(65),
            ExsValue::Int(3),
            ExsValue::List(vec![
                ExsValue::Int(65),
                ExsValue::Int(0),
                ExsValue::Int(255)
            ]),
            ExsValue::Bytes(vec![0, 255]),
            ExsValue::String("ok".to_owned()),
            ExsValue::Int(320),
            ExsValue::Bytes(b"go".to_vec()),
        ])
    );
}

/// Returns documented recoverable errors for invalid Bytes construction and UTF-8 decoding.
#[test]
fn reports_native_bytes_errors() {
    for (source, kind) in [
        (
            "fn main() -> Error { ret Bytes::from_list([256]); }",
            "ValueError",
        ),
        (
            "fn main() -> Error { ret Bytes::from_list([255]).decode_utf8(); }",
            "EncodingError",
        ),
    ] {
        let result = execute_source_with_inputs(source, &[]);
        let ExsValue::Error(error) = result else {
            panic!("Bytes failure did not return an Error");
        };
        assert_eq!(error.kind, kind);
        assert_eq!(error.severity, ErrorSeverity::Recoverable);
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

/// Evaluates checked numeric parsing, conversions, and extended numeric helpers.
#[test]
fn executes_extended_numeric_methods() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                fn main() -> List | Error {
                    let integer = Int::parse("-42")?;
                    let float = Float::parse("3.5")?;
                    ret [
                        integer.signum(),
                        integer.abs(),
                        (12).gcd(18),
                        (12).lcm(18),
                        (2).pow(10),
                        (9).clamp(0, 5),
                        (7).to_float(),
                        float.trunc(),
                        float.fract(),
                        (2.0).pow(3.0),
                        (4.0).sqrt(),
                        (3.5).to_int()?,
                        (1.0).is_finite(),
                    ];
                }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::Int(-1),
            ExsValue::Int(42),
            ExsValue::Int(6),
            ExsValue::Int(36),
            ExsValue::Int(1024),
            ExsValue::Int(5),
            ExsValue::Float(7.0),
            ExsValue::Float(3.0),
            ExsValue::Float(0.5),
            ExsValue::Float(8.0),
            ExsValue::Float(2.0),
            ExsValue::Int(3),
            ExsValue::Bool(true),
        ]),
    );
}

/// Returns recoverable parse and range errors from checked numeric operations.
#[test]
fn reports_extended_numeric_method_errors() {
    for (source, kind) in [
        (
            "fn main() -> Error { ret Int::parse(\"4.2\"); }",
            "ParseError",
        ),
        ("fn main() -> Error { ret (2).pow(-1); }", "ValueError"),
        (
            "fn main() -> Error { ret (9223372036854775807).lcm(2); }",
            "IntOverflowError",
        ),
        (
            "fn main() -> Error { ret (3.0).clamp(4.0, 2.0); }",
            "ValueError",
        ),
    ] {
        let ExsValue::Error(error) = execute_source_with_inputs(source, &[]) else {
            panic!("numeric operation did not return an Error");
        };
        assert_eq!(error.kind, kind);
        assert_eq!(error.severity, ErrorSeverity::Recoverable);
    }
}

/// Evaluates Unicode-aware String and binary Bytes convenience methods.
#[test]
fn executes_extended_string_and_bytes_methods() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                fn main() -> List | Error {
                    let text = "  A🙂b  ";
                    let bytes = Bytes::from_hex("00ff41")?;
                    ret [
                        text.slice(2, 5)?,
                        text.trim(),
                        text.contains("🙂"),
                        text.starts_with("  A"),
                        text.ends_with("  "),
                        "a,b,c".split(","),
                        "Hello".replace("l", "x"),
                        "SS".to_lowercase(),
                        "ha".repeat(3),
                        text.encode_utf8().decode_utf8()?,
                        bytes.to_hex(),
                        bytes.to_base64(),
                        Bytes::from_base64("AP9B")?,
                        bytes.starts_with(Bytes::from_list([0])?),
                        bytes.ends_with(b"A"),
                        bytes.contains(Bytes::from_list([255])?),
                        b"go".repeat(2),
                    ];
                }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::String("A🙂b".to_owned()),
            ExsValue::String("A🙂b".to_owned()),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::List(vec![
                ExsValue::String("a".to_owned()),
                ExsValue::String("b".to_owned()),
                ExsValue::String("c".to_owned()),
            ]),
            ExsValue::String("Hexxo".to_owned()),
            ExsValue::String("ss".to_owned()),
            ExsValue::String("hahaha".to_owned()),
            ExsValue::String("  A🙂b  ".to_owned()),
            ExsValue::String("00ff41".to_owned()),
            ExsValue::String("AP9B".to_owned()),
            ExsValue::Bytes(vec![0, 255, 65]),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bytes(b"gogo".to_vec()),
        ]),
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

/// Executes strict-Float static Math constants and functions.
#[test]
fn executes_static_math_functions() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                fn main() -> List | Error {
                    ret [
                        Math::pi(),
                        Math::tau(),
                        Math::e(),
                        Math::sin(0.0),
                        Math::cos(0.0),
                        Math::tan(0.0),
                        Math::asin(0.0),
                        Math::acos(1.0),
                        Math::atan(0.0),
                        Math::atan2(0.0, 1.0),
                        Math::exp(0.0),
                        Math::ln(1.0),
                        Math::log2(1.0),
                        Math::log10(1.0),
                        Math::hypot(3.0, 4.0),
                    ];
                }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::Float(core::f64::consts::PI),
            ExsValue::Float(core::f64::consts::TAU),
            ExsValue::Float(core::f64::consts::E),
            ExsValue::Float(0.0),
            ExsValue::Float(1.0),
            ExsValue::Float(0.0),
            ExsValue::Float(0.0),
            ExsValue::Float(0.0),
            ExsValue::Float(0.0),
            ExsValue::Float(0.0),
            ExsValue::Float(1.0),
            ExsValue::Float(0.0),
            ExsValue::Float(0.0),
            ExsValue::Float(0.0),
            ExsValue::Float(5.0),
        ]),
    );
}

/// Rejects non-Float Math inputs rather than coercing Int values.
#[test]
fn static_math_functions_require_float_arguments() {
    let ExsValue::Error(error) =
        execute_source_with_inputs("fn main() -> Error { ret Math::sin(1); }", &[])
    else {
        panic!("Math::sin accepted an Int argument");
    };
    assert_eq!(error.kind, "TypeError");
}
