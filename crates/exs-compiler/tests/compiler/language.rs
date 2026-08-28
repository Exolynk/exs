use super::*;

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

/// Compiles enum variants with ordered payloads and nominal implementations.
#[test]
fn compiles_enums_and_implementations() {
    let module = compile(
        SourceInput {
            source_id: "enums.exs",
            text: "enum Color { Rgb(red: Int, green: Int, blue: Int), Transparent, } trait Rank { fn rank(self) -> Int; } impl Color { fn channels(self) -> Int { ret 3; } } impl Rank for Color { fn rank(self) -> Int { ret self.channels(); } } fn main() -> Int { let color = Color::Rgb(255, 0, 128); let transparent = Color::Transparent; ret color.rank() + transparent.channels(); }",
        },
        CompileOptions::default(),
    );
    assert!(module.is_ok(), "{module:?}");
}

/// Compiles exhaustive enum matches with ordered payload bindings.
#[test]
fn compiles_exhaustive_enum_match_expressions() {
    let module = compile(
        SourceInput {
            source_id: "enum-match.exs",
            text: "enum Color { Rgb(red: Int, green: Int, blue: Int), Transparent, } fn main() -> Int { let color = Color::Rgb(255, 0, 128); ret match color { Color::Rgb(red, green, blue) => red + green + blue, Color::Transparent => 0, }; }",
        },
        CompileOptions::default(),
    );
    assert!(module.is_ok(), "{module:?}");
}

