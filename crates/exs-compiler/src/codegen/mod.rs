//! WebAssembly code generation and runtime-template linking.

mod continuation;
mod entry;
mod function;
mod linker;
mod literals;
pub mod source_map;
pub(crate) mod standard;
mod suspension;
pub(crate) mod trait_registry;
mod traits;
mod types;

use crate::CompileOptions;
use crate::ast::Module;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};
use crate::hir::HirModule;

/// Compiles a resolved source graph into a complete linked Wasm module.
pub fn compile_project_module<'a>(
    module: &mut Module<'a>,
    sources: &[crate::SourceInput<'a>],
    options: CompileOptions,
) -> Result<Vec<u8>, CompileDiagnostics<'a>> {
    let traits_registry = trait_registry::TraitRegistry::build(module);
    let mut diagnostics = types::validate(module);
    diagnostics.extend(traits::validate(module, &traits_registry));
    diagnostics.extend(function::validate(module));
    diagnostics.sort_by_span();
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    traits::apply_defaults(module, &traits_registry);
    let hir = HirModule::lower(module, &traits_registry);
    let mut suspendability = suspension::Suspendability::analyze(&hir);
    if hir.closures().next().is_some() {
        suspendability.include_all(hir.functions().map(|(key, _)| key));
    }
    let _has_suspendable_function = suspendability.has_any();
    let _main_is_suspendable = suspendability.contains("main");
    let lifted = linker::lifted_functions(module, &hir);
    let suspendable_functions = suspendability.functions().clone();
    drop(hir);
    linker::link(
        module,
        sources,
        options,
        lifted,
        &suspendable_functions,
        &traits_registry,
    )
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
