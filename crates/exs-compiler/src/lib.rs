//! `ExS` source compiler.

use std::fmt;

mod ast;
mod codegen;
mod constants;
mod diagnostic;
mod documentation;
mod formatter;
mod formatter_trivia;
mod highlighting;
mod hir;
mod lexer;
mod limits;
mod loaded_project;
mod module_graph;
mod parser;
mod prelude;
mod sources;

pub use codegen::source_map::{
    DebugInfoError, EmbeddedSource, FunctionDebugInfo, ModuleDebugInfo, SourcePosition,
    read_debug_info,
};
pub use diagnostic::{CompileError, SourceDiagnostic};
pub(crate) use diagnostic::{RelatedSpan, SourceSpan};
pub use documentation::{
    ApiDocumentation, CompactForm, ItemDocs, ItemKind, ItemRef, MarkdownOptions, MemberDocs,
    ModuleDocs, ModuleOrigin, PageInfo, PageKind, PageLayout, SectionDocs, StandardEnum,
    StandardFunction, StandardMethod, StandardNamespace, StandardTrait, StandardType,
    standard_library_enums, standard_library_functions, standard_library_namespace,
    standard_library_namespaces, standard_library_traits, standard_library_type_static_functions,
    standard_library_types,
};
pub use highlighting::{
    HighlightKind, HighlightSpan, SourceComment, SourceLex, SourceToken, SourceTokenKind,
    highlight, source_lex, tokens,
};
pub use limits::CompileLimits;
pub use sources::{Source, Sources};

/// The borrowed source text the compiler passes work on.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SourceInput<'a> {
    /// The human-readable source identity used in diagnostics.
    pub(crate) source_id: &'a str,
    /// UTF-8 `ExS` source text.
    pub(crate) text: &'a str,
}

/// How one source compiles.
#[derive(Clone, Copy, Default)]
#[non_exhaustive]
pub struct CompileOptions<'a> {
    /// Whether to embed the original source text in the `exs.sources` custom section.
    pub embed_sources: bool,
    /// Resource limits of the compilation.
    pub limits: CompileLimits,
    /// Provides the imports and library modules the source names.
    resolver: Option<&'a dyn ModuleResolver>,
}

impl fmt::Debug for CompileOptions<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompileOptions")
            .field("embed_sources", &self.embed_sources)
            .field("limits", &self.limits)
            .field("resolver", &self.resolver.is_some())
            .finish_non_exhaustive()
    }
}

impl<'a> CompileOptions<'a> {
    /// Returns these options with `resolver` providing the imports and library modules; without
    /// one, a source can use neither.
    #[must_use]
    pub fn resolver(mut self, resolver: &'a dyn ModuleResolver) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// Returns the resolver of the compilation.
    pub(crate) fn module_resolver(&self) -> &'a dyn ModuleResolver {
        self.resolver.unwrap_or(&NoResolver)
    }
}

/// A linked, executable `ExS` WebAssembly module.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CompiledModule {
    /// The complete linked WebAssembly module.
    pub wasm: Vec<u8>,
}

/// One test case compiled into a test executable module.
#[derive(Debug, Clone, Eq, PartialEq)]
#[non_exhaustive]
pub struct CompiledTest {
    /// Human-readable test description written in source.
    pub description: String,
}

/// A compiled executable module and its source-declared test cases.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CompiledTests {
    /// The complete linked WebAssembly module with an internal test dispatcher entry point.
    pub wasm: Vec<u8>,
    /// Test cases addressed by their zero-based execution index.
    pub tests: Vec<CompiledTest>,
}

/// One generated Markdown documentation page.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DocumentationPage {
    /// Relative Markdown path within the generated documentation directory.
    pub path: String,
    /// Complete generated Markdown page.
    pub markdown: String,
}

/// Generated Markdown documentation pages.
#[derive(Debug, Clone, Eq, PartialEq)]
#[non_exhaustive]
pub struct Documentation {
    /// The index page, then one page per module and, in the per-item layout, one page per
    /// type, enum, trait, and namespace.
    pub pages: Vec<DocumentationPage>,
}

/// Tells the compiler where the code a source names comes from: the application decides how
/// imports and library modules load, and a script reaches nothing else.
///
/// [`Sources`] implements it for sources known up front.
pub trait ModuleResolver {
    /// Resolves `path` relative to the source identified by `importer`.
    ///
    /// # Errors
    ///
    /// Returns the reason the import cannot load, which the compilation reports at the import.
    fn resolve(&self, importer: &str, path: &str) -> Result<Source<'_>, String>;

    /// Returns the library module at `path`, such as `log` or `app::db`, which every module
    /// may name without importing it, or `None` when the embedder provides no library module
    /// there. Library names start with a lowercase letter.
    fn library(&self, path: &str) -> Option<Source<'_>> {
        let _ = path;
        None
    }

    /// Returns the path of every library module the embedder provides, such as `log` and
    /// `app::db`; [`document_libraries`] documents them. The default provides none.
    fn libraries(&self) -> Vec<String> {
        Vec::new()
    }
}

