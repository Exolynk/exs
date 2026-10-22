//! Continuation graphs of functions that may suspend: the operations of a function in
//! states, which the resumable step functions are emitted from.

mod graph;
mod graph_builder;
mod graph_expression;

use std::collections::HashMap;

use crate::ast::FunctionDeclaration;
use crate::codegen::declarations::{FunctionSignature, LiftedFunction, MethodRegistry};
use crate::codegen::types::TypeRegistry;

pub(in crate::codegen) use self::graph::{
    ComparisonFallback, ContinuationGraph, FormatPart, HostTimeField, Operation, operation_span,
};

/// Compiler-known durable capacity for one suspendable frame.
#[derive(Clone, Copy)]
pub(in crate::codegen) struct FrameLayout {
    /// Compiler-assigned function identifier stored in the runtime frame.
    pub(in crate::codegen) function_id: u32,
    /// Number of durable slots allocated when this function is invoked.
    pub(in crate::codegen) slot_count: u32,
}

/// Returns the exact durable-frame slot count of one resumable function.
///
/// Returns `None` when the continuation graph cannot be built; compiling the function body
/// reports the same diagnostics later.
#[allow(clippy::too_many_arguments)] // Graph construction needs the same linked context.
pub(super) fn frame_slot_count<'source>(
    declaration: &FunctionDeclaration<'source>,
    key: &str,
    signatures: &HashMap<String, FunctionSignature>,
    frame_layouts: &HashMap<String, FrameLayout>,
    lifted: &[LiftedFunction<'source>],
    methods: &MethodRegistry,
    types: &TypeRegistry,
) -> Option<u32> {
    let signature = signatures.get(key)?;
    ContinuationGraph::build(
        declaration,
        signature,
        signatures,
        frame_layouts,
        lifted,
        methods,
        types,
    )
    .ok()
    .map(|graph| graph.slot_count)
}
