//! Integration tests for the public Phase-1 compiler API.

use std::collections::HashMap;

use exs_compiler::{
    CompileOptions, ModuleResolver, ResolvedSource, SourceInput, compile, compile_with_resolver,
    document_with_resolver, format, read_debug_info,
};
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

/// Formats valid source into a stable, reparsable canonical layout.
#[test]
fn formats_source_into_canonical_layout() {
    let source = "import \"./math.exs\" as math;use math::{add as plus,Point};fn main(value:Int)->Int{let point=Point{value:value};if value>0{ret plus(point.value,1);}else{ret 0;}}";
    let formatted = match format(SourceInput {
        source_id: "format.exs",
        text: source,
    }) {
        Ok(formatted) => formatted,
        Err(error) => panic!("formatting failed: {error}"),
    };
    assert_eq!(
        formatted,
        "import \"./math.exs\" as math;\nuse math::{add as plus, Point};\n\nfn main(value: Int) -> Int {\n    let point = Point {value: value};\n    if value > 0 {\n        ret plus(point.value, 1);\n    }\n    else {\n        ret 0;\n    }\n}\n"
    );
    let reformatted = match format(SourceInput {
        source_id: "format.exs",
        text: &formatted,
    }) {
        Ok(formatted) => formatted,
        Err(error) => panic!("reformatting failed: {error}"),
    };
    assert_eq!(reformatted, formatted);
}

/// Rejects malformed source instead of attempting a best-effort rewrite.
#[test]
fn formatter_returns_syntax_diagnostics() {
    let error = match format(SourceInput {
        source_id: "format-error.exs",
        text: "fn main( {",
    }) {
        Ok(_) => panic!("malformed source was formatted"),
        Err(error) => error,
    };
    assert!(!error.diagnostics.is_empty());
}

/// Formats statement-block match arms in their compact expression position.
#[test]
fn formats_match_block_arms() {
    let formatted = match format(SourceInput {
        source_id: "format-match.exs",
        text: "enum Color{Transparent,}fn main(value:Color)->Int{ret match value{Color::Transparent=>{ret -1;}};}",
    }) {
        Ok(formatted) => formatted,
        Err(error) => panic!("formatting failed: {error}"),
    };
    assert_eq!(
        formatted,
        "enum Color {\n    Transparent,\n}\n\nfn main(value: Color) -> Int {\n    ret match value { Color::Transparent => { ret -1; } };\n}\n"
    );
}