/// The resolver of a compilation without one, which provides nothing.
struct NoResolver;

impl ModuleResolver for NoResolver {
    fn resolve(&self, _importer: &str, _path: &str) -> Result<Source<'_>, String> {
        Err("this compilation has no resolver; see `CompileOptions::resolver`".to_owned())
    }
}

/// Compiles an entry source and everything the resolver of `options` provides for it.
///
/// # Errors
///
/// Returns the rendered report and the located diagnostics when parsing, resolution, or
/// compilation fails.
pub fn compile(
    source: Source<'_>,
    options: CompileOptions<'_>,
) -> Result<CompiledModule, CompileError> {
    module_graph::compile(source.input(), options)
}

/// Compiles the test dispatcher of a source like [`compile`], with an internal entry point for
/// the tests the entry source declares.
///
/// # Errors
///
/// Returns the rendered report and the located diagnostics when parsing, resolution, or
/// compilation fails.
pub fn compile_tests(
    source: Source<'_>,
    options: CompileOptions<'_>,
) -> Result<CompiledTests, CompileError> {
    module_graph::compile_tests(source.input(), options)
}

/// Returns source-declared test descriptions in declaration order.
///
/// # Errors
///
/// Returns the rendered report and the located diagnostics when the source is invalid.
pub fn test_descriptions(source: Source<'_>) -> Result<Vec<String>, CompileError> {
    let source = source.input();
    let lexed = lexer::lex(source);
    if !lexed.diagnostics.is_empty() {
        return Err(CompileError::from_diagnostics(
            &lexed.diagnostics,
            source.text,
        ));
    }
    parser::parse(source.source_id, lexed.tokens, false)
        .map(|module| {
            module
                .tests
                .into_iter()
                .map(|test| test.description)
                .collect()
        })
        .map_err(|error| CompileError::from_diagnostics(&error, source.text))
}

/// Checks the lexical and syntactic validity of one source unit.
///
/// This is the light check an editor runs while typing: it neither resolves imports nor
/// compiles, and it does not require `fn main()`. Semantic errors are reported by the
/// compilation functions.
#[must_use]
pub fn check_syntax(source: Source<'_>) -> Vec<SourceDiagnostic> {
    let source = source.input();
    let lexed = lexer::lex(source);
    let mut diagnostics = lexed.diagnostics;
    if let Err(syntax) = parser::parse(source.source_id, lexed.tokens, false) {
        diagnostics.extend(syntax);
    }
    diagnostics.sort_by_span();
    SourceDiagnostic::located_in(&diagnostics, source.source_id)
}

/// Formats one syntactically valid ExS source unit into the canonical source layout.
///
/// Unlike compilation, formatting accepts an imported module without `fn main(...)`.
///
/// # Errors
///
/// Returns the rendered report and the located diagnostics when the input cannot be formatted
/// safely.
pub fn format(source: Source<'_>) -> Result<String, CompileError> {
    let source = source.input();
    formatter::format(source).map_err(|error| CompileError::from_diagnostics(&error, source.text))
}

/// Documents the standard library, `source`, every module it imports, and every library module
/// they name; `options` provide the resolver and limits.
///
/// Render the result with [`ApiDocumentation::markdown`] or [`ApiDocumentation::llm`]; neither
/// touches the filesystem.
///
/// # Errors
///
/// Returns the rendered report when source loading or parsing fails.
pub fn document(
    source: Source<'_>,
    options: CompileOptions<'_>,
) -> Result<ApiDocumentation, CompileError> {
    documentation::project(source.input(), options)
}

/// Documents the standard library and every library module the resolver of `options` lists.
///
/// # Errors
///
/// Returns the rendered report when a listed library is missing or does not parse.
pub fn document_libraries(options: CompileOptions<'_>) -> Result<ApiDocumentation, CompileError> {
    documentation::libraries(options)
}

impl ApiDocumentation {
    /// Renders the documentation as Markdown pages laid out by `options`.
    #[must_use]
    pub fn markdown(&self, options: &MarkdownOptions<'_>) -> Documentation {
        documentation::render_markdown(self, options)
    }

    /// Renders the language specification followed by a compact reference of the
    /// documentation, for an LLM context window.
    #[must_use]
    pub fn llm(&self) -> String {
        documentation::render_llm(self)
    }

    /// Renders the language specification, the complete `full_modules` and an index of the
    /// other modules' declarations, for an LLM that looks up further entries with `llm_items`.
    #[must_use]
    pub fn llm_core(&self, full_modules: &[&str]) -> String {
        documentation::render_llm_core(self, full_modules)
    }

    /// Renders the requested entries of the compact reference: whole modules,
    /// `module::name` paths or bare names. Returns the reference and the unmatched names.
    #[must_use]
    pub fn llm_items(&self, names: &[&str]) -> (String, Vec<String>) {
        documentation::render_llm_items(self, names)
    }
}
