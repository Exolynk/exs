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
    fn resolve(&mut self, _importer: &str, path: &str) -> Result<ResolvedSource, String> {
        Err(format!("no file `{path}`"))
    }

    /// Returns the library module at `path`, if the table has one.
    fn library(&mut self, path: &str) -> Option<ResolvedSource> {
        self.asked.borrow_mut().push(path.to_owned());
        self.libraries.get(path).map(|text| ResolvedSource {
            source_id: path.to_owned(),
            text: (*text).to_owned(),
        })
    }
}

/// Returns a resolver with the libraries `log`, `exo` and `exo::db`, and the paths it was
/// asked for.
fn resolver() -> (LibraryResolver, Rc<RefCell<Vec<String>>>) {
    let asked = Rc::new(RefCell::new(Vec::new()));
    let resolver = LibraryResolver {
        libraries: HashMap::from([
            ("log", "fn info(value) -> Any { ret value; }"),
            (
                "exo",
                "type Record { id: Int } impl Record { fn new(id: Int) -> Record { ret Record { id: id }; } }",
            ),
            (
                "exo::db",
                "fn query(id: Int) -> exo::Record { ret exo::Record::new(id); } fn count() -> Int { ret 3; }",
            ),
            ("unused", "fn never() -> Int { ret 0; }"),
        ]),
        asked: Rc::clone(&asked),
    };
    (resolver, asked)
}

/// Compiles `text` with the test libraries and returns the error, if any.
fn compile_text(text: &str) -> Result<(), String> {
    let (mut resolver, _) = resolver();
    compile_with_resolver(
        SourceInput {
            source_id: "./main.exs",
            text,
        },
        CompileOptions::default(),
        &mut resolver,
    )
    .map(|_| ())
}

/// Names libraries without importing them, qualified, through `use` and through module aliases.
#[test]
fn names_libraries_without_imports() {
    for text in [
        "fn main() -> Int { ret log::info(1); }",
        "use log::info; fn main() -> Int { ret info(1); }",
        "use exo::db::{query, count as total}; fn main() -> Int { ret query(total()).id; }",
        "use exo::db; fn main() -> Int { ret db::query(2).id; }",
        "use exo::db as data; fn main() -> Int { ret data::count(); }",
        "use exo::Record; fn main() -> Int { ret Record::new(1).id + exo::db::count(); }",
        "use log; fn main() -> Int { ret log::info(4); }",
        "fn main() -> String { ret f\"{log::info(5)}\"; }",
        "use exo::{Record}; fn main() -> Int { ret Record::new(6).id; }",
        "use log; fn main() -> Int { ret 7; }",
    ] {
        if let Err(error) = compile_text(text) {
            panic!("{text}\n{error}");
        }
    }
}

/// Loads only the library modules a script names.
#[test]
fn loads_only_named_libraries() {
    let (mut resolver, asked) = resolver();
    let compiled = compile_with_resolver(
        SourceInput {
            source_id: "./main.exs",
            text: "fn main() -> Int { ret exo::db::count(); }",
        },
        CompileOptions::default(),
        &mut resolver,
    );
    assert!(compiled.is_ok(), "{compiled:?}");
    let asked = asked.borrow();
    assert!(asked.iter().any(|path| path == "exo::db"));
    assert!(!asked.iter().any(|path| path == "unused" || path == "log"));
}

/// Rejects unknown library members and import namespaces that collide with a library.
#[test]
fn rejects_unknown_library_names() {
    for text in [
        "use log::missing; fn main() -> Int { ret 1; }",
        "use exo::nothing; fn main() -> Int { ret 1; }",
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
        "import \"exo::db\" as data; fn main() -> Int { ret data::count(); }",
    ] {
        let error = compile_text(text).err().unwrap_or_default();
        assert!(
            error.contains("is a library and needs no import"),
            "{text}: {error}"
        );
    }
}
