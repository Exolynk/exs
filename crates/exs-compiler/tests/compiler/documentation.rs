use exs_compiler::{
    ApiDocumentation, Documentation, DocumentationPage, MarkdownOptions, PageInfo, PageKind,
    PageLayout, document_libraries,
};

use super::*;

/// Root source that imports `./math.exs`.
const MAIN: &str = "//! The demo program.\nimport \"./math.exs\" as math;\n/// Runs the program.\nfn main() -> Int { ret math::add(20, 22); }";

/// Module with public and private declarations of every kind.
const MATH: &str = "/// Adds two integers.\nfn add(left: Int, right: Int) -> Int { ret left + right; }\n\nprivate fn hidden() -> Int { ret 1; }\n\n/// A coordinate.\ntype Point { value: Int }\n\nimpl Point {\n    fn coordinate(self) -> Int { ret self.value; }\n    private fn secret(self) -> Int { ret 0; }\n}\n\n/// A display color.\nenum Color {\n    /// Red, green, and blue channels.\n    Rgb(red: Int, green: Int, blue: Int),\n    Transparent,\n}\n\nimpl Color { fn channels(self) -> Int { ret 3; } }\n\nprivate type Secret {}";

/// Documents `MAIN` and `MATH`.
fn project_documentation() -> ApiDocumentation {
    let mut resolver = TestResolver {
        sources: HashMap::from([("./math.exs".to_owned(), MATH.to_owned())]),
    };
    match document_with_resolver(
        SourceInput {
            source_id: "./main.exs",
            text: MAIN,
        },
        &mut resolver,
    ) {
        Ok(documentation) => documentation,
        Err(error) => panic!("documentation generation failed: {error}"),
    }
}

/// Returns the page at `path`.
fn page<'d>(documentation: &'d Documentation, path: &str) -> &'d DocumentationPage {
    documentation
        .pages
        .iter()
        .find(|page| page.path == path)
        .unwrap_or_else(|| panic!("missing page {path}"))
}

