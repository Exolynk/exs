//! Transitive suspendability analysis over resolved source call edges.

use std::collections::HashSet;

use crate::hir::{ClosureOwner, HirModule};

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
                (!function.host_calls().is_empty()
                    || !function.parallel_calls().is_empty()
                    || function.has_matches()
                    || function.has_for_loops())
                .then_some(key.to_owned())
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

    /// Marks every direct source callable as frame-backed for closure dynamic invocation.
    pub(super) fn include_all<'a>(&mut self, functions: impl Iterator<Item = &'a str>) {
        self.functions
            .extend(functions.map(std::borrow::ToOwned::to_owned));
    }
}

/// Verifies the binding identities consumed by continuation-frame lowering.
fn validate_hir(hir: &HirModule<'_>) {
    let mut bindings = HashSet::new();
    for (key, function) in hir.functions() {
        debug_assert!(hir.function(key).is_some());
        for binding in function.bindings() {
            debug_assert!(bindings.insert(binding.id));
            debug_assert!(!binding.name.is_empty());
            debug_assert!(binding.span.start_byte <= binding.span.end_byte);
        }
        for reference in function.references() {
            if let Some(binding) = reference.binding {
                debug_assert!(bindings.contains(&binding));
            }
            debug_assert!(reference.span.start_byte <= reference.span.end_byte);
        }
        for call in function.calls() {
            debug_assert!(call.span.start_byte <= call.span.end_byte);
        }
        for call in function.callable_calls() {
            debug_assert!(bindings.contains(&call.binding));
            debug_assert!(call.span.start_byte <= call.span.end_byte);
        }
        for host_call in function.host_calls() {
            debug_assert!(host_call.span.start_byte <= host_call.span.end_byte);
        }
        for parallel in function.parallel_calls() {
            debug_assert!(parallel.start_byte <= parallel.end_byte);
        }
    }
    for closure in hir.closures() {
        match closure.owner() {
            ClosureOwner::Function(key) => debug_assert!(hir.function(key).is_some()),
            ClosureOwner::Closure(parent) => debug_assert!(parent.0 < closure.id().0),
        }
        for binding in closure.bindings() {
            debug_assert!(bindings.insert(binding.id));
            debug_assert!(!binding.name.is_empty());
            debug_assert!(binding.span.start_byte <= binding.span.end_byte);
        }
        for parameter in closure.parameters() {
            debug_assert!(
                closure
                    .bindings()
                    .iter()
                    .any(|binding| binding.id == *parameter)
            );
        }
        for reference in closure.references() {
            if let Some(binding) = reference.binding {
                debug_assert!(bindings.contains(&binding));
            }
            debug_assert!(reference.span.start_byte <= reference.span.end_byte);
        }
        for capture in closure.captures() {
            debug_assert!(bindings.contains(&capture.binding));
            debug_assert!(capture.span.start_byte <= capture.span.end_byte);
        }
        for call in closure.calls() {
            debug_assert!(call.span.start_byte <= call.span.end_byte);
        }
        for call in closure.callable_calls() {
            debug_assert!(bindings.contains(&call.binding));
            debug_assert!(call.span.start_byte <= call.span.end_byte);
        }
        for host_call in closure.host_calls() {
            debug_assert!(host_call.span.start_byte <= host_call.span.end_byte);
        }
    }
}
