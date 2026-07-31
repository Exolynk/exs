//! Transitive suspendability analysis over resolved source call edges.

use std::collections::HashSet;

use crate::hir::HirModule;

/// The functions that require resumable lowering because they may reach a suspend point.
pub(super) struct Suspendability {
    functions: HashSet<String>,
}

impl Suspendability {
    /// Computes the fixed point of direct host-call and direct function-call reachability.
    #[must_use]
    pub(super) fn analyze(hir: &HirModule<'_>) -> Self {
        validate_hir(hir);
        let mut functions = hir
            .functions()
            .filter_map(|(key, function)| {
                (!function.host_calls().is_empty()).then_some(key.to_owned())
            })
            .collect::<HashSet<_>>();
        loop {
            let mut changed = false;
            for (key, function) in hir.functions() {
                if functions.contains(key) {
                    continue;
                }
                if function
                    .calls()
                    .iter()
                    .any(|call| functions.contains(&call.key))
                {
                    changed |= functions.insert(key.to_owned());
                }
            }
            if !changed {
                return Self { functions };
            }
        }
    }

    /// Returns whether the named direct function or implementation method may suspend.
    #[must_use]
    pub(super) fn contains(&self, key: &str) -> bool {
        self.functions.contains(key)
    }

    /// Returns whether this module contains any potential suspend point.
    #[must_use]
    pub(super) fn has_any(&self) -> bool {
        !self.functions.is_empty()
    }

    /// Returns the compiler keys of all functions requiring resumable lowering.
    #[must_use]
    pub(super) fn functions(&self) -> &HashSet<String> {
        &self.functions
    }
}

/// Verifies the binding identities consumed by continuation-frame lowering.
fn validate_hir(hir: &HirModule<'_>) {
    for (key, function) in hir.functions() {
        debug_assert!(hir.function(key).is_some());
        for (index, binding) in function.bindings().iter().enumerate() {
            debug_assert_eq!(binding.id.0 as usize, index);
            debug_assert!(!binding.name.is_empty());
            debug_assert!(binding.span.start_byte <= binding.span.end_byte);
        }
        for reference in function.references() {
            if let Some(binding) = reference.binding {
                debug_assert!((binding.0 as usize) < function.bindings().len());
            }
            debug_assert!(reference.span.start_byte <= reference.span.end_byte);
        }
        for call in function.calls() {
            debug_assert!(call.span.start_byte <= call.span.end_byte);
        }
        for host_call in function.host_calls() {
            debug_assert!(host_call.span.start_byte <= host_call.span.end_byte);
        }
    }
}
