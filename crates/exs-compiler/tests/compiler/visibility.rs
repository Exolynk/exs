use super::*;

/// Returns the names of the entry exports of `wasm`.
fn entry_exports(wasm: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    for payload in Parser::new(0).parse_all(wasm) {
        if let Ok(Payload::ExportSection(exports)) = payload {
            for export in exports.into_iter().flatten() {
                if let Some(name) = export.name.strip_prefix("__exs_start_") {
                    names.push(name.to_owned());
                }
            }
        }
    }
    names
}

/// Gives public root functions an entry export and keeps private ones internal.
#[test]
fn exports_only_public_root_functions() {
    let compiled = match compile(
        SourceInput {
            source_id: "visibility.exs",
            text: "private type Secret { value: Int } private enum Mode { On, } private trait Hidden { fn hidden(self) -> Int; } type Open { value: Int } impl Open { private fn helper(self) -> Int { ret self.value; } fn get(self) -> Int { ret self.helper(); } } private fn double(value: Int) -> Int { ret value * 2; } fn run() -> Int { ret double(Open { value: 21 }.get()); } fn main() -> Int { ret run(); }",
        },
        CompileOptions::default(),
    ) {
        Ok(compiled) => compiled,
        Err(error) => panic!("compilation failed: {error}"),
    };
    let mut exports = entry_exports(&compiled.wasm);
    exports.sort();
    assert_eq!(exports, ["main", "run"]);
}

/// Keeps `private` when formatting and rejects it before other declarations.
#[test]
fn formats_and_validates_private_declarations() {
    let source = "private fn helper() -> Int { ret 1; }\n\nprivate type Secret {\n    value: Int,\n}\n\nfn main() -> Int { ret helper(); }\n";
    let formatted = match format(SourceInput {
        source_id: "format.exs",
        text: source,
    }) {
        Ok(formatted) => formatted,
        Err(error) => panic!("formatting failed: {error}"),
    };
    assert!(
        formatted.contains("private fn helper() -> Int"),
        "{formatted}"
    );
    assert!(formatted.contains("private type Secret {"), "{formatted}");
    for text in [
        "private impl Open {} fn main() { ret 1; }",
        "private test \"x\" { } fn main() { ret 1; }",
    ] {
        let error = match compile(
            SourceInput {
                source_id: "invalid.exs",
                text,
            },
            CompileOptions::default(),
        ) {
            Ok(_) => panic!("misplaced private unexpectedly compiled"),
            Err(error) => error,
        };
        assert_eq!(error.diagnostics[0].code, "E0115", "{error}");
    }
}
