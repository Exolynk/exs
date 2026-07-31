//! WebAssembly code generation and runtime-template linking.

mod continuation;
mod entry;
mod function;
mod linker;
mod literals;
pub mod source_map;
mod suspension;
mod traits;
mod types;

use crate::CompileOptions;
use crate::ast::Module;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};
use crate::hir::HirModule;

/// Compiles a parsed module into a complete linked Wasm module.
pub fn compile_module<'a>(
    module: &mut Module<'a>,
    source: &'a str,
    options: CompileOptions,
) -> Result<Vec<u8>, CompileDiagnostics<'a>> {
    let mut diagnostics = types::validate(module);
    diagnostics.extend(traits::validate(module));
    diagnostics.extend(function::validate(module));
    diagnostics.sort_by_span();
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    traits::apply_defaults(module);
    let hir = HirModule::lower(module);
    let suspendability = suspension::Suspendability::analyze(&hir);
    let _has_suspendable_function = suspendability.has_any();
    let _main_is_suspendable = suspendability.contains("main");
    linker::link(module, source, options, suspendability.functions())
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
