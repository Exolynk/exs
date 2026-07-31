//! Integration tests for the public Phase-1 compiler API.

use exs_compiler::{CompileOptions, SourceInput, compile, read_debug_info};
use wasmparser::{Parser, Payload, Validator};

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

/// Compiles a frame-backed Host ABI call and its runner-facing control exports.
#[test]
fn compiles_a_resumable_host_call() {
    let compiled = match compile(
        SourceInput {
            source_id: "host-call.exs",
            text: "fn main(input) { ret host.call(\"echo\", input); }",
        },
        CompileOptions::default(),
    ) {
        Ok(compiled) => compiled,
        Err(error) => panic!("compilation failed: {error}"),
    };
    if let Err(error) = Validator::new().validate_all(&compiled.wasm) {
        panic!("generated Wasm is invalid: {error}");
    }
    let exports = Parser::new(0)
        .parse_all(&compiled.wasm)
        .filter_map(Result::ok)
        .filter_map(|payload| match payload {
            Payload::ExportSection(section) => Some(section),
            _ => None,
        })
        .flat_map(|section| section.into_iter().filter_map(Result::ok))
        .map(|export| export.name.to_owned())
        .collect::<Vec<_>>();
    assert!(exports.iter().any(|name| name == "__exs_resume_host"));
    assert!(exports.iter().any(|name| name == "__exs_cancel"));
}

