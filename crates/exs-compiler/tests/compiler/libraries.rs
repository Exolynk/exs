use super::*;

use std::cell::RefCell;
use std::rc::Rc;

/// Resolves library modules from a table and records every path asked for.
struct LibraryResolver {
    /// Library sources by module path.
    libraries: HashMap<&'static str, &'static str>,
    /// Every path asked for as a library.
    asked: Rc<RefCell<Vec<String>>>,
}

impl ModuleResolver for LibraryResolver {
    /// Rejects file imports; these tests only use libraries.
    fn resolve(&self, _importer: &str, path: &str) -> Result<Source<'_>, String> {
        Err(format!("no file `{path}`"))
    }

    /// Returns the library module at `path`, if the table has one.
    fn library(&self, path: &str) -> Option<Source<'_>> {
        self.asked.borrow_mut().push(path.to_owned());
        self.libraries
            .get(path)
            .map(|text| Source::new(path.to_owned(), (*text).to_owned()))
    }
}

/// Returns a resolver with the libraries `log`, `app` and `app::db`, and the paths it was
/// asked for.
fn resolver() -> (LibraryResolver, Rc<RefCell<Vec<String>>>) {
    let asked = Rc::new(RefCell::new(Vec::new()));
    let resolver = LibraryResolver {
        libraries: HashMap::from([
            ("log", "fn info(value) -> Any { ret value; }"),
            (
                "app",
                "type Record { id: Int } impl Record { fn new(id: Int) -> Record { ret Record { id: id }; } }",
            ),
            (
                "app::db",
                "type Row { id: Int } fn query(id: Int) -> app::Record { ret app::Record::new(id); } fn count() -> Int { ret 3; }",
            ),
            ("unused", "fn never() -> Int { ret 0; }"),
        ]),
        asked: Rc::clone(&asked),
    };
    (resolver, asked)
}

/// Compiles `text` with the test libraries and returns the error, if any.
fn compile_text(text: &str) -> Result<(), exs_compiler::CompileError> {
    let (resolver, _) = resolver();
    compile(
        Source::new("./main.exs", text),
        CompileOptions::default().resolver(&resolver),
    )
    .map(|_| ())
}

/// Names libraries without importing them, qualified, through `use` and through module aliases.
#[test]
fn names_libraries_without_imports() {
    for text in [
        "fn main() -> Int { ret log::info(1); }",
        "use log::info; fn main() -> Int { ret info(1); }",
        "use app::db::{query, count as total}; fn main() -> Int { ret query(total()).id; }",
        "use app::db; fn main() -> Int { ret db::query(2).id; }",
        "use app::db as data; fn main() -> Int { ret data::count(); }",
        "use app::Record; fn main() -> Int { ret Record::new(1).id + app::db::count(); }",
        "use log; fn main() -> Int { ret log::info(4); }",
        "fn main() -> String { ret f\"{log::info(5)}\"; }",
        "use app::{Record}; fn main() -> Int { ret Record::new(6).id; }",
        "use log; fn main() -> Int { ret 7; }",
    ] {
        if let Err(error) = compile_text(text) {
            panic!("{text}\n{error}");
        }
    }
}

/// Names types of nested library modules by their full path in every type position.
#[test]
fn names_types_of_nested_library_modules() {
    for text in [
        "fn main(value: Any) -> Bool { ret value is app::db::Row; }",
        "fn id(row: app::db::Row) -> Int { ret row.id; } fn main() -> Int { ret 1; }",
        "fn count(rows: List<app::db::Row>) -> Int { ret rows.length(); } fn main() -> Int { ret 1; }",
        "fn find() -> app::db::Row | None { ret None; } fn main() -> Int { ret 1; }",
    ] {
        if let Err(error) = compile_text(text) {
            panic!("{text}\n{error}");
        }
    }
    let error = compile_text("fn main(value: Any) -> Bool { ret value is app::db::Missing; }")
        .expect_err("an unknown type of a nested module is rejected");
    assert!(error.report.contains("E0216"), "{error}");
}

/// Loads only the library modules a script names.
#[test]
fn loads_only_named_libraries() {
    let (resolver, asked) = resolver();
    let compiled = compile(
        Source::new("./main.exs", "fn main() -> Int { ret app::db::count(); }"),
        CompileOptions::default().resolver(&resolver),
    );
    assert!(compiled.is_ok(), "{compiled:?}");
    let asked = asked.borrow();
    assert!(asked.iter().any(|path| path == "app::db"));
    assert!(!asked.iter().any(|path| path == "unused" || path == "log"));
}

/// Rejects unknown library members and import namespaces that collide with a library.
#[test]
fn rejects_unknown_library_names() {
    for text in [
        "use log::missing; fn main() -> Int { ret 1; }",
        "use app::nothing; fn main() -> Int { ret 1; }",
        "fn main() -> Int { ret log::missing(1); }",
    ] {
        assert!(compile_text(text).is_err(), "{text} unexpectedly compiled");
    }
}

/// Rejects imports of library modules with a hint to name them directly.
#[test]
fn rejects_library_imports() {
    for text in [
        "import \"log\"; fn main() -> Int { ret log::info(1); }",
        "import \"app::db\" as data; fn main() -> Int { ret data::count(); }",
    ] {
        let error = compile_text(text)
            .expect_err("the import is rejected")
            .report;
        assert!(
            error.contains("is a library and needs no import"),
            "{text}: {error}"
        );
    }
}
