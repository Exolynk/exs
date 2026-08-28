use super::*;

/// Compiles the required minimal entry point.
#[test]
fn compiles_a_minimal_main_function() {
    let source = "fn main(input) { ret 42; }";
    let module = compile(
        SourceInput {
            source_id: "test.exs",
            text: source,
        },
        CompileOptions::default(),
    );
    assert!(module.is_ok());
}

/// Compiles direct test assertions and their equivalent qualified spellings.
#[test]
fn compiles_direct_and_qualified_test_assertions() {
    let module = compile(
        SourceInput {
            source_id: "standard-assertions.exs",
            text: "fn main() { assert(true); std::test::assert_eq(21 * 2, 42); ret None; }",
        },
        CompileOptions::default(),
    );
    if let Err(error) = module {
        panic!("compilation failed: {error}");
    }
}

/// Rejects the removed one-level qualified assertion spelling.
#[test]
fn rejects_removed_standard_assertion_aliases() {
    let error = match compile(
        SourceInput {
            source_id: "removed-standard-assertion.exs",
            text: "fn main() { std::assert(true); ret None; }",
        },
        CompileOptions::default(),
    ) {
        Ok(_) => panic!("compilation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("unknown function `std::assert`"));
}

/// Rejects declarations that would shadow directly available standard functions.
#[test]
fn rejects_shadowing_standard_assert_functions() {
    let error = match compile(
        SourceInput {
            source_id: "shadow-standard-function.exs",
            text: "fn assert() { ret None; } fn main() { ret None; }",
        },
        CompileOptions::default(),
    ) {
        Ok(_) => panic!("compilation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("`assert` is a reserved standard-library function")
    );
}

/// Compiles the standard Duration factories and suspendable built-in Host sleep operation.
#[test]
fn compiles_a_builtin_host_sleep() {
    let module = compile(
        SourceInput {
            source_id: "timer.exs",
            text: "fn main() { Host::sleep(Duration::seconds(1)); ret None; }",
        },
        CompileOptions::default(),
    );
    assert!(module.is_ok());
}

/// Compiles runner-owned wall-clock and monotonic-time Host operations.
#[test]
fn compiles_builtin_host_time_operations() {
    let module = compile(
        SourceInput {
            source_id: "clock.exs",
            text: "fn main() -> Duration | Error { let now = Host::now(); let elapsed = Host::elapsed(); ret now.duration_since(now)? + elapsed; }",
        },
        CompileOptions::default(),
    );
    assert!(module.is_ok());
}

/// Rejects the removed lower-case host boundary spelling.
#[test]
fn rejects_lowercase_host_call() {
    let module = compile(
        SourceInput {
            source_id: "lowercase-host.exs",
            text: "fn main() { ret host.call(\"echo\"); }",
        },
        CompileOptions::default(),
    );
    assert!(module.is_err());
}

/// Compiles a typed trailing variadic parameter into a packed List call boundary.
#[test]
fn compiles_variadic_function_parameters() {
    let source = "fn total(values: Int...) -> Int { let sum = 0; for value in values { sum = sum + value; } ret sum; } fn main(inputs: Int...) -> Int { ret total(1, 2, 3); }";
    let compiled = match compile(
        SourceInput {
            source_id: "variadic.exs",
            text: source,
        },
        CompileOptions::default(),
    ) {
        Ok(compiled) => compiled,
        Err(error) => panic!("compilation failed: {error}"),
    };
    if let Err(error) = Validator::new().validate_all(&compiled.wasm) {
        panic!("generated Wasm is invalid: {error}");
    }
}

/// Compiles formatted strings with nested source expressions in every supported delimiter form.
#[test]
fn compiles_formatted_strings() {
    let source = r##"
        fn main(name: String) -> String {
            let first = f"Hello {name}: {20 + 1}";
            let second = f#"raw {first}"#;
            ret fd#"
                {second}
            "#;
        }
    "##;
    let compiled = match compile(
        SourceInput {
            source_id: "formatted-strings.exs",
            text: source,
        },
        CompileOptions::default(),
    ) {
        Ok(compiled) => compiled,
        Err(error) => panic!("compilation failed: {error}"),
    };
    if let Err(error) = Validator::new().validate_all(&compiled.wasm) {
        panic!("generated Wasm is invalid: {error}");
    }
}

/// Compiles and formats immutable Bytes literals alongside ordinary Strings.
#[test]
fn compiles_and_formats_bytes_literals() {
    let source = "fn main() -> Bytes { ret b\"hello\\n\"; }";
    let compiled = match compile(
        SourceInput {
            source_id: "bytes.exs",
            text: source,
        },
        CompileOptions::default(),
    ) {
        Ok(compiled) => compiled,
        Err(error) => panic!("compilation failed: {error}"),
    };
    if let Err(error) = Validator::new().validate_all(&compiled.wasm) {
        panic!("generated Wasm is invalid: {error}");
    }
    let formatted = match format(SourceInput {
        source_id: "bytes.exs",
        text: source,
    }) {
        Ok(formatted) => formatted,
        Err(error) => panic!("formatting failed: {error}"),
    };
    assert!(formatted.contains("b\"hello\\n\""));
}

/// Formats every formatted-string delimiter form into reparsable source.
#[test]
fn formats_formatted_strings() {
    let source = r##"fn main(name:String)->String{let first=f"hello {name}";let second=f#"{first}\n{{raw}}"#;ret fd#"
    {second}
    done
"#;}"##;
    let formatted = match format(SourceInput {
        source_id: "format-formatted-strings.exs",
        text: source,
    }) {
        Ok(formatted) => formatted,
        Err(error) => panic!("formatting failed: {error}"),
    };
    assert!(formatted.contains("f\"hello {name}\""));
    assert!(formatted.contains("f#\"{first}\\n{{raw}}\"#"));
    assert!(formatted.contains("fd#\"{second}\ndone\"#"));
    assert!(
        compile(
            SourceInput {
                source_id: "formatted-after-format.exs",
                text: &formatted,
            },
            CompileOptions::default(),
        )
        .is_ok()
    );
}

/// Rejects a rest parameter that is followed by another parameter.
#[test]
fn rejects_non_final_variadic_parameter() {
    let error = match compile(
        SourceInput {
            source_id: "invalid-variadic.exs",
            text: "fn main(values..., tail) { ret tail; }",
        },
        CompileOptions::default(),
    ) {
        Ok(_) => panic!("compilation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(
        error
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0217"),
        "{error:?}"
    );
}