/// Emits compact source positions by default and embeds source text only when requested.
#[test]
fn emits_source_map_and_optional_source_sections() {
    let source = "fn main(input) { ret input + 1; }";
    let compiled = match compile(
        SourceInput {
            source_id: "maps.exs",
            text: source,
        },
        CompileOptions {
            embed_sources: true,
        },
    ) {
        Ok(compiled) => compiled,
        Err(error) => panic!("compilation failed: {error}"),
    };
    let sections = Parser::new(0)
        .parse_all(&compiled.wasm)
        .filter_map(Result::ok)
        .filter_map(|payload| match payload {
            Payload::CustomSection(section) => Some((section.name(), section.data().to_vec())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        sections
            .iter()
            .any(|(name, data)| { *name == "exs.source.map" && data.starts_with(b"EXSMAP2\0") })
    );
    assert!(sections.iter().any(|(name, data)| {
        *name == "exs.sources"
            && data.starts_with(b"EXSSRC1\0")
            && data.ends_with(source.as_bytes())
    }));

    let without_sources = match compile(
        SourceInput {
            source_id: "maps.exs",
            text: source,
        },
        CompileOptions::default(),
    ) {
        Ok(compiled) => compiled,
        Err(error) => panic!("compilation failed: {error}"),
    };
    let has_embedded_source = Parser::new(0)
        .parse_all(&without_sources.wasm)
        .filter_map(Result::ok)
        .any(|payload| {
            matches!(payload, Payload::CustomSection(section) if section.name() == "exs.sources")
        });
    assert!(!has_embedded_source);

    let debug_info = match read_debug_info(&compiled.wasm) {
        Ok(debug_info) => debug_info,
        Err(error) => panic!("could not read debug metadata: {error}"),
    };
    assert_eq!(debug_info.function_name(0), Some("main"));
    assert_eq!(debug_info.source.as_deref(), Some(source));
}

/// Compiles decimal and exponent floating-point literals.
#[test]
fn compiles_floating_point_literals() {
    let module = compile(
        SourceInput {
            source_id: "float.exs",
            text: "fn main(input) { ret 1.0 + 0.25 + 1e2 + 2.5e-3; }",
        },
        CompileOptions::default(),
    );
    assert!(module.is_ok());
}

/// Compiles optional parameter and return union type annotations.
#[test]
fn compiles_function_type_annotations() {
    let module = compile(
        SourceInput {
            source_id: "types.exs",
            text: "fn convert(value: Int, offset: Float) -> Float | Error { ret value + offset; } fn main(input) { ret convert(input, 0.5); }",
        },
        CompileOptions::default(),
    );
    assert!(module.is_ok());
}

/// Compiles nominal Object declarations, implementation methods, and static calls.
#[test]
fn compiles_nominal_types_and_implementations() {
    let module = compile(
        SourceInput {
            source_id: "nominal-types.exs",
            text: "type User { name: String, nickname: String | None, metadata, } impl User { fn display(self) -> String { ret self.name; } fn named(name: String) -> User { ret User { name: name }; } } fn main() -> String { let user = User::named(\"Ada\"); ret user.display(); }",
        },
        CompileOptions::default(),
    );
    assert!(module.is_ok(), "{module:?}");
}

/// Emits a valid Wasm module for nominal Object construction.
#[test]
fn validates_wasm_for_nominal_object_construction() {
    let compiled = match compile(
        SourceInput {
            source_id: "nominal-validation.exs",
            text: "type User { name: String, } fn main(input) -> String { let user = User { name: \"Ada\" }; ret user.name; }",
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

/// Rejects an implementation declaration that shadows one built-in method name.
#[test]
fn rejects_reserved_implementation_method_name() {
    let result = compile(
        SourceInput {
            source_id: "reserved-method.exs",
            text: "type User {} impl User { fn keys(self) { ret None; } } fn main() { ret None; }",
        },
        CompileOptions::default(),
    );
    assert!(result.is_err());
}

/// Rejects a type name that is not in the current built-in type set.
#[test]
fn rejects_an_unknown_function_type() {
    let result = compile(
        SourceInput {
            source_id: "unknown-type.exs",
            text: "fn value(input: Unknown) { ret input; } fn main(input) { ret value(input); }",
        },
        CompileOptions::default(),
    );
    let error = match result {
        Ok(_) => panic!("compilation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.diagnostics[0].code, "E0216");
}

/// Rejects propagation in a function whose declared return type excludes Error.
#[test]
fn rejects_propagation_without_an_error_return_type() {
    let result = compile(
        SourceInput {
            source_id: "strict-return.exs",
            text: "fn value(input) -> Int { ret input?; } fn main(input) { ret value(input); }",
        },
        CompileOptions::default(),
    );
    let error = match result {
        Ok(_) => panic!("compilation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.diagnostics[0].code, "E0218");
}

/// Compiles typed main declarations with multiple parameters.
#[test]
fn compiles_typed_multi_parameter_main() {
    let module = compile(
        SourceInput {
            source_id: "typed-main.exs",
            text: "fn main(first: Int, second: Float) -> Float { ret first + second; }",
        },
        CompileOptions::default(),
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
        CompileOptions::default(),
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
        CompileOptions::default(),
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
        CompileOptions::default(),
    );
    assert!(module.is_ok());
}

/// Compiles both loop forms and their nearest-loop control statements.
#[test]
fn compiles_while_for_break_and_continue_syntax() {
    let module = compile(
        SourceInput {
            source_id: "loops.exs",
            text: "fn main(input) { let value = 0; while value < 3 { value = value + 1; } for item in [1, 2] { if item == 1 { continue; } break; } ret value; }",
        },
        CompileOptions::default(),
    );
    assert!(module.is_ok());
}

/// Rejects a loop-control statement that has no enclosing loop target.
#[test]
fn rejects_break_outside_a_loop() {
    let result = compile(
        SourceInput {
            source_id: "break.exs",
            text: "fn main(input) { break; ret input; }",
        },
        CompileOptions::default(),
    );
    let error = match result {
        Ok(_) => panic!("compilation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.diagnostics[0].code, "E0213");
}

/// Validates the fixed source arity of the Error constructor.
#[test]
fn validates_the_error_constructor_arity() {
    let wrong_arity = compile(
        SourceInput {
            source_id: "error.exs",
            text: "fn main(input) { ret Error(\"Kind\", \"message\"); }",
        },
        CompileOptions::default(),
    );
    let error = match wrong_arity {
        Ok(_) => panic!("wrong Error constructor arity unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.diagnostics[0].code, "E0208");
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
        CompileOptions::default(),
    );
    let error = match result {
        Ok(_) => panic!("compilation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.diagnostics[0].code, "E0103");
}

/// Collects independent syntax errors after recovering at statement boundaries.
#[test]
fn collects_multiple_syntax_diagnostics() {
    let source = r#"
fn main() {
    let value = { name: "Ada"; };
    ret value
}
"#;
    let error = match compile(
        SourceInput {
            source_id: "syntax-errors.exs",
            text: source,
        },
        CompileOptions::default(),
    ) {
        Ok(_) => panic!("compilation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.diagnostics.len(), 2);
    assert!(
        error.diagnostics.iter().all(
            |diagnostic| diagnostic.category == exs_compiler::CompileDiagnosticCategory::Syntax
        )
    );
    let rendered = error.render(source);
    assert!(rendered.contains("error: E0103 (compile syntax)"));
    assert!(rendered.contains("origin: syntax-errors.exs:3:"));
    assert!(rendered.contains("ret value"));
}

/// Collects independent nominal-type and function declaration diagnostics.
#[test]
fn collects_multiple_semantic_diagnostics() {
    let source = r#"
type User {
    name: Unknown,
    name: Missing,
}
fn duplicate(value: Undefined, value: Int) -> ReturnType { ret value; }
fn duplicate(other: AlsoUndefined) { ret other; }
fn main() { ret None; }
"#;
    let error = match compile(
        SourceInput {
            source_id: "semantic-errors.exs",
            text: source,
        },
        CompileOptions::default(),
    ) {
        Ok(_) => panic!("compilation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(error.diagnostics.len() >= 6, "{error:?}");
    assert!(
        error
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.category
                == exs_compiler::CompileDiagnosticCategory::Semantic)
    );
    assert!(
        error
            .diagnostics
            .iter()
            .any(|diagnostic| !diagnostic.related.is_empty())
    );
}

/// Collects independent function-body lowering diagnostics before rejecting the module.
#[test]
fn collects_multiple_function_body_diagnostics() {
    let error = match compile(
        SourceInput {
            source_id: "body-errors.exs",
            text: "fn first() { ret missing_first; } fn second() { ret missing_second; } fn main() { ret None; }",
        },
        CompileOptions::default(),
    ) {
        Ok(_) => panic!("compilation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.diagnostics.len(), 2, "{error:?}");
    assert!(error.diagnostics.iter().all(|diagnostic| {
        diagnostic.category == exs_compiler::CompileDiagnosticCategory::Semantic
    }));
}

/// Collects malformed tokens while continuing to lex later source text.
#[test]
fn collects_multiple_lexical_diagnostics() {
    let error = match compile(
        SourceInput {
            source_id: "lexical-errors.exs",
            text: "@ # fn main() { ret None; }",
        },
        CompileOptions::default(),
    ) {
        Ok(_) => panic!("compilation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.diagnostics.len(), 2, "{error:?}");
    assert!(
        error
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.category
                == exs_compiler::CompileDiagnosticCategory::Lexical)
    );
}

/// Compiles trait contracts, required methods, and inherited default methods.
#[test]
fn compiles_traits_and_default_methods() {
    let module = compile(
        SourceInput {
            source_id: "traits.exs",
            text: r#"
trait Label {
    fn label(self) -> String;
    fn category() -> String { ret "person"; }
}

type User { name: String, }
impl Label for User {
    fn label(self) -> String { ret self.name; }
}
fn main() -> String { ret User::category(); }
"#,
        },
        CompileOptions::default(),
    );
    assert!(module.is_ok(), "{module:?}");
}

/// Accepts a declared trait contract before any nominal type implements it.
#[test]
fn compiles_unimplemented_trait_contracts() {
    let module = compile(
        SourceInput {
            source_id: "unimplemented-trait.exs",
            text: "trait Label { fn label(self); } fn main(value: Label | None) { ret value; }",
        },
        CompileOptions::default(),
    );
    assert!(module.is_ok(), "{module:?}");
}

/// Rejects a type and trait that share one source-visible contract name.
#[test]
fn rejects_type_and_trait_name_collisions() {
    let error = match compile(
        SourceInput {
            source_id: "trait-name-collision.exs",
            text: "type Label {} trait Label {} fn main() { ret None; }",
        },
        CompileOptions::default(),
    ) {
        Ok(_) => panic!("compilation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(error.diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("conflicts with an existing type or trait")
            && !diagnostic.related.is_empty()
    }));
}

/// Rejects a trait implementation that omits a required method.
#[test]
fn rejects_missing_required_trait_methods() {
    let error = match compile(
        SourceInput {
            source_id: "missing-trait-method.exs",
            text: "trait Label { fn label(self); } type User {} impl Label for User {} fn main() { ret None; }",
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
            .any(|diagnostic| diagnostic.code == "E0228")
    );
}

/// Rejects method names made ambiguous by separate trait implementations for one type.
#[test]
fn rejects_duplicate_trait_method_names() {
    let error = match compile(
        SourceInput {
            source_id: "duplicate-trait-method.exs",
            text: r#"
trait First { fn display(self) { ret None; } }
trait Second { fn display(self) { ret None; } }
type User {}
impl First for User {}
impl Second for User {}
fn main() { ret None; }
"#,
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
            .any(|diagnostic| diagnostic.code == "E0226")
    );
}

/// Compiles a zero-parameter main entry point.
#[test]
fn compiles_zero_parameter_main() {
    let module = compile(
        SourceInput {
            source_id: "entry.exs",
            text: "fn main() { ret 42; }",
        },
        CompileOptions::default(),
    );
    assert!(module.is_ok());
}
