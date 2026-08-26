//! Continuation-graph lowering for functions that may suspend through the Host ABI.

mod entry;
mod graph;
mod graph_builder;
mod graph_expression;
mod step;
mod step_calls;

use std::collections::HashMap;

use wasm_encoder::{Function, Instruction, ValType};

use crate::ast::FunctionDeclaration;
use crate::codegen::diagnostics;
use crate::codegen::function::{FunctionSignature, LiftedFunction, MethodRegistry};
use crate::codegen::source_map::SourceMap;
use crate::codegen::types::TypeRegistry;
use crate::codegen::{CompileDiagnostic, CompileDiagnostics};

use self::graph::ContinuationGraph;
use self::step::StepCompiler;

pub(super) use self::entry::{compile_cancel, compile_dispatch, compile_resume, compile_start};

/// One lowered resumable function and the durable frame capacity it requires.
pub(super) struct CompiledContinuation {
    /// The generated one-argument frame step function.
    pub(super) function: Function,
}

/// Compiler-known durable capacity for one suspendable frame.
#[derive(Clone, Copy)]
pub(super) struct FrameLayout {
    /// Compiler-assigned function identifier stored in the runtime frame.
    pub(super) function_id: u32,
    /// Number of durable slots allocated when this function is invoked.
    pub(super) slot_count: u32,
}

/// Returns a conservative durable-frame capacity for one source declaration.
pub(super) fn frame_slot_capacity<'a>(
    declaration: &FunctionDeclaration<'a>,
    capture_count: usize,
) -> Result<u32, CompileDiagnostics<'a>> {
    let source_bytes = declaration
        .span
        .end_byte
        .checked_sub(declaration.span.start_byte)
        .ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0999",
                declaration.span,
                "invalid function source span",
            ))
        })?;
    let parameter_count = declaration
        .parameters
        .len()
        .checked_add(capture_count)
        .and_then(|count| u32::try_from(count).ok())
        .ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                declaration.span,
                "too many continuation function parameters",
            ))
        })?;
    let capacity = source_bytes
        .checked_add(parameter_count)
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                declaration.span,
                "too many continuation frame slots",
            ))
        })?;
    Ok(capacity)
}

/// Lowers a sequential resumable function into a frame-driven Wasm step function.
#[allow(clippy::too_many_arguments)] // The linked compiler dependencies are intentionally explicit.
pub(super) fn compile_function<'source>(
    declaration: &FunctionDeclaration<'source>,
    key: &str,
    signatures: &HashMap<String, FunctionSignature>,
    runtime: &HashMap<String, u32>,
    literals: &HashMap<String, u32>,
    source_map: &SourceMap<'source>,
    frame_layouts: &HashMap<String, FrameLayout>,
    lifted: &[LiftedFunction<'source>],
    methods: &MethodRegistry,
    types: &TypeRegistry,
) -> Result<CompiledContinuation, CompileDiagnostics<'source>> {
    let signature = signatures.get(key).ok_or_else(|| {
        diagnostics(CompileDiagnostic::new(
            "E0999",
            declaration.name.span,
            "missing resumable function signature",
        ))
    })?;
    let graph = ContinuationGraph::build(
        declaration,
        signature,
        signatures,
        frame_layouts,
        lifted,
        methods,
        types,
    )?;
    let mut compiler = StepCompiler {
        runtime,
        literals,
        source_map,
        frame_layouts,
        return_contract: &signature.return_type,
        function: Function::new([(8, ValType::I32)]),
        scratch_local: 1,
        literal_buffer_local: 3,
        variadic_length_local: 7,
        variadic_index_local: 8,
    };
    for (state, operation) in graph.operations.iter().enumerate() {
        let state = u32::try_from(state).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                declaration.span,
                "too many continuation states for one function",
            ))
        })?;
        compiler.emit_state(state, operation)?;
    }
    compiler.function.instruction(&Instruction::Unreachable);
    compiler.function.instruction(&Instruction::End);
    Ok(CompiledContinuation {
        function: compiler.function,
    })
}
