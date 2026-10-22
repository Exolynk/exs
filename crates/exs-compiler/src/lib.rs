//! `ExS` source compiler.

mod ast;
mod codegen;
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

pub use codegen::source_map::{
    DebugInfoError, EmbeddedSource, FunctionDebugInfo, ModuleDebugInfo, SourcePosition,
    read_debug_info,
};
pub use diagnostic::{
    CompileDiagnostic, CompileDiagnosticCategory, CompileDiagnostics, RelatedSpan, SourceSpan,
};
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

/// Immutable source input supplied to the compiler.
#[derive(Debug, Clone, Copy)]
pub struct SourceInput<'a> {
    /// The human-readable source identity used in diagnostics.
    pub source_id: &'a str,
    /// UTF-8 `ExS` source text.
    pub text: &'a str,
}

/// Compiler options controlling optional executable-module metadata.
#[derive(Debug, Clone, Copy, Default)]
pub struct CompileOptions {
    /// Whether to embed the original source text in the `exs.sources` custom section.
    pub embed_sources: bool,
    /// Resource limits of the compilation.
    pub limits: CompileLimits,
}

/// A linked, executable `ExS` WebAssembly module.
#[derive(Debug, Clone)]
pub struct CompiledModule {
    /// The complete linked WebAssembly module.
    pub wasm: Vec<u8>,
}

/// One test case compiled into a test executable module.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CompiledTest {
    /// Human-readable test description written in source.
    pub description: String,
}

/// A compiled executable module and its source-declared test cases.
#[derive(Debug, Clone)]
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
pub struct Documentation {
    /// The index page, then one page per module and, in the per-item layout, one page per
    /// type, enum, trait, and namespace.
    pub pages: Vec<DocumentationPage>,
}

/// One source file returned by a module resolver.
#[derive(Debug, Clone)]
pub struct ResolvedSource {
    /// Canonical, stable identity of the resolved source file.
    pub source_id: String,
    /// UTF-8 `ExS` source text.
    pub text: String,
}

/// Loads a relative import without coupling the compiler to a storage system.
pub trait ModuleResolver {
    /// Resolves `path` relative to the source identified by `importer`.
    fn resolve(&mut self, importer: &str, path: &str) -> Result<ResolvedSource, String>;

    /// Returns the library module at `path`, such as `log` or `exo::db`, which every module
    /// may name without importing it, or `None` when the embedder provides no library module
    /// there. Library names start with a lowercase letter.
    fn library(&mut self, path: &str) -> Option<ResolvedSource> {
        let _ = path;
        None
    }

    /// Returns the path of every library module the embedder provides, such as `log` and
    /// `exo::db`; [`document_libraries`] documents them. The default provides none.
    fn libraries(&mut self) -> Vec<String> {
        Vec::new()
    }
}

/// Compiles one `ExS` module against the core library template.
pub fn compile<'a>(
    source: SourceInput<'a>,
    options: CompileOptions,
) -> Result<CompiledModule, CompileDiagnostics<'a>> {
    if source.text.len() > options.limits.max_source_bytes {
        return Err(CompileDiagnostics::from(CompileDiagnostic::new(
            "E0010",
            SourceSpan::empty(source.source_id),
            format!(
                "source exceeds the limit of {} bytes",
                options.limits.max_source_bytes
            ),
        )));
    }
    let lexed = lexer::lex(source);
    let mut diagnostics = lexed.diagnostics;
    let parsed = parser::parse_with_nesting(
        source.source_id,
        lexed.tokens,
        true,
        options.limits.max_nesting,
    );
    let mut module = match parsed {
        Ok(module) => module,
        Err(parser_diagnostics) => {
            diagnostics.extend(parser_diagnostics);
            diagnostics.sort_by_span();
            return Err(diagnostics);
        }
    };
    if !diagnostics.is_empty() {
        diagnostics.sort_by_span();
        return Err(diagnostics);
    }
    module_graph::hide_prelude_private(&mut module);
    let mut prelude = prelude::parse()?;
    prelude.types.append(&mut module.types);
    prelude.enums.append(&mut module.enums);
    prelude.traits.append(&mut module.traits);
    prelude.implementations.append(&mut module.implementations);
    let mut functions = std::mem::take(&mut module.functions);
    functions.append(&mut prelude.functions);
    prelude.functions = functions;
    let mut sources = prelude::source_inputs();
    sources.push(source);
    let wasm = codegen::compile_project_module(&mut prelude, &sources, options)?;
    Ok(CompiledModule { wasm })
}

