//! Runs ExS scripts that use the enabled libraries on the native server runner.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use exs_compiler::{
    CompileOptions, ModuleResolver, ResolvedSource, SourceInput, compile_with_resolver,
};
use exs_runner::{ExecutionCancellation, ExsValue, ServerRunner};

/// Provides the libraries of `exs-libs` and rejects file imports.
struct LibraryResolver;

impl ModuleResolver for LibraryResolver {
    /// Rejects every import; the tests only name libraries.
    fn resolve(&mut self, _importer: &str, path: &str) -> Result<ResolvedSource, String> {
        Err(format!("unexpected import `{path}`"))
    }

    /// Returns the enabled library at `path`.
    fn library(&mut self, path: &str) -> Option<ResolvedSource> {
        exs_libs::source(path).map(|text| ResolvedSource {
            source_id: path.to_owned(),
            text: text.to_owned(),
        })
    }

    /// Returns every enabled library.
    fn libraries(&mut self) -> Vec<String> {
        exs_libs::libraries()
    }
}

/// Polls `future` to completion on the current thread.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        std::thread::yield_now();
    }
}

/// Compiles `source` with the libraries and returns the result of its `main`.
fn run(source: &str) -> ExsValue {
    let compiled = compile_with_resolver(
        SourceInput {
            source_id: "test.exs",
            text: source,
        },
        CompileOptions::default(),
        &mut LibraryResolver,
    )
    .unwrap_or_else(|error| panic!("{source}\n{error}"));
    let mut runner = ServerRunner::default();
    exs_libs::register(runner.registry_mut()).expect("library host functions register");
    block_on(runner.execute(&compiled.wasm, "main", &[], &ExecutionCancellation::new()))
        .unwrap_or_else(|error| panic!("{source}\n{error}"))
}

/// Lists the enabled libraries.
#[test]
fn lists_enabled_libraries() {
    assert_eq!(exs_libs::libraries(), ["json", "url", "rand"]);
    assert!(exs_libs::source("json").is_some());
    assert!(exs_libs::source("missing").is_none());
}

/// Parses JSON into values a script can read and writes them back.
#[test]
fn round_trips_json() {
    assert_eq!(
        run(r#"fn main() -> String | Error {
            let order = json::parse("{\"id\": 7, \"items\": [1, 2.5], \"note\": null}")?;
            order.items.push("x");
            ret json::stringify(order);
        }"#),
        ExsValue::String(r#"{"id":7,"items":[1,2.5,"x"],"note":null}"#.to_owned())
    );
}

/// Returns invalid JSON as an Error the script can inspect.
#[test]
fn reports_invalid_json_to_scripts() {
    assert_eq!(
        run(r#"fn main() -> String {
            let result = json::parse("{");
            if result is Error { ret result.kind(); }
            ret "parsed";
        }"#),
        ExsValue::String("JsonSyntax".to_owned())
    );
}

/// Encodes and decodes query strings and URL parts.
#[test]
fn encodes_urls() {
    assert_eq!(
        run(r#"fn main() -> List | Error {
            let query = url::encode_query({ q: "a b", tag: ["x", "y"], page: 2 })?;
            let decoded = url::decode_query(f"?{query}")?;
            ret [query, decoded.q, decoded.tag, url::encode("a/b c")?, url::decode("a%2Fb%20c")?];
        }"#),
        ExsValue::List(vec![
            ExsValue::String("q=a+b&tag=x&tag=y&page=2".to_owned()),
            ExsValue::String("a b".to_owned()),
            ExsValue::List(vec![
                ExsValue::String("x".to_owned()),
                ExsValue::String("y".to_owned()),
            ]),
            ExsValue::String("a%2Fb%20c".to_owned()),
            ExsValue::String("a/b c".to_owned()),
        ])
    );
}

/// Draws random values within the requested bounds.
#[test]
fn draws_random_values() {
    assert_eq!(
        run(r#"fn main() -> Bool | Error {
            let die = rand::int(1, 6)?;
            let ratio = rand::float(0, 1)?;
            let code = rand::text(12)?;
            let key = rand::bytes(16)?;
            ret die >= 1 && die <= 6 && ratio >= 0.0 && ratio < 1.0 && code.length() == 12 && key.length() == 16;
        }"#),
        ExsValue::Bool(true)
    );
}

/// Documents the libraries from their sources.
#[test]
fn documents_libraries() {
    let documentation =
        exs_compiler::document_libraries(&mut LibraryResolver).expect("libraries document");
    let reference = documentation.llm();
    for declaration in [
        "fn parse(text: String) -> Any | Error",
        "fn stringify(value: Any) -> String | Error",
        "fn encode_query(values: Object) -> String | Error",
        "fn decode(text: String) -> String | Error",
        "fn int(min: Int, max: Int) -> Int | Error",
    ] {
        assert!(reference.contains(declaration), "{declaration}");
    }
}
