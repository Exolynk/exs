//! `ExS` Phase-1 source compiler.

mod ast;
mod codegen;
mod diagnostic;
mod documentation;
mod formatter;
mod formatter_trivia;
mod highlighting;
mod hir;
mod lexer;
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
    StandardEnum, StandardFunction, StandardMethod, StandardNamespace, StandardTrait, StandardType,
    standard_library_enums, standard_library_functions, standard_library_namespace,
    standard_library_namespaces, standard_library_traits, standard_library_types,
};
pub use highlighting::{
    HighlightKind, HighlightSpan, SourceComment, SourceLex, SourceToken, SourceTokenKind,
    highlight, source_lex, tokens,
};

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
}

/// A linked, executable `ExS` WebAssembly module.
#[derive(Debug, Clone)]
pub struct CompiledModule {
    /// The complete linked WebAssembly module.
    pub wasm: Vec<u8>,
}

/// One generated Markdown documentation page.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DocumentationPage {
    /// Relative Markdown path within the generated documentation directory.
    pub path: String,
    /// Complete generated Markdown page.
    pub markdown: String,
}

/// Generated Markdown documentation for one resolved ExS module graph.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Documentation {
    /// Project overview and language reference page.
    pub index: String,
    /// Module, type, enum, trait, and function API pages.
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
}

/// Compiles one Phase-1 `ExS` module against a runtime template.
pub fn compile<'a>(
    source: SourceInput<'a>,
    options: CompileOptions,
) -> Result<CompiledModule, CompileDiagnostics<'a>> {
    let lexed = lexer::lex(source);
    let mut diagnostics = lexed.diagnostics;
    let mut module = match parser::parse(source.source_id, lexed.tokens, true) {
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
    let mut prelude = prelude::parse()?;
    prelude.types.append(&mut module.types);
    prelude.enums.append(&mut module.enums);
    prelude.traits.append(&mut module.traits);
    prelude.implementations.append(&mut module.implementations);
    prelude.functions.append(&mut module.functions);
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

/// Generates Markdown language and API documentation for a resolved module graph.
///
/// The resolver owns import loading and canonical source identities. The returned documentation
/// contains no filesystem side effects, so callers decide where and whether to write pages.
///
/// # Errors
///
/// Returns a rendered diagnostic report when source loading or parsing fails.
pub fn document_with_resolver<R: ModuleResolver>(
    source: SourceInput<'_>,
    resolver: &mut R,
) -> Result<Documentation, String> {
    documentation::generate(source, resolver)
}