/// Builds one module per standard, library, and project module with only public items.
#[test]
fn builds_the_documentation_model() {
    let documentation = project_documentation();
    let names = documentation
        .modules
        .iter()
        .map(|module| module.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["std", "std::test", "./main.exs", "./math.exs"]);
    let main = documentation.module("./main.exs").expect("main module");
    assert_eq!(main.segments, ["main"]);
    assert_eq!(main.description, "The demo program.");
    let math = documentation.module("./math.exs").expect("math module");
    let items = math
        .items
        .iter()
        .map(|item| item.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(items, ["Point", "Color", "add"]);
    let point = math.item("Point").expect("Point");
    assert!(
        point
            .sections
            .iter()
            .flat_map(|section| &section.members)
            .all(|member| member.name != "secret")
    );
    let std = documentation.module("std").expect("std module");
    assert!(std.items.iter().all(|item| !item.name.starts_with("__")));
    assert!(std.item("Duration").is_some());
    assert!(std.item("Host").is_some());
}

/// Renders one page per module by default.
#[test]
fn renders_one_page_per_module() {
    let documentation = project_documentation().markdown(&MarkdownOptions::default());
    let paths = documentation
        .pages
        .iter()
        .map(|page| page.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            "index.md",
            "std/index.md",
            "std/test/index.md",
            "main/index.md",
            "math/index.md"
        ]
    );
    let index = page(&documentation, "index.md");
    assert!(index.markdown.starts_with("# ExS API Documentation"));
    assert!(index.markdown.contains("## Language"));
    assert!(index.markdown.contains("- [`std`](std/index.md)"));
    assert!(
        index
            .markdown
            .contains("- [`./main.exs`](main/index.md) - The demo program.")
    );
    let math = &page(&documentation, "math/index.md").markdown;
    assert!(math.starts_with("# Module `./math.exs`"));
    assert!(math.contains(
        "## Type `Point`\n\nA coordinate.\n\n```exs\ntype Point {\n    value: Int,\n}\n```"
    ));
    assert!(
        math.contains("### Implemented Methods\n\n#### Inherent methods\n\n##### `coordinate`")
    );
    assert!(math.contains("fn clone(self) -> Point | Error"));
    assert!(math.contains("Rgb(red: Int, green: Int, blue: Int)"));
    assert!(math.contains("#### `Rgb`\n\nRed, green, and blue channels."));
    assert!(math.contains("## Functions\n\n### `add`\n\nAdds two integers."));
    assert!(!math.contains("hidden"));
    assert!(!math.contains("secret"));
    assert!(!math.contains("Secret"));
    let std = &page(&documentation, "std/index.md").markdown;
    assert!(std.contains("`std::` qualifier"));
    assert!(std.contains("- [`std::test`](test/index.md)"));
    assert!(std.contains("## Namespace `Host`"));
    assert!(std.contains("fn Host::call(name, arguments...)"));
    assert!(std.contains("fn Host::stream(name, arguments...) -> HostStream | Error"));
    assert!(std.contains("fn Host::sleep(duration: Duration) -> Unit"));
    assert!(std.contains("## Type `Duration`"));
    assert!(std.contains("Creates an exact Duration from a non-negative millisecond count."));
    assert!(std.contains("fn milliseconds(value: Int) -> Duration | Error"));
    assert!(std.contains("fn as_nanoseconds(self) -> Int | Error"));
    assert!(std.contains("optional IANA time-zone metadata"));
    assert!(std.contains("## Enum `IteratorStep`"));
    assert!(std.contains("Item(value: Any)"));
    assert!(std.contains("## Trait `Iterator`"));
    assert!(std.contains("fn next(self) -> IteratorStep | Error;"));
    assert!(std.contains("##### `map`"));
    assert!(std.contains("#### Trait [`Iterator`](#trait-iterator)"));
    assert!(std.contains("#### Trait [`Add`](#trait-add)"));
    assert!(std.contains("- [`String`](#type-string)"));
    assert!(std.contains("### `Error`\n\nConstructs a recoverable Error"));
    assert!(std.contains("fn Error(kind, message, data)"));
    assert!(std.contains("##### `push`"));
    assert!(std.contains("mutates the existing List"));
    assert!(std.contains("fn compare(self, other: Any) -> Ordering;"));
    assert!(std.contains("Ordering::Unordered"));
    assert!(!std.contains("__exs_"));
    let test = &page(&documentation, "std/test/index.md").markdown;
    assert!(test.contains("fn assert(condition: Bool"));
    assert!(test.contains("fn assert_eq(actual: Any, expected: Any"));
}

/// Gives types, enums, traits, and namespaces their own pages in the per-item layout.
#[test]
fn renders_one_page_per_item() {
    let documentation = project_documentation().markdown(&MarkdownOptions {
        layout: PageLayout::PerItem,
        ..MarkdownOptions::default()
    });
    let math = &page(&documentation, "math/index.md").markdown;
    assert!(math.contains("## Types\n\n- [`Point`](point.md)"));
    assert!(math.contains("## Enums\n\n- [`Color`](color.md)"));
    assert!(math.contains("## Functions\n\n### `add`"));
    let color = &page(&documentation, "math/color.md").markdown;
    assert!(color.starts_with("# Enum `Color`"));
    assert!(color.contains("## Variants"));
    let range = &page(&documentation, "std/range.md").markdown;
    assert!(range.contains("### Trait [`Iterator`](iterator.md)"));
    let string = &page(&documentation, "std/string.md").markdown;
    assert!(string.contains("### Trait [`Add`](add.md)"));
    assert!(
        page(&documentation, "std/host.md")
            .markdown
            .contains("## Functions")
    );
    let add = &page(&documentation, "std/add.md").markdown;
    assert!(add.contains("## Built-in Implementations"));
    assert!(add.contains("- [`Int`](int.md)"));
    assert!(add.contains("Vector { value: 20 } + Vector { value: 22 }"));
}

