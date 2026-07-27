//! `ExS` Phase-1 source compiler.

mod ast;
mod codegen;
mod diagnostic;
mod lexer;
mod parser;

pub use diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// Immutable source input supplied to the compiler.
#[derive(Debug, Clone, Copy)]
pub struct SourceInput<'a> {
    /// The human-readable source identity used in diagnostics.
    pub source_id: &'a str,
    /// UTF-8 `ExS` source text.
    pub text: &'a str,
}

/// Compiler options for the initial implementation.
#[derive(Debug, Clone, Copy, Default)]
pub struct CompileOptions;

/// A linked, executable `ExS` WebAssembly module.
#[derive(Debug, Clone)]
pub struct CompiledModule {
    /// The complete linked WebAssembly module.
    pub wasm: Vec<u8>,
}

/// Compiles one Phase-1 `ExS` module against a runtime template.
pub fn compile<'a>(
    source: SourceInput<'a>,
    _options: CompileOptions,
) -> Result<CompiledModule, CompileDiagnostics<'a>> {
    let tokens = lexer::lex(source).map_err(CompileDiagnostics::from)?;
    let module = parser::parse(source.source_id, tokens)?;
    let wasm = codegen::compile_module(&module)?;
    Ok(CompiledModule { wasm })
}