/// Generates language and API pages for reachable imported modules.
#[test]
fn generates_markdown_api_documentation() {
    let mut resolver = TestResolver {
        sources: HashMap::from([(
            "./math.exs".to_owned(),
            "/// Adds two integers.\nfn add(left: Int, right: Int) -> Int { ret left + right; }\n\n/// A coordinate.\ntype Point { value: Int }\n\nimpl Point { fn coordinate(self) -> Int { ret self.value; } }\n\n/// A display color.\nenum Color { Rgb(red: Int, green: Int, blue: Int), Transparent, }\n\nimpl Color { fn channels(self) -> Int { ret 3; } }".to_owned(),
        )]),
    };
    let documentation = match document_with_resolver(
        SourceInput {
            source_id: "./main.exs",
            text: "import \"./math.exs\" as math;\n/// Runs the program.\nfn main() -> Int { ret math::add(20, 22); }",
        },
        &mut resolver,
    ) {
        Ok(documentation) => documentation,
        Err(error) => panic!("documentation generation failed: {error}"),
    };
    assert!(documentation.index.contains("## Language"));
    assert!(
        documentation
            .index
            .contains("[`std`](modules/std/index.md)")
    );
    let main = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/00-main/index.md")
        .unwrap_or_else(|| panic!("missing main module page"));
    assert!(main.markdown.contains("[module](../01-math/index.md)"));
    let math = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/01-math/index.md")
        .unwrap_or_else(|| panic!("missing math module index"));
    assert!(math.markdown.starts_with("# Module `./math.exs`"));
    assert!(math.markdown.contains("[`add`](fn/add.md)"));
    assert!(math.markdown.contains("[`Point`](types/point.md)"));
    assert!(math.markdown.contains("[`Color`](enums/color.md)"));
    assert!(!math.markdown.contains("## Implementations"));
    let add = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/01-math/fn/add.md")
        .unwrap_or_else(|| panic!("missing function page"));
    assert!(add.markdown.contains("Adds two integers."));
    let point = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/01-math/types/point.md")
        .unwrap_or_else(|| panic!("missing Point type page"));
    assert!(point.markdown.contains("## Implemented Methods"));
    assert!(point.markdown.contains("coordinate"));
    let color = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/01-math/enums/color.md")
        .unwrap_or_else(|| panic!("missing Color enum page"));
    assert!(
        color
            .markdown
            .contains("Rgb(red: Int, green: Int, blue: Int)")
    );
    assert!(color.markdown.contains("## Implemented Methods"));
    let host_call = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/fn/host-call.md")
        .unwrap_or_else(|| panic!("missing std host.call page"));
    assert!(host_call.markdown.contains("host.call(name, arguments...)"));
    assert!(host_call.markdown.contains("## Usage"));
    assert!(host_call.markdown.contains("fn main()"));
    let standard = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/index.md")
        .unwrap_or_else(|| panic!("missing std module page"));
    assert!(!standard.markdown.contains("[`type`]"));
    assert!(!standard.markdown.contains("[`len`]"));
    assert!(standard.markdown.contains("`std::` qualifier"));
    assert!(standard.markdown.contains("[`Add`](traits/add.md)"));
    assert!(documentation.pages.iter().all(|page| !matches!(
        page.path.as_str(),
        "modules/std/fn/type.md" | "modules/std/fn/len.md"
    )));
    let error = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/types/error.md")
        .unwrap_or_else(|| panic!("missing std Error type page"));
    assert!(error.markdown.contains("## Constructor"));
    let any = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/types/any.md")
        .unwrap_or_else(|| panic!("missing std Any page"));
    assert!(any.markdown.contains("clone() -> Any | Error"));
    assert!(any.markdown.contains("preserving aliases and cycles"));
    assert!(error.markdown.contains("Error(kind, message, data)"));
    assert!(error.markdown.contains("### `kind() -> String`"));
    assert!(error.markdown.contains("machine-readable category"));
    assert!(error.markdown.contains("fn main()"));
    assert!(
        documentation
            .pages
            .iter()
            .all(|page| page.path != "modules/std/fn/error.md")
    );
    let list = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/types/list.md")
        .unwrap_or_else(|| panic!("missing standard List page"));
    assert!(list.markdown.contains("### `push(value) -> Int`"));
    assert!(list.markdown.contains("### `length() -> Int`"));
    assert!(list.markdown.contains("mutates the existing List"));
    assert!(list.markdown.contains("let items = [\"Ada\"];"));
    let float = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/types/float.md")
        .unwrap_or_else(|| panic!("missing standard Float page"));
    assert!(float.markdown.contains("### `round() -> Float`"));
    let add_trait = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/traits/add.md")
        .unwrap_or_else(|| panic!("missing std Add trait page"));
    assert!(
        add_trait
            .markdown
            .contains("fn add(self, other: Any) -> Any;")
    );
    assert!(
        add_trait
            .markdown
            .contains("Vector { value: 20 } + Vector { value: 22 }")
    );
    assert!(add_trait.markdown.contains("## Built-in Implementations"));
    assert!(
        add_trait
            .markdown
            .contains("[`std::String`](../types/string.md)")
    );
    let string = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/types/string.md")
        .unwrap_or_else(|| panic!("missing standard String page"));
    assert!(string.markdown.contains("## Implemented Methods"));
    assert!(string.markdown.contains("Trait [`Add`](../traits/add.md)"));
    assert!(string.markdown.contains("#### `add`"));
    assert!(
        string
            .markdown
            .contains("Adds the receiver to the evaluated `other` operand.")
    );
}

/// Links a standard trait implementation and inherits its method documentation on an enum page.
#[test]
fn documents_standard_add_implementations_on_nominal_pages() {
    let mut resolver = TestResolver {
        sources: HashMap::new(),
    };
    let documentation = match document_with_resolver(
        SourceInput {
            source_id: "./add.exs",
            text: "enum Abc { A, B, } impl Add for Abc { fn add(self, other: Any) -> Any { ret self; } } fn main() { ret Abc::A; }",
        },
        &mut resolver,
    ) {
        Ok(documentation) => documentation,
        Err(error) => panic!("documentation generation failed: {error}"),
    };
    let abc = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/00-add/enums/abc.md")
        .unwrap_or_else(|| panic!("missing enum page"));
    assert!(
        abc.markdown
            .contains("Trait [`Add`](../../std/traits/add.md)")
    );
    assert!(
        abc.markdown
            .contains("Adds the receiver to the evaluated `other` operand.")
    );
}

/// Prefers an implementation method's own documentation over inherited trait documentation.
#[test]
fn prefers_implementation_documentation_over_standard_add_documentation() {
    let mut resolver = TestResolver {
        sources: HashMap::new(),
    };
    let documentation = match document_with_resolver(
        SourceInput {
            source_id: "./custom-add.exs",
            text: "type Counter {} impl Add for Counter {\n/// Adds one application-specific count.\nfn add(self, other: Any) -> Any { ret self; } } fn main() { ret Counter {}; }",
        },
        &mut resolver,
    ) {
        Ok(documentation) => documentation,
        Err(error) => panic!("documentation generation failed: {error}"),
    };
    let counter = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/00-custom-add/types/counter.md")
        .unwrap_or_else(|| panic!("missing Counter type page"));
    assert!(
        counter
            .markdown
            .contains("Adds one application-specific count.")
    );
    assert!(
        !counter
            .markdown
            .contains("Adds the receiver to the evaluated `other` operand.")
    );
}

/// Resolves compiler-test source files from an in-memory canonical source table.
struct TestResolver {
    /// Sources keyed by their canonical identity.
    sources: HashMap<String, String>,
}