/// Compiles an entry source and every file reachable through its imports.
///
/// The resolver owns path resolution, source loading, and canonical source identities.
///
/// # Errors
///
/// Returns a rendered diagnostic report when parsing, resolution, or compilation fails.
pub fn compile_with_resolver<R: ModuleResolver>(
    source: SourceInput<'_>,
    options: CompileOptions,
    resolver: &mut R,
) -> Result<CompiledModule, String> {
    module_graph::compile(source, options, resolver)
}

/// Determines whether one source unit declares at least one source test.
///
/// # Errors
///
/// Returns rendered lexer or parser diagnostics when the source is invalid.
pub fn has_tests(source: SourceInput<'_>) -> Result<bool, String> {
    let lexed = lexer::lex(source);
    if !lexed.diagnostics.is_empty() {
        return Err(lexed.diagnostics.render(source.text));
    }
    parser::parse(source.source_id, lexed.tokens, false)
        .map(|module| !module.tests.is_empty())
        .map_err(|error| error.render(source.text))
}

/// Returns source-declared test descriptions in declaration order.
///
/// # Errors
///
/// Returns rendered lexer or parser diagnostics when the source is invalid.
pub fn test_descriptions(source: SourceInput<'_>) -> Result<Vec<String>, String> {
    let lexed = lexer::lex(source);
    if !lexed.diagnostics.is_empty() {
        return Err(lexed.diagnostics.render(source.text));
    }
    parser::parse(source.source_id, lexed.tokens, false)
        .map(|module| {
            module
                .tests
                .into_iter()
                .map(|test| test.description)
                .collect()
        })
        .map_err(|error| error.render(source.text))
}

/// Compiles one source graph with an internal entry point for source-declared tests.
///
/// # Errors
///
/// Returns a rendered diagnostic report when parsing, resolution, or compilation fails.
pub fn compile_tests_with_resolver<R: ModuleResolver>(
    source: SourceInput<'_>,
    options: CompileOptions,
    resolver: &mut R,
) -> Result<CompiledTests, String> {
    module_graph::compile_tests(source, options, resolver)
}

/// Formats one syntactically valid ExS source unit into the canonical source layout.
///
/// Unlike compilation, formatting accepts an imported module without `fn main(...)`.
///
/// # Errors
///
/// Returns lexer or parser diagnostics when the input cannot be formatted safely.
pub fn format<'a>(source: SourceInput<'a>) -> Result<String, CompileDiagnostics<'a>> {
    formatter::format(source)
}

/// Documents the standard library, `source`, every module it imports, and every library
/// module they name.
///
/// The resolver owns import loading and canonical source identities. Render the result with
/// [`ApiDocumentation::markdown`] or [`ApiDocumentation::llm`]; neither touches the
/// filesystem.
///
/// # Errors
///
/// Returns a rendered diagnostic report when source loading or parsing fails.
pub fn document_with_resolver<R: ModuleResolver>(
    source: SourceInput<'_>,
    resolver: &mut R,
) -> Result<ApiDocumentation, String> {
    documentation::project(source, resolver)
}

/// Documents the standard library and every library module [`ModuleResolver::libraries`]
/// lists.
///
/// # Errors
///
/// Returns a rendered diagnostic report when a listed library is missing or does not parse.
pub fn document_libraries<R: ModuleResolver>(resolver: &mut R) -> Result<ApiDocumentation, String> {
    documentation::libraries(resolver)
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
}
