//! Diagnostics for the functions the backend does not compile.
//!
//! The code generator only compiles units reachable from public entry points. To report errors such as unknown bindings or a
//! `break` outside a loop in source functions nothing calls yet, those functions are lowered
//! into continuation graphs, which check the same rules, and only their diagnostics are kept.
//! The prelude is left out: it is valid, and lowering it would only cost compile time.

use std::collections::{HashMap, HashSet};

use super::units::Unit;
use crate::ast::Module;
use crate::codegen::continuation::{ContinuationGraph, FrameLayout};
use crate::codegen::declarations::{LiftedFunction, MethodRegistry, build_signatures};
use crate::codegen::trait_registry::TraitRegistry;
use crate::codegen::types::TypeRegistry;
use crate::diagnostic::CompileDiagnostics;

/// Source identities of the prelude start with this prefix.
const PRELUDE_SOURCE_PREFIX: &str = "<std>/";

/// Returns the diagnostics of every source unit of `units` outside `reached`.
pub(super) fn unreached_diagnostics<'a>(
    module: &Module<'a>,
    traits: &TraitRegistry<'a>,
    types: &TypeRegistry,
    lifted: &[LiftedFunction<'a>],
    units: &[Unit<'_, 'a>],
    reached: &HashSet<&str>,
) -> CompileDiagnostics<'a> {
    let mut diagnostics = CompileDiagnostics::new();
    let unreached: Vec<_> = units
        .iter()
        .filter(|unit| {
            !reached.contains(unit.key.as_str())
                && !unit
                    .declaration
                    .span
                    .source_id
                    .starts_with(PRELUDE_SOURCE_PREFIX)
        })
        .collect();
    if unreached.is_empty() {
        return diagnostics;
    }
    let signatures = match build_signatures(module, lifted, types) {
        Ok(signatures) => signatures,
        Err(error) => return error,
    };
    let methods = match MethodRegistry::build(module, traits, types, &signatures) {
        Ok(methods) => methods,
        Err(error) => return error,
    };
    // Closures inside the unreached units need a layout like they would in a resumable unit.
    let frame_layouts: HashMap<String, FrameLayout> = signatures
        .iter()
        .filter(|(key, _)| key.starts_with("$closure:"))
        .map(|(key, signature)| {
            (
                key.clone(),
                FrameLayout {
                    function_id: signature.function_id,
                    slot_count: 0,
                },
            )
        })
        .collect();
    for unit in unreached {
        let Some(signature) = signatures.get(&unit.key) else {
            continue;
        };
        if let Err(error) = ContinuationGraph::build(
            unit.declaration,
            signature,
            &signatures,
            &frame_layouts,
            lifted,
            &methods,
            types,
            false,
        ) {
            diagnostics.extend(error);
        }
    }
    diagnostics
}
