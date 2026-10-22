//! WebAssembly code generation and runtime-template linking.

mod builtin_calls;
mod continuation;
mod entry;
mod function;
mod immediate;
mod linker;
mod literals;
mod match_arms;
mod prune;
pub(crate) mod range_variables;
mod registers;
pub mod source_map;
pub(crate) mod standard;
mod suspension;
pub(crate) mod trait_registry;
mod traits;
mod types;
mod v2;

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
    compile_project_module_for(module, sources, options, crate::Backend::V2)
}

/// Compiles a resolved source graph like [`compile_project_module`] for one backend.
pub(crate) fn compile_project_module_for<'a>(
    module: &mut Module<'a>,
    sources: &[crate::SourceInput<'a>],
    options: CompileOptions,
    backend: crate::Backend,
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
    if backend == crate::Backend::V2 {
        return v2::compile(module, sources, options, &traits_registry);
    }
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
    let (wasm, hints) = linker::link(
        module,
        sources,
        options,
        lifted,
        &suspendable_functions,
        &traits_registry,
    )?;
    prune::prune_unreachable(&wasm, &hints).map_err(|message| {
        CompileDiagnostics::from(CompileDiagnostic::new(
            "E1001",
            module_span(module),
            message,
        ))
    })
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

/// Wasm value type of one `ValueRef` in generated code: a 64-bit tagged reference.
pub(super) const VALUE_TYPE: wasm_encoder::ValType = wasm_encoder::ValType::I64;

/// Returns the `i64.store` operand addressing one root slot relative to its frame base address.
pub(super) fn root_slot_memory_arg(slot: u32) -> wasm_encoder::MemArg {
    wasm_encoder::MemArg {
        offset: u64::from(slot) * 8,
        align: 3,
        memory_index: 0,
    }
}
