use super::*;

/// Returns the error of compiling `text`.
fn compile_error(text: &str) -> String {
    match compile(
        SourceInput {
            source_id: "./main.exs",
            text,
        },
        CompileOptions::default(),
    ) {
        Ok(_) => panic!("{text} unexpectedly compiled"),
        Err(error) => error.to_string(),
    }
}

/// Rejects constant values that are no literals, cycles, and assignments to constants.
#[test]
fn rejects_invalid_constants() {
    for (text, code) in [
        ("const A = [1]; fn main() { ret A; }", "E0119"),
        ("const A = 1 + 2; fn main() { ret A; }", "E0119"),
        ("const A = missing; fn main() { ret A; }", "E0119"),
        ("const A = B; const B = A; fn main() { ret A; }", "E0121"),
        ("const A = 1; fn main() { A = 2; ret A; }", "E0120"),
        (
            "const A = 1; fn A() { ret 1; } fn main() { ret A; }",
            "duplicate",
        ),
    ] {
        let error = compile_error(text);
        assert!(error.contains(code), "{text}\n{error}");
    }
}

/// Resolves constants across imported files, through `use`, and between constants.
#[test]
fn resolves_constants_across_modules() {
    let mut resolver = TestResolver {
        sources: HashMap::from([(
            "./limits.exs".to_owned(),
            "const MAX = 10; const DEFAULT = MAX; private const HIDDEN = 1;".to_owned(),
        )]),
    };
    let compiled = compile_with_resolver(
        SourceInput {
            source_id: "./main.exs",
            text: "import \"./limits.exs\" as limits; use limits::DEFAULT; const LOCAL = limits::MAX; fn main() -> Int { ret DEFAULT + LOCAL + limits::HIDDEN; }",
        },
        CompileOptions::default(),
        &mut resolver,
    );
    assert!(compiled.is_ok(), "{:?}", compiled.err());
}

/// Formats constants as one group before the other declarations.
#[test]
fn formats_constants() {
    let formatted = match format(SourceInput {
        source_id: "./main.exs",
        text: "fn main(){ret A;}\nconst A=1;\nprivate const B=-2;",
    }) {
        Ok(formatted) => formatted,
        Err(error) => panic!("formatting failed: {error}"),
    };
    assert!(
        formatted.starts_with("const A = 1;\nprivate const B = -2;\n\nfn main() {"),
        "{formatted}"
    );
}
