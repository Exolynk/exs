use super::*;

use exs_compiler::CompileLimits;

/// Returns options whose limits `limits` adjusts from the defaults.
fn options(limits: impl FnOnce(&mut CompileLimits)) -> CompileOptions {
    let mut options = CompileOptions::default();
    limits(&mut options.limits);
    options
}

/// Rejects nesting beyond the limit with a diagnostic instead of exhausting the stack.
#[test]
fn rejects_nesting_beyond_the_limit() {
    // Debug builds use far more stack per level, so the test keeps every depth small.
    let limited = |max_nesting| {
        options(|limits| {
            limits.max_nesting = max_nesting;
            limits.max_source_bytes = usize::MAX;
        })
    };
    let nested = |depth: usize| {
        format!(
            "fn main() {{ ret {}1{}; }}",
            "(".repeat(depth),
            ")".repeat(depth)
        )
    };
    let compiles = |text: &str, max_nesting| {
        compile(
            SourceInput {
                source_id: "nested.exs",
                text,
            },
            limited(max_nesting),
        )
        .map(|_| ())
        .map_err(|error| error.diagnostics[0].code)
    };
    assert_eq!(compiles(&nested(10), 16), Ok(()));
    assert_eq!(compiles(&nested(10), 8), Err("E0114"));
    for text in [
        nested(100_000),
        format!("fn main() {{ ret {}1; }}", "-".repeat(100_000)),
        format!(
            "fn main() {{ {} ret 1; {} }}",
            "{".repeat(100_000),
            "}".repeat(100_000)
        ),
        format!(
            "fn f(value: {}Int{}) {{ ret 1; }} fn main() {{ ret 1; }}",
            "List<".repeat(100_000),
            ">".repeat(100_000)
        ),
        format!(
            "fn main() {{ ret f\"{}\"; }}",
            "{f\"".repeat(1_000) + &"\"}".repeat(1_000)
        ),
    ] {
        assert_eq!(compiles(&text, 16), Err("E0114"));
    }
}

/// Rejects a source larger than the limit before lexing it.
#[test]
fn rejects_sources_beyond_the_size_limit() {
    let error = match compile(
        SourceInput {
            source_id: "large.exs",
            text: "fn main() { ret 1; }",
        },
        options(|limits| limits.max_source_bytes = 10),
    ) {
        Ok(_) => panic!("a large source unexpectedly compiled"),
        Err(error) => error,
    };
    assert_eq!(error.diagnostics[0].code, "E0010");
}

/// Rejects projects with more modules or source bytes than the limits allow.
#[test]
fn rejects_projects_beyond_the_module_and_size_limits() {
    let source = SourceInput {
        source_id: "./main.exs",
        text: "import \"./math.exs\"; fn main() -> Int { ret math::one(); }",
    };
    let resolver = || TestResolver {
        sources: HashMap::from([(
            "./math.exs".to_owned(),
            "fn one() -> Int { ret 1; }".to_owned(),
        )]),
    };
    assert!(compile_with_resolver(source, CompileOptions::default(), &mut resolver()).is_ok());
    let modules = compile_with_resolver(
        source,
        options(|limits| limits.max_modules = 1),
        &mut resolver(),
    );
    assert!(matches!(modules, Err(error) if error.contains("limit of 1 modules")));
    let total = compile_with_resolver(
        source,
        options(|limits| limits.max_total_source_bytes = source.text.len() + 5),
        &mut resolver(),
    );
    assert!(matches!(total, Err(error) if error.contains("sources exceed the limit")));
}
