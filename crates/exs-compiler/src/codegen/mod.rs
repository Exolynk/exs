//! WebAssembly code generation.
//!
//! Values live in compiler-owned linear memory laid out by `exs-layout`; the linked
//! `exs-core-template` only contributes stateless functions. The code generator compiles the
//! functions reachable from public entry points and reports every construct it does not
//! support as a compile error.

mod assertions;
mod closures;
mod collections;
mod continuation;
mod declarations;
mod dispatch;
mod entry;
mod enums;
mod errors;
mod expressions;
mod function;
mod immediate;
mod intrinsics;
mod iteration;
mod linker;
mod match_arms;
mod methods;
mod objects;
mod prune;
pub(crate) mod range_variables;
mod reachability;
mod resumable;
pub mod source_map;
pub(crate) mod standard;
mod statements;
mod static_keys;
mod template_calls;
pub(crate) mod trait_registry;
mod traits;
mod type_table;
mod types;
mod units;
mod untyped;
mod used_types;
mod validation;
mod wire_schema;

use crate::CompileOptions;
use crate::ast::Module;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// Compiles a resolved source graph into a complete linked Wasm module.
pub fn compile_project_module<'a>(
    module: &mut Module<'a>,
    sources: &[crate::SourceInput<'a>],
    options: CompileOptions,
) -> Result<Vec<u8>, CompileDiagnostics<'a>> {
    let traits_registry = trait_registry::TraitRegistry::build(module);
    let mut diagnostics = types::validate(module);
    diagnostics.extend(traits::validate(module, &traits_registry));
    diagnostics.extend(declarations::validate(module));
    diagnostics.sort_by_span();
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    traits::apply_defaults(module, &traits_registry);
    linker::compile(module, sources, options, &traits_registry)
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
