//! Embedder libraries and the visibility of their declarations, compiled and executed.

mod support;

use std::collections::HashMap;

use exs_abi::ExsValue;
use exs_compiler::{CompileOptions, ModuleResolver, Source, compile};
use exs_runner::ServerRunner;
use support::block_on;

/// Provides the test libraries `app` and `app::db`.
struct Libraries;

impl ModuleResolver for Libraries {
    /// Rejects file imports; these tests only use libraries.
    fn resolve(&self, _importer: &str, path: &str) -> Result<Source<'_>, String> {
        Err(format!("no file `{path}`"))
    }

    /// Returns the library module at `path`.
    fn library(&self, path: &str) -> Option<Source<'_>> {
        let text = HashMap::from([
            (
                "app",
                r#"
                type Record { id: Int }
                private type Hidden { id: Int }
                impl Record {
                    fn new(id: Int) -> Record { ret Record { id: id }; }
                    private fn make(id: Int) -> Record { ret Record { id: id * 10 }; }
                    fn label(self) -> String { ret f"record {self.secret()}"; }
                    private fn secret(self) -> Int { ret self.id + 1; }
                }
                private fn helper(value: Int) -> Int { ret value * 2; }
                const LIMIT = 10;
                private const SECRET = 7;
                "#,
            ),
            (
                "app::db",
                r#"
                fn query(id: Int) -> app::Record { ret app::Record::make(app::helper(id)); }
                fn secret_of(record: app::Record) -> Int { ret record.secret(); }
                fn limit_and_secret() -> Int { ret app::LIMIT + app::SECRET; }
                "#,
            ),
        ])
        .remove(path)?;
        Some(Source::new(path.to_owned(), text.to_owned()))
    }
}

/// Compiles `text` with the test libraries and runs its `main`.
fn run(text: &str) -> Result<ExsValue, String> {
    let module = compile(
        Source::new("./main.exs", text),
        CompileOptions::default().resolver(&Libraries),
    )
    .map_err(|error| error.to_string())?;
    block_on(ServerRunner::default().execute(&module.wasm, "main", &[]))
        .map_err(|error| error.to_string())
}

/// Uses a library's public declarations, which reach its private ones internally.
#[test]
fn library_code_uses_its_private_declarations() {
    assert_eq!(
        run(
            "use app::db; fn main() -> Any { let record = db::query(2); ret [record.id, record.label(), db::secret_of(record)]; }"
        ),
        Ok(ExsValue::List(vec![
            ExsValue::Int(40),
            ExsValue::String("record 41".to_owned()),
            ExsValue::Int(41),
        ]))
    );
}

/// Uses library constants by path and through `use`; the library reaches its private ones.
#[test]
fn uses_library_constants() {
    assert_eq!(
        run(
            "use app::LIMIT; fn main() -> Any { ret [app::LIMIT, LIMIT, app::db::limit_and_secret()]; }"
        ),
        Ok(ExsValue::List(vec![
            ExsValue::Int(10),
            ExsValue::Int(10),
            ExsValue::Int(17),
        ]))
    );
}

/// Rejects naming private functions, types and static methods from outside their library.
#[test]
fn rejects_private_names_from_outside() {
    for (text, message) in [
        (
            "fn main() -> Int { ret app::helper(1); }",
            "`app::helper` is private to its library",
        ),
        (
            "use app::helper; fn main() -> Int { ret helper(1); }",
            "`app::helper` is private to its library",
        ),
        (
            "fn main() -> Any { ret app::Hidden { id: 1 }; }",
            "`app::Hidden` is private to its library",
        ),
        (
            "fn main() -> Any { ret app::Record::make(1); }",
            "static method `make` is private to its library",
        ),
        (
            "fn main() -> Int { ret app::SECRET; }",
            "`app::SECRET` is private to its library",
        ),
        (
            "fn main() -> Any { let ignored = Host::call(\"x\"); ret app::Record::make(1); }",
            "static method `make` is private to its library",
        ),
    ] {
        match run(text) {
            Err(error) => assert!(error.contains(message), "{text}\n{error}"),
            Ok(value) => panic!("{text} unexpectedly ran: {value:?}"),
        }
    }
}

/// Calls of a private instance method from outside its library fall back to the built-in
/// behavior: MethodNotFound.
#[test]
fn private_instance_methods_are_not_found_from_outside() {
    let Ok(ExsValue::Error(error)) = run("fn main() -> Any { ret app::Record::new(1).secret(); }")
    else {
        panic!("expected a MethodNotFound Error");
    };
    assert_eq!(error.kind, "MethodNotFound");
}

/// Keeps the prelude's private helpers from scripts, with or without a resolver.
#[test]
fn hides_private_prelude_functions() {
    let text = "fn main() -> Any { ret __exs_list_map([1], (value) => { ret value; }); }";
    let error = match compile(Source::new("main.exs", text), CompileOptions::default()) {
        Ok(_) => panic!("a private prelude function was callable"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains("`__exs_list_map` is private to its library"),
        "{error}"
    );
    let error = run(text).expect_err("a private prelude function was callable");
    assert!(
        error.contains("`__exs_list_map` is private to its library"),
        "{error}"
    );
}
