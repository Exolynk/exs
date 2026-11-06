use super::*;

/// Compiles direct namespace calls and `use` aliases from an in-memory source graph.
#[test]
fn compiles_imported_functions_and_use_aliases() {
    let resolver = TestResolver {
        sources: HashMap::from([(
            "./math.exs".to_owned(),
            "fn add(left: Int, right: Int) -> Int { ret left + right; }".to_owned(),
        )]),
    };
    let compiled = compile(
        Source::new(
            "./main.exs",
            "import \"./math.exs\"; use math::add as plus; fn main() -> Int { ret math::add(20, 22); }",
        ),
        CompileOptions::default().resolver(&resolver),
    );
    if let Err(error) = compiled {
        panic!("compilation failed: {error}");
    }
}

/// Rejects imports that would shadow the compiler-provided std type namespace.
#[test]
fn rejects_an_import_named_std() {
    let resolver = TestResolver {
        sources: HashMap::from([(
            "./math.exs".to_owned(),
            "fn add(left: Int, right: Int) -> Int { ret left + right; }".to_owned(),
        )]),
    };
    let error = match compile(
        Source::new(
            "./main.exs",
            "import \"./math.exs\" as std; fn main() -> Int { ret 0; }",
        ),
        CompileOptions::default().resolver(&resolver),
    ) {
        Ok(_) => panic!("expected the std namespace import to fail"),
        Err(error) => error,
    };
    assert!(error.report.contains("reserved built-in type namespace"));
}