/// Writes front matter, absolute links, and custom index file names for static-site
/// generators such as Zola.
#[test]
fn renders_front_matter_and_absolute_links() {
    let front_matter = |info: &PageInfo<'_>| {
        let template = match info.kind {
            PageKind::Index | PageKind::Module => "section",
            PageKind::Item(_) => "page",
        };
        format!(
            "+++\ntitle = \"{}\"\ntemplate = \"{template}\"\n+++\n\n",
            info.title
        )
    };
    let documentation = project_documentation().markdown(&MarkdownOptions {
        layout: PageLayout::PerItem,
        index_file: "_index.md",
        link_prefix: Some("@/exs/"),
        title_heading: false,
        front_matter: Some(&front_matter),
    });
    let index = &page(&documentation, "_index.md").markdown;
    assert!(index.starts_with(
        "+++\ntitle = \"ExS API Documentation\"\ntemplate = \"section\"\n+++\n\nExS is"
    ));
    assert!(index.contains("- [`std`](@/exs/std/_index.md)"));
    let std = &page(&documentation, "std/_index.md").markdown;
    assert!(std.starts_with("+++\ntitle = \"std\""));
    let range = &page(&documentation, "std/range.md").markdown;
    assert!(range.starts_with("+++\ntitle = \"Range\"\ntemplate = \"page\""));
    assert!(range.contains("[`Iterator`](@/exs/std/iterator.md)"));
}

/// Combines the language specification and a compact reference of the API.
#[test]
fn renders_the_compact_llm_reference() {
    let mut resolver = TestResolver {
        sources: HashMap::from([(
            "./math.exs".to_owned(),
            "/// Adds two integers.\nfn add(left: Int, right: Int) -> Int { ret left + right; }\n\ntype Counter {}\n\nimpl Counter {\n/// Returns the current count.\nfn count(self) -> Int { ret 0; }\n}"
                .to_owned(),
        )]),
    };
    let documentation = match document_with_resolver(
        SourceInput {
            source_id: "./main.exs",
            text: "import \"./math.exs\" as math;\n/// Runs the program.\nfn main() -> Int { ret math::add(20, 22); }",
        },
        &mut resolver,
    ) {
        Ok(documentation) => documentation.llm(),
        Err(error) => panic!("LLM documentation generation failed: {error}"),
    };
    assert!(documentation.starts_with("# Exolynk Script (ExS) LLM Specification"));
    assert!(documentation.contains("# Compact API Reference"));
    assert!(documentation.contains("fn next(self) -> IteratorStep | Error;"));
    assert!(documentation.contains("Adds two integers."));
    assert!(documentation.contains("```exs\n/// Runs the program.\nfn main() -> Int { ... }"));
    assert!(
        documentation.contains("impl Counter {\n    /// Returns the current count.\n    fn count")
    );
    assert!(documentation.contains("type Counter {\n}\n\nimpl Counter {"));
    assert!(
        documentation.contains(
            "/// Opens a runner-registered pull stream selected by a runtime String name."
        )
    );
    assert!(
        documentation.contains("fn Host::stream(name, arguments...) -> HostStream | Error { ... }")
    );
    assert!(documentation.contains("### Type `Int`"));
    assert!(!documentation.contains("__exs_list_any"));
}

/// Links a standard trait implementation and inherits its method documentation.
#[test]
fn documents_standard_add_implementations() {
    let mut resolver = TestResolver {
        sources: HashMap::new(),
    };
    let documentation = match document_with_resolver(
        SourceInput {
            source_id: "./add.exs",
            text: "enum Abc { A, B, } impl Add for Abc { fn add(self, other: Any) -> Any { ret self; } } fn main() { ret Abc::A; }",
        },
        &mut resolver,
    ) {
        Ok(documentation) => documentation.markdown(&MarkdownOptions::default()),
        Err(error) => panic!("documentation generation failed: {error}"),
    };
    let abc = &page(&documentation, "add/index.md").markdown;
    assert!(abc.contains("#### Trait [`Add`](../std/index.md#trait-add)"));
    assert!(abc.contains("Adds the receiver to the evaluated `other` operand."));
}

