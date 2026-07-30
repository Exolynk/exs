//! WebAssembly code generation and runtime-template linking.

mod entry;
mod function;
mod linker;
mod literals;
pub mod source_map;
mod types;

use crate::CompileOptions;
use crate::ast::Module;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// Compiles a parsed module into a complete linked Wasm module.
pub fn compile_module<'a>(
    module: &Module<'a>,
    source: &'a str,
    options: CompileOptions,
) -> Result<Vec<u8>, CompileDiagnostics<'a>> {
    linker::link(module, source, options)
}

/// Returns a diagnostic span covering the module's first function.
pub(super) fn module_span<'a>(module: &Module<'a>) -> SourceSpan<'a> {
    module
        .functions
        .first()
        .map_or_else(|| SourceSpan::empty("<unknown>"), |function| function.span)
}

/// Wraps one diagnostic in the compiler's diagnostic collection.
pub(super) fn diagnostics(diagnostic: CompileDiagnostic<'_>) -> CompileDiagnostics<'_> {
    CompileDiagnostics::from(diagnostic)
}