impl ModuleResolver for TestResolver {
    /// Resolves a test import through the preconfigured source table.
    fn resolve(&mut self, _importer: &str, path: &str) -> Result<ResolvedSource, String> {
        let source_id = path.to_owned();
        let text = self
            .sources
            .get(&source_id)
            .cloned()
            .ok_or_else(|| format!("missing test source {path}"))?;
        Ok(ResolvedSource { source_id, text })
    }
}

/// Compiles direct namespace calls and `use` aliases from an in-memory source graph.
#[test]
fn compiles_imported_functions_and_use_aliases() {
    let mut resolver = TestResolver {
        sources: HashMap::from([(
            "./math.exs".to_owned(),
            "fn add(left: Int, right: Int) -> Int { ret left + right; }".to_owned(),
        )]),
    };
    let compiled = compile_with_resolver(
        SourceInput {
            source_id: "./main.exs",
            text: "import \"./math.exs\"; use math::add as plus; fn main() -> Int { ret math::add(20, 22); }",
        },
        CompileOptions::default(),
        &mut resolver,
    );
    if let Err(error) = compiled {
        panic!("compilation failed: {error}");
    }
}

/// Rejects imports that would shadow the compiler-provided std type namespace.
#[test]
fn rejects_an_import_named_std() {
    let mut resolver = TestResolver {
        sources: HashMap::from([(
            "./math.exs".to_owned(),
            "fn add(left: Int, right: Int) -> Int { ret left + right; }".to_owned(),
        )]),
    };
    let error = match compile_with_resolver(
        SourceInput {
            source_id: "./main.exs",
            text: "import \"./math.exs\" as std; fn main() -> Int { ret 0; }",
        },
        CompileOptions::default(),
        &mut resolver,
    ) {
        Ok(_) => panic!("expected the std namespace import to fail"),
        Err(error) => error,
    };
    assert!(error.contains("reserved built-in type namespace"));
}

/// Preserves imported source identities in linked debug metadata.
#[test]
fn emits_multisource_debug_metadata_for_imports() {
    let mut resolver = TestResolver {
        sources: HashMap::from([(
            "./math.exs".to_owned(),
            "fn add(left: Int, right: Int) -> Int { ret left + right; }".to_owned(),
        )]),
    };
    let compiled = match compile_with_resolver(
        SourceInput {
            source_id: "./main.exs",
            text: "import \"./math.exs\"; fn main() -> Int { ret math::add(20, 22); }",
        },
        CompileOptions {
            embed_sources: true,
        },
        &mut resolver,
    ) {
        Ok(compiled) => compiled,
        Err(error) => panic!("compilation failed: {error}"),
    };
    let debug_info = match read_debug_info(&compiled.wasm) {
        Ok(debug_info) => debug_info,
        Err(error) => panic!("could not read debug info: {error}"),
    };
    assert!(
        debug_info
            .positions
            .iter()
            .any(|position| position.source_id == "./math.exs")
    );
    assert!(debug_info.source_for("./math.exs").is_some());
}

/// Resolves imported nominal types for `use` aliases, construction, and static methods.
#[test]
fn compiles_imported_types_and_static_methods() {
    let mut resolver = TestResolver {
        sources: HashMap::from([(
            "./geometry.exs".to_owned(),
            "type Point { value: Int } impl Point { fn new(value: Int) -> Point { ret Point { value: value }; } }".to_owned(),
        )]),
    };
    let compiled = compile_with_resolver(
        SourceInput {
            source_id: "./main.exs",
            text: "import \"./geometry.exs\" as geo; use geo::{Point}; fn main() -> Point { ret Point::new(42); }",
        },
        CompileOptions::default(),
        &mut resolver,
    );
    if let Err(error) = compiled {
        panic!("compilation failed: {error}");
    }
}

/// Rejects cycles before attempting to merge imported declarations.
#[test]
fn rejects_import_cycles() {
    let mut resolver = TestResolver {
        sources: HashMap::from([
            (
                "./a.exs".to_owned(),
                "import \"./b.exs\"; fn value() { ret 1; }".to_owned(),
            ),
            (
                "./b.exs".to_owned(),
                "import \"./a.exs\"; fn value() { ret 2; }".to_owned(),
            ),
        ]),
    };
    let error = match compile_with_resolver(
        SourceInput {
            source_id: "./main.exs",
            text: "import \"./a.exs\"; fn main() { ret 0; }",
        },
        CompileOptions::default(),
        &mut resolver,
    ) {
        Ok(_) => panic!("expected import cycle to fail"),
        Err(error) => error,
    };
    assert!(error.contains("import cycle"));
}

/// Validates the generated Wasm shape for one Cell-backed closure expression.
#[test]
fn validates_wasm_for_a_closure_expression() {
    let compiled = match compile(
        SourceInput {
            source_id: "closure.exs",
            text: "fn main() -> Int { let count = 0; let increment = () => { count = count + 1; ret count; }; increment(); ret increment(); }",
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
