//! Which units may suspend, and so run as resumable frames.
//!
//! Unlike v1, which makes every function with a `for` loop or a binding call resumable and all
//! of them once the program has a closure, the code generator only lowers units resumable that can reach a
//! suspend point: a host operation or a parallel expression, directly or through calls,
//! dispatch, closure calls of a resumable closure, or `for` loops over a resumable iterator.

use std::collections::{HashMap, HashSet};

use crate::ast::Expression;
use crate::codegen::closures::closure_span_key;
use crate::codegen::dispatch::Dispatch;
use crate::codegen::reachability::visit_block;
use crate::codegen::units::Unit;
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

/// Returns the units among `reached` that `par` makes resumable: the units with a `par`
/// expression, which wait for their children, and the closures `par` starts as tasks, which
/// run as frames. `closures` maps closure spans to their keys.
///
/// A static `par` starts the closures it lists; a dynamic `par` may start any closure.
pub(in crate::codegen) fn parallel_units(
    reached: &[&Unit<'_, '_>],
    closures: &HashMap<(String, u32, u32), String>,
) -> HashSet<String> {
    let mut units = HashSet::new();
    let mut dynamic = false;
    for unit in reached {
        visit_block(&unit.declaration.body, &mut |expression| match expression {
            Expression::ParallelStatic { tasks, .. } => {
                units.insert(unit.key.clone());
                for task in tasks {
                    match task {
                        Expression::Closure { span, .. } => {
                            if let Some(key) = closures.get(&closure_span_key(*span)) {
                                units.insert(key.clone());
                            }
                        }
                        _ => dynamic = true,
                    }
                }
            }
            Expression::ParallelDynamic { .. } => {
                units.insert(unit.key.clone());
                dynamic = true;
            }
            _ => {}
        });
    }
    if dynamic {
        units.extend(
            reached
                .iter()
                .filter(|unit| unit.key.starts_with("$closure:"))
                .map(|unit| unit.key.clone()),
        );
    }
    units
}

/// Returns the keys of the units among `reached` that run as resumable frames, starting from
/// the units that suspend through host operations and the `parallel` units.
pub(in crate::codegen) fn resumable_units(
    hir: &HirModule<'_>,
    dispatch: &Dispatch,
    reached: &HashSet<&str>,
    parallel: &HashSet<String>,
) -> HashSet<String> {
    let functions = hir.functions().map(|(key, function)| Node {
        key: key.to_owned(),
        suspends: function.calls_host() || !function.parallel_calls().is_empty(),
        calls: function
            .calls()
            .iter()
            .map(|call| call.key.clone())
            .collect(),
        calls_bindings: function.calls_bindings(),
        iterates: function.has_for_loops(),
    });
    let closures = hir.closures().map(|closure| Node {
        key: format!("$closure:{}", closure.id().0),
        suspends: closure.calls_host(),
        calls: closure
            .calls()
            .iter()
            .map(|call| call.key.clone())
            .collect(),
        calls_bindings: closure.calls_bindings(),
        iterates: false,
    });
    let nodes: Vec<Node> = functions
        .chain(closures)
        .filter(|node| reached.contains(node.key.as_str()))
        .collect();
    let mut resumable: HashSet<String> = nodes
        .iter()
        .filter(|node| node.suspends || parallel.contains(&node.key))
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
