use super::*;

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