/// Preserves imported source identities in linked debug metadata.
#[test]
fn emits_multisource_debug_metadata_for_imports() {
    let resolver = TestResolver {
        sources: HashMap::from([(
            "./math.exs".to_owned(),
            "fn add(left: Int, right: Int) -> Int { ret left + right; }".to_owned(),
        )]),
    };
    let compiled = match compile(
        Source::new(
            "./main.exs",
            "import \"./math.exs\"; fn main() -> Int { ret math::add(20, 22); }",
        ),
        {
            let mut options = CompileOptions::default();
            options.embed_sources = true;
            options
        }
        .resolver(&resolver),
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
    let resolver = TestResolver {
        sources: HashMap::from([(
            "./geometry.exs".to_owned(),
            "type Point { value: Int } impl Point { fn new(value: Int) -> Point { ret Point { value: value }; } }".to_owned(),
        )]),
    };
    let compiled = compile(
        Source::new(
            "./main.exs",
            "import \"./geometry.exs\" as geo; use geo::{Point}; fn main() -> Point { ret Point::new(42); }",
        ),
        CompileOptions::default().resolver(&resolver),
    );
    if let Err(error) = compiled {
        panic!("compilation failed: {error}");
    }
}

/// Resolves static methods through a module-qualified nominal type.
#[test]
fn compiles_module_qualified_static_methods() {
    let resolver = TestResolver {
        sources: HashMap::from([(
            "./geometry.exs".to_owned(),
            "type Point { value: Int } impl Point { fn new(value: Int) -> Point { ret Point { value: value }; } }".to_owned(),
        )]),
    };
    compile(
        Source::new(
            "./main.exs",
            "import \"./geometry.exs\" as geo; fn main() -> Int { ret geo::Point::new(42).value; }",
        ),
        CompileOptions::default().resolver(&resolver),
    )
    .expect("module-qualified static method compiles");
}

/// Resolves bare enum variants inside imported methods and through module or use aliases.
#[test]
fn compiles_imported_enum_variant_values() {
    let resolver = TestResolver {
        sources: HashMap::from([(
            "./status.exs".to_owned(),
            "enum Status { Booked, Pending, Unknown(code: String), } impl Status { fn booked() -> Status { ret Status::Booked; } fn is_booked(self) -> Bool { ret self == Status::Booked; } }".to_owned(),
        )]),
    };
    compile(Source::new("./main.exs", "import \"./status.exs\" as status; use status::{Status}; fn main() -> Bool { let booked = status::Status::Booked; let unknown = status::Status::Unknown(\"CUSTOM\"); ret booked.is_booked() && Status::Booked == status::Status::booked() && Status::Pending != booked; }"), CompileOptions::default().resolver(&resolver)).expect("imported enum values compile");
}

/// Preserves lexical variables that shadow unqualified declarations during import rewriting.
#[test]
fn imported_modules_preserve_local_bindings() {
    let resolver = TestResolver {
        sources: HashMap::from([(
            "./math.exs".to_owned(),
            "fn value() -> Int { ret 1; } fn answer() -> Int { let value = 42; ret value; }"
                .to_owned(),
        )]),
    };
    compile(
        Source::new(
            "./main.exs",
            "import \"./math.exs\"; fn main() -> Int { ret math::answer(); }",
        ),
        CompileOptions::default().resolver(&resolver),
    )
    .expect("lexical binding remains local");
}

/// Rejects function names with `::`, which library modules replace.
#[test]
fn rejects_qualified_function_names() {
    let error = match compile(
        Source::new(
            "./main.exs",
            "fn ui::reload() -> String { ret \"reloaded\"; } fn main() -> String { ret \"\"; }",
        ),
        CompileOptions::default(),
    ) {
        Ok(_) => panic!("a qualified function name unexpectedly compiled"),
        Err(error) => error,
    };
    assert_eq!(error.diagnostics[0].code, "E0116");
}

/// Resolves nominal types used by typed bindings inside imported function bodies.
#[test]
fn compiles_imported_typed_host_bindings() {
    let resolver = TestResolver {
        sources: HashMap::from([(
            "./fleet.exs".to_owned(),
            "type Vehicle { id: String } fn current() -> Vehicle | Error { let vehicle: Vehicle = Host::call(\"fleet.current\")?; ret vehicle; }".to_owned(),
        )]),
    };
    let compiled = compile(
        Source::new(
            "./main.exs",
            "import \"./fleet.exs\" as fleet; fn main() -> Any { ret fleet::current(); }",
        ),
        CompileOptions::default().resolver(&resolver),
    );
    if let Err(error) = compiled {
        panic!("compilation failed: {error}");
    }
}

/// Rejects cycles before attempting to merge imported declarations.
#[test]
fn rejects_import_cycles() {
    let resolver = TestResolver {
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
    let error = match compile(
        Source::new("./main.exs", "import \"./a.exs\"; fn main() { ret 0; }"),
        CompileOptions::default().resolver(&resolver),
    ) {
        Ok(_) => panic!("expected import cycle to fail"),
        Err(error) => error,
    };
    assert!(error.report.contains("import cycle"));
}

/// Uses the shared project graph to report matching cycles for compilation and documentation.
#[test]
fn compilation_and_documentation_report_the_same_import_cycle() {
    let sources = HashMap::from([
        (
            "./a.exs".to_owned(),
            "import \"./b.exs\"; fn value() { ret 1; }".to_owned(),
        ),
        (
            "./b.exs".to_owned(),
            "import \"./a.exs\"; fn value() { ret 2; }".to_owned(),
        ),
    ]);
    let source = Source::new("./main.exs", "import \"./a.exs\"; fn main() { ret 0; }");
    let compilation_error = compile(
        source.clone(),
        CompileOptions::default().resolver(&TestResolver {
            sources: sources.clone(),
        }),
    )
    .expect_err("cyclic imports compiled");
    let documentation_error = document(
        source,
        CompileOptions::default().resolver(&TestResolver { sources }),
    )
    .expect_err("cyclic imports generated documentation");

    assert_eq!(compilation_error, documentation_error);
}

/// Finds imports after `use` declarations and still reports syntax errors in the bodies of
/// imported modules, which the loader no longer parses in full.
#[test]
fn reports_syntax_errors_of_imported_modules() {
    let source = Source::new(
        "./main.exs",
        "import \"./math.exs\"; use math::one; import \"./text.exs\"; fn main() -> Int { ret one(); }",
    );
    let resolver = |math: &str| TestResolver {
        sources: HashMap::from([
            ("./math.exs".to_owned(), math.to_owned()),
            (
                "./text.exs".to_owned(),
                "fn hello() -> String { ret \"hi\"; }".to_owned(),
            ),
        ]),
    };
    assert!(
        compile(
            source.clone(),
            CompileOptions::default().resolver(&resolver("fn one() -> Int { ret 1; }"))
        )
        .is_ok()
    );
    let error = match compile(
        source,
        CompileOptions::default().resolver(&resolver("fn one() -> Int { ret 1 }")),
    ) {
        Ok(_) => panic!("a syntax error in an imported module unexpectedly compiled"),
        Err(error) => error,
    };
    assert!(error.report.contains("./math.exs"), "{error}");
}

/// Provides one imported module with the source `text`.
struct OneImport(&'static str);

impl ModuleResolver for OneImport {
    fn resolve(&self, _importer: &str, _path: &str) -> Result<Source<'_>, String> {
        Ok(Source::new("other.exs".to_owned(), self.0.to_owned()))
    }
}

/// Compiles the entry source `text` with the import `other`.
fn project_error(text: &str, other: &'static str) -> exs_compiler::CompileError {
    match compile(
        Source::new("app.exs", text),
        CompileOptions::default().resolver(&OneImport(other)),
    ) {
        Ok(_) => panic!("{text} compiled"),
        Err(error) => error,
    }
}

/// Reports the lexical and syntax errors of one source together, in source order.
#[test]
fn reports_lexical_and_syntax_errors_together() {
    let error = project_error(
        "fn main() { @ }
fn other( {",
        "",
    );
    let codes: Vec<_> = error.diagnostics.iter().map(|error| error.code).collect();
    assert!(codes.contains(&"E0001"), "{error}");
    assert!(codes.iter().any(|code| code.starts_with("E01")), "{error}");
    let positions: Vec<_> = error
        .diagnostics
        .iter()
        .map(|error| error.start_byte)
        .collect();
    assert!(positions.is_sorted(), "{error}");
    let checked = exs_compiler::check_syntax(Source::new(
        "app.exs",
        "fn main() { @ }
fn other( {",
    ));
    assert_eq!(checked, error.diagnostics);
}

/// Reports every error of a project with its code and location.
#[test]
fn reports_structured_diagnostics_for_projects() {
    let lexical = project_error("@ # fn main() { ret None; }", "");
    assert_eq!(lexical.diagnostics.len(), 2, "{lexical}");
    assert!(
        lexical
            .diagnostics
            .iter()
            .all(|error| error.code == "E0001")
    );
    let semantic = project_error(
        "type A { x } type A { y } enum B { C, C, } fn main() { ret missing; }",
        "",
    );
    assert!(semantic.diagnostics.len() >= 2, "{semantic}");
    assert!(
        semantic
            .diagnostics
            .iter()
            .any(|error| error.code == "E0219")
    );
    let import = project_error("import \"./a.exs\" as 1;", "");
    assert!(!import.diagnostics.is_empty(), "{import}");
}

/// Locates an error of an imported module in that module, with an excerpt of its text.
#[test]
fn locates_errors_in_imported_modules() {
    let error = project_error(
        "import \"./other.exs\" as other; fn main() { ret other::broken(); }",
        "fn broken() { ret missing_in_other; }",
    );
    assert_eq!(error.diagnostics[0].source_id, "other.exs", "{error}");
    assert!(error.report.contains("missing_in_other"), "{error}");
    let syntax = project_error(
        "import \"./other.exs\" as other; fn main() { ret 1; }",
        "fn broken( {",
    );
    assert_eq!(syntax.diagnostics[0].source_id, "other.exs", "{syntax}");
}

/// Locates import failures at the import declaration.
#[test]
fn locates_failed_imports() {
    let text = "import \"./missing.exs\" as other; fn main() { ret 1; }";
    let error = match compile(Source::new("app.exs", text), CompileOptions::default()) {
        Ok(_) => panic!("the import resolved"),
        Err(error) => error,
    };
    assert_eq!(error.diagnostics[0].code, "E0233", "{error}");
    assert_eq!(error.diagnostics[0].start_byte, 0);
    assert!(error.report.contains("./missing.exs"), "{error}");
}

/// Compiles one program to the same bytes every time, with and without embedded sources.
#[test]
fn compiles_deterministically() {
    let text = "type A { v: Int, w: String } enum B { C(x: Int), D, } trait T { fn f(self); } \
                impl T for A { fn f(self) { ret b\"x\"; } } \
                fn main() { let k = { one: 1, two: 2 }; ret [A { v: 1, w: \"s\" }.f(), B::D, k]; }";
    for embed_sources in [false, true] {
        let mut options = CompileOptions::default();
        options.embed_sources = embed_sources;
        let build = || {
            compile(Source::new("app.exs", text), options)
                .expect("the source compiles")
                .wasm
        };
        let first = build();
        for _ in 0..8 {
            assert!(first == build(), "two builds differ");
        }
    }
}

/// Provides known sources without a resolver of the application's own, resolving imports relative
/// to the importing file.
#[test]
fn compiles_with_sources() {
    let mut sources = exs_compiler::Sources::new();
    sources
        .add(
            "./lib/a.exs",
            "import \"./b.exs\" as b; fn a() -> Int { ret b::b(); }",
        )
        .add(
            "lib/b.exs",
            "import \"../c.exs\" as c; fn b() -> Int { ret c::c(); }",
        )
        .add("c.exs", "fn c() -> Int { ret log::info(1); }")
        .add_library("log", "fn info(value) -> Any { ret value; }");
    let options = CompileOptions::default().resolver(&sources);
    let text = "import \"./lib/a.exs\" as a; fn main() -> Int { ret a::a(); }";
    // One `Copy` options value serves several compilations.
    assert!(compile(Source::new("app.exs", text), options).is_ok());
    assert!(compile(Source::new("other.exs", text), options).is_ok());
    let missing = compile(
        Source::new(
            "app.exs",
            "import \"./none.exs\" as n; fn main() { ret 1; }",
        ),
        options,
    )
    .expect_err("the file is missing");
    assert_eq!(missing.diagnostics[0].code, "E0233");
    assert!(
        missing.report.contains("no source at `none.exs`"),
        "{missing}"
    );
    assert_eq!(sources_libraries(&sources), ["log"]);
}

/// Returns the libraries `resolver` lists.
fn sources_libraries(resolver: &dyn ModuleResolver) -> Vec<String> {
    resolver.libraries()
}
