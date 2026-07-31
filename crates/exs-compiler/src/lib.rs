//! `ExS` Phase-1 source compiler.

mod ast;
mod codegen;
mod diagnostic;
mod hir;
mod lexer;
mod module_graph;
mod parser;

pub use codegen::source_map::{
    DebugInfoError, EmbeddedSource, FunctionDebugInfo, ModuleDebugInfo, SourcePosition,
    read_debug_info,
};
pub use diagnostic::{
    CompileDiagnostic, CompileDiagnosticCategory, CompileDiagnostics, RelatedSpan, SourceSpan,
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
    let wasm = codegen::compile_module(&mut module, source.text, options)?;
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
