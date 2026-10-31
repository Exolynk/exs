//! Located diagnostics for editors: the light syntax check and detailed compilation.

use std::collections::HashMap;

use exs_compiler::{CompileOptions, SourceInput, check_syntax, compile_with_resolver_diagnostics};

use super::TestResolver;

/// The syntax check locates a broken statement in the entry source.
#[test]
fn syntax_check_locates_errors() {
    let text = "fn main() {\n    let value = ;\n}\n";
    let diagnostics = check_syntax(SourceInput {
        source_id: "check.exs",
        text,
    });
    assert!(!diagnostics.is_empty());
    let first = &diagnostics[0];
    let line = text[..first.start_byte as usize].matches('\n').count();
    assert_eq!(line, 1, "{diagnostics:?}");
}

/// Valid syntax passes the check, also without `fn main()` and with unknown names.
#[test]
fn syntax_check_accepts_valid_source() {
    let diagnostics = check_syntax(SourceInput {
        source_id: "check.exs",
        text: "fn helper() -> Int {\n    ret missing;\n}\n",
    });
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

/// Detailed compilation keeps semantic errors with their entry-source position.
#[test]
fn compilation_locates_semantic_errors() {
    let text = "fn main() -> Int {\n    ret missing;\n}\n";
    let mut resolver = TestResolver {
        sources: HashMap::new(),
    };
    let failure = match compile_with_resolver_diagnostics(
        SourceInput {
            source_id: "semantic.exs",
            text,
        },
        CompileOptions::default(),
        &mut resolver,
    ) {
        Ok(_) => panic!("an unknown binding compiled"),
        Err(failure) => failure,
    };
    assert!(failure.report.contains("missing"), "{}", failure.report);
    let diagnostic = failure
        .diagnostics
        .first()
        .unwrap_or_else(|| panic!("no located diagnostic: {}", failure.report));
    assert_eq!(
        &text[diagnostic.start_byte as usize..diagnostic.end_byte as usize],
        "missing"
    );
}

/// Syntax errors found while compiling keep their entry-source position too.
#[test]
fn compilation_locates_syntax_errors() {
    let text = "fn main() -> Int {\n    ret 1\n}\n";
    let mut resolver = TestResolver {
        sources: HashMap::new(),
    };
    let failure = match compile_with_resolver_diagnostics(
        SourceInput {
            source_id: "syntax.exs",
            text,
        },
        CompileOptions::default(),
        &mut resolver,
    ) {
        Ok(_) => panic!("a missing semicolon compiled"),
        Err(failure) => failure,
    };
    assert!(!failure.diagnostics.is_empty(), "{}", failure.report);
}