/// Rejects an enum match that omits a variant without a wildcard fallback.
#[test]
fn rejects_non_exhaustive_enum_match_expressions() {
    let error = match compile(
        SourceInput {
            source_id: "enum-match-error.exs",
            text: "enum Color { Red, Blue, } fn main(value: Color) -> Int { ret match value { Color::Red => 1, }; }",
        },
        CompileOptions::default(),
    ) {
        Ok(_) => panic!("non-exhaustive match compiled"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("non-exhaustive match: missing `Color::Blue`")
    );
}

/// Resolves imported enum declarations and their variants through a `use` alias.
#[test]
fn compiles_used_imported_enum_constructors() {
    let mut resolver = TestResolver {
        sources: HashMap::from([(
            "./colors.exs".to_owned(),
            "enum Color { Rgb(red: Int, green: Int, blue: Int), }".to_owned(),
        )]),
    };
    let module = compile_with_resolver(
        SourceInput {
            source_id: "./main.exs",
            text: "import \"./colors.exs\" as colors; use colors::{Color}; fn main() -> Color { ret Color::Rgb(255, 0, 128); }",
        },
        CompileOptions::default(),
        &mut resolver,
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

/// Compiles every built-in type through its optional std namespace qualifier.
#[test]
fn compiles_std_qualified_builtin_type_annotations() {
    let module = compile(
        SourceInput {
            source_id: "std-types.exs",
            text: "fn main(any: std::Any, none: std::None, error: std::Error, boolean: std::Bool, integer: std::Int, float: std::Float, string: std::String, list: std::List, object: std::Object, function: std::Fn) -> std::Int { ret integer; }",
        },
        CompileOptions::default(),
    );
    assert!(module.is_ok(), "{module:?}");
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

/// Compiles raw and dedented hash-delimited multiline string literals.
#[test]
fn compiles_hash_delimited_multiline_string_literals() {
    let module = compile(
        SourceInput {
            source_id: "multiline-string.exs",
            text: r###"
            fn main() -> List {
                let raw = r##"first
  "# remains raw
last"##;
                let dedented = d#"
                    first
                      second
                "#;
                ret [raw, dedented];
            }
            "###,
        },
        CompileOptions::default(),
    );
    assert!(module.is_ok());
}

/// Rejects hash-delimited raw strings whose closing delimiter never appears.
#[test]
fn rejects_unterminated_hash_delimited_string_literals() {
    let error = compile(
        SourceInput {
            source_id: "unterminated-raw-string.exs",
            text: "fn main() { ret r#\"missing closing delimiter; }",
        },
        CompileOptions::default(),
    )
    .expect_err("unterminated raw string compiled");
    assert!(
        error
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0007")
    );
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

/// Compiles the Error constructor when a host call requires continuation lowering.
#[test]
fn compiles_error_constructor_in_a_suspendable_function() {
    let module = compile(
        SourceInput {
            source_id: "suspendable-error.exs",
            text: "fn main() { Host::call(\"noop\"); Error(\"test\", \"lala\", {}); }",
        },
        CompileOptions::default(),
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

/// Rejects non-ASCII spellings in every source position that requires an identifier.
#[test]
fn rejects_non_ascii_identifiers() {
    let error = match compile(
        SourceInput {
            source_id: "unicode-identifiers.exs",
            text: concat!(
                "import \"./math.exs\" as caf\u{00e9};\n",
                "fn main() {\n",
                "    let object = {caf\u{00e9}: 1};\n",
                "    let caf\u{00e9} = object[\"caf\u{00e9}\"];\n",
                "    ret caf\u{00e9};\n",
                "}\n",
            ),
        },
        CompileOptions::default(),
    ) {
        Ok(_) => panic!("compilation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.diagnostics.len(), 4, "{error:?}");
    assert!(error.diagnostics.iter().all(|diagnostic| {
        diagnostic.category == exs_compiler::CompileDiagnosticCategory::Lexical
            && diagnostic.message.contains("identifiers use ASCII letters")
    }));
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

/// Compiles trait signatures that resolve contextual Self to the implementation target.
#[test]
fn compiles_trait_self_annotations() {
    let module = compile(
        SourceInput {
            source_id: "trait-self.exs",
            text: r#"
trait Combine {
    fn combine(self, other: Self) -> Self;
}

type Number { value: Int, }
impl Combine for Number {
    fn combine(self, other: Number) -> Self {
        ret Number { value: self.value + other.value };
    }
}

fn main() -> Int {
    let left = Number { value: 20 };
    let right = Number { value: 22 };
    let result = left.combine(right);
    ret result.value;
}
"#,
        },
        CompileOptions::default(),
    );
    assert!(module.is_ok(), "{module:?}");
}

/// Compiles the compiler-owned `Add` contract and permits it in type annotations.
#[test]
fn compiles_standard_add_trait_implementations() {
    let module = compile(
        SourceInput {
            source_id: "add.exs",
            text: "type Number { value: Int } impl std::Add for Number { fn add(self, other: Any) -> Any { ret Number { value: self.value + other.value }; } } fn identity(value: Add) -> Any { ret value; } fn main() -> Int { ret (Number { value: 20 } + Number { value: 22 }).value; }",
        },
        CompileOptions::default(),
    );
    if let Err(error) = module {
        panic!("standard Add did not compile: {error}");
    }
}

/// Compiles the standard arithmetic trait contracts and division source syntax.
#[test]
fn compiles_standard_sub_mul_and_div_trait_implementations() {
    let module = compile(
        SourceInput {
            source_id: "arithmetic.exs",
            text: "type Number { value: Float } impl std::Sub for Number { fn sub(self, other: Any) -> Any { ret Number { value: self.value - other.value }; } } impl std::Mul for Number { fn mul(self, other: Any) -> Any { ret Number { value: self.value * other.value }; } } impl std::Div for Number { fn div(self, other: Any) -> Any { ret Number { value: self.value / other.value }; } } fn identity(value: Sub | Mul | Div) -> Any { ret value; } fn main() -> Float { ret (Number { value: 84.0 } / Number { value: 2.0 }).value; }",
        },
        CompileOptions::default(),
    );
    if let Err(error) = module {
        panic!("standard arithmetic traits did not compile: {error}");
    }
}

/// Compiles the standard Compare contract and compiler-owned Ordering enum.
#[test]
fn compiles_standard_compare_trait_and_ordering_enum() {
    let module = compile(
        SourceInput {
            source_id: "compare.exs",
            text: "type Version { value: Int } impl std::Compare for Version { fn compare(self, other: Any) -> Ordering { if self.value < other.value { ret Ordering::Less; } if self.value > other.value { ret Ordering::Greater; } ret Ordering::Equal; } } fn identity(value: Compare) -> Ordering { ret value.compare(value); } fn main() -> Bool { ret Version { value: 1 } < Version { value: 2 }; }",
        },
        CompileOptions::default(),
    );
    if let Err(error) = module {
        panic!("standard Compare did not compile: {error}");
    }
}

/// Rejects standard `Add` implementations that do not meet its fixed method contract.
#[test]
fn rejects_invalid_standard_add_signature() {
    let error = match compile(
        SourceInput {
            source_id: "invalid-add.exs",
            text: "type Number {} impl Add for Number { fn add(self, other: Int) -> Int { ret 0; } } fn main() { ret None; }",
        },
        CompileOptions::default(),
    ) {
        Ok(_) => panic!("invalid Add implementation compiled"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("fn add(self, other: Any) -> Any")
    );
}

/// Reserves the compiler-owned standard `Add` name from source trait declarations.
#[test]
fn rejects_standard_add_trait_redeclaration() {
    let error = match compile(
        SourceInput {
            source_id: "redeclare-add.exs",
            text: "trait Add { fn add(self, other: Any) -> Any; } fn main() { ret None; }",
        },
        CompileOptions::default(),
    ) {
        Ok(_) => panic!("source Add trait declaration compiled"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("duplicate or reserved trait `Add`")
    );
}

/// Rejects Self in a function annotation that has no trait implementation context.
#[test]
fn rejects_self_outside_trait_context() {
    let error = match compile(
        SourceInput {
            source_id: "invalid-self.exs",
            text: "fn main(value: Self) { ret value; }",
        },
        CompileOptions::default(),
    ) {
        Ok(_) => panic!("compilation unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(error.diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "`Self` is valid only in trait declarations and trait implementations"
    }));
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

/// Treats `pub` as an ordinary identifier after removing it from the language keywords.
#[test]
fn treats_pub_as_an_unreserved_identifier() {
    let module = compile(
        SourceInput {
            source_id: "pub-identifier.exs",
            text: "fn pub() -> Int { ret 42; } fn main() -> Int { ret pub(); }",
        },
        CompileOptions::default(),
    );
    assert!(module.is_ok(), "{module:?}");
}

/// Rejects the removed `pub fn` modifier at the module boundary.
#[test]
fn rejects_removed_pub_function_modifier() {
    let error = match compile(
        SourceInput {
            source_id: "pub-modifier.exs",
            text: "pub fn main() { ret None; }",
        },
        CompileOptions::default(),
    ) {
        Ok(_) => panic!("removed `pub` modifier compiled"),
        Err(error) => error,
    };
    assert!(
        error.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "E0100" && diagnostic.message.contains("`fn`")
        })
    );
}
