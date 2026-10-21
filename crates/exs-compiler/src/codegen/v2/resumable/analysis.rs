//! Which units may suspend, and so run as resumable frames.
//!
//! Unlike v1, which makes every function with a `for` loop or a binding call resumable and all
//! of them once the program has a closure, v2 only lowers units resumable that can reach a
//! suspend point: a host operation or a parallel expression, directly or through calls,
//! dispatch, closure calls of a resumable closure, or `for` loops over a resumable iterator.

use std::collections::HashSet;

use crate::codegen::v2::dispatch::Dispatch;
use crate::hir::HirModule;

/// The call edges and suspend points of one unit.
struct Node {
    /// Unit key.
    key: String,
    /// Whether the unit has a host operation or a parallel expression.
    suspends: bool,
    /// Keys of the units it calls directly or through dispatch.
    calls: Vec<String>,
    /// Whether it calls a binding, which may hold a resumable closure.
    calls_bindings: bool,
    /// Whether it has a `for` loop, whose iterator may be resumable.
    iterates: bool,
}

/// Returns the keys of the units among `reached` that run as resumable frames.
pub(in crate::codegen::v2) fn resumable_units(
    hir: &HirModule<'_>,
    dispatch: &Dispatch,
    reached: &HashSet<&str>,
) -> HashSet<String> {
    let functions = hir.functions().map(|(key, function)| Node {
        key: key.to_owned(),
        suspends: !function.host_calls().is_empty() || !function.parallel_calls().is_empty(),
        calls: function
            .calls()
            .iter()
            .map(|call| call.key.clone())
            .collect(),
        calls_bindings: !function.callable_calls().is_empty(),
        iterates: function.has_for_loops(),
    });
    let closures = hir.closures().map(|closure| Node {
        key: format!("$closure:{}", closure.id().0),
        suspends: !closure.host_calls().is_empty(),
        calls: closure
            .calls()
            .iter()
            .map(|call| call.key.clone())
            .collect(),
        calls_bindings: !closure.callable_calls().is_empty(),
        iterates: false,
    });
    let nodes: Vec<Node> = functions
        .chain(closures)
        .filter(|node| reached.contains(node.key.as_str()))
        .collect();
    let mut resumable: HashSet<String> = nodes
        .iter()
        .filter(|node| node.suspends)
        .map(|node| node.key.clone())
        .collect();
    loop {
        let closure_suspends = resumable.iter().any(|key| key.starts_with("$closure:"));
        let iterator_suspends = dispatch
            .iterator_next()
            .iter()
            .any(|target| resumable.contains(&target.key));
        let before = resumable.len();
        for node in &nodes {
            if resumable.contains(&node.key) {
                continue;
            }
            if node.calls.iter().any(|call| resumable.contains(call))
                || (closure_suspends && node.calls_bindings)
                || (iterator_suspends && node.iterates)
            {
                resumable.insert(node.key.clone());
            }
        }
        if resumable.len() == before {
            return resumable;
        }
    }
}