/// Prefers an implementation method's own documentation over inherited trait documentation.
#[test]
fn prefers_implementation_documentation_over_standard_add_documentation() {
    let mut resolver = TestResolver {
        sources: HashMap::new(),
    };
    let documentation = match document_with_resolver(
        SourceInput {
            source_id: "./custom-add.exs",
            text: "type Counter {} impl Add for Counter {\n/// Adds one application-specific count.\nfn add(self, other: Any) -> Any { ret self; } } fn main() { ret Counter {}; }",
        },
        &mut resolver,
    ) {
        Ok(documentation) => documentation.markdown(&MarkdownOptions::default()),
        Err(error) => panic!("documentation generation failed: {error}"),
    };
    let counter = &page(&documentation, "custom-add/index.md").markdown;
    assert!(counter.contains("Adds one application-specific count."));
    assert!(!counter.contains("Adds the receiver to the evaluated `other` operand."));
}

/// Provides the libraries `log`, `exo`, and `exo::db`.
struct Libraries;

impl ModuleResolver for Libraries {
    /// Rejects file imports.
    fn resolve(&mut self, _importer: &str, path: &str) -> Result<ResolvedSource, String> {
        Err(format!("no file `{path}`"))
    }

    /// Returns the library module at `path`.
    fn library(&mut self, path: &str) -> Option<ResolvedSource> {
        let text = match path {
            "log" => {
                "//! Logging.\n/// The default level.\nconst LEVEL = \"info\";\nprivate const HIDDEN = 1;\n/// Logs an informational message.\nfn info(message: String) -> None { ret None; }"
            }
            "exo" => {
                "//! The Exolynk API.\n/// A stored record.\ntype Record { id: Int }\nprivate fn internal() -> Int { ret 1; }"
            }
            "exo::db" => {
                "/// Loads one record.\nfn load(id: Int) -> exo::Record { ret exo::Record { id: id }; }"
            }
            _ => return None,
        };
        Some(ResolvedSource {
            source_id: path.to_owned(),
            text: text.to_owned(),
        })
    }

    /// Lists every library module.
    fn libraries(&mut self) -> Vec<String> {
        vec!["log".to_owned(), "exo::db".to_owned(), "exo".to_owned()]
    }
}

/// Documents the standard library and every listed library module, sorted by path.
#[test]
fn documents_listed_libraries() {
    let documentation = match document_libraries(&mut Libraries) {
        Ok(documentation) => documentation,
        Err(error) => panic!("library documentation failed: {error}"),
    };
    let names = documentation
        .modules
        .iter()
        .map(|module| module.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["std", "std::test", "exo", "exo::db", "log"]);
    let exo = documentation.module("exo").expect("exo module");
    assert_eq!(exo.description, "The Exolynk API.");
    assert_eq!(exo.items.len(), 1);
    let pages = documentation.markdown(&MarkdownOptions::default());
    let exo = &page(&pages, "exo/index.md").markdown;
    assert!(exo.contains("## Modules\n\n- [`exo::db`](db/index.md)"));
    assert!(!exo.contains("internal"));
    let db = &page(&pages, "exo/db/index.md").markdown;
    assert!(db.contains("fn load(id: Int) -> exo::Record"));
    let log = &page(&pages, "log/index.md").markdown;
    assert!(log.contains("Logging.\n\n## Constants\n\n### `LEVEL`\n\nThe default level."));
    assert!(log.contains("const LEVEL = \"info\";"));
    assert!(log.contains("## Functions\n\n### `info`"));
    assert!(!log.contains("HIDDEN"));
    let llm = documentation.llm();
    assert!(
        llm.contains(
            "### Constants\n\n```exs\n/// The default level.\nconst LEVEL = \"info\";\n```"
        )
    );
}

/// Rejects a listed library the resolver does not provide.
#[test]
fn rejects_missing_listed_libraries() {
    /// Lists a library it does not provide.
    struct Missing;

    impl ModuleResolver for Missing {
        /// Rejects file imports.
        fn resolve(&mut self, _importer: &str, path: &str) -> Result<ResolvedSource, String> {
            Err(format!("no file `{path}`"))
        }

        /// Lists the library `ghost`.
        fn libraries(&mut self) -> Vec<String> {
            vec!["ghost".to_owned()]
        }
    }

    let error = document_libraries(&mut Missing).expect_err("missing library documented");
    assert!(error.contains("library `ghost` is listed"));
}
