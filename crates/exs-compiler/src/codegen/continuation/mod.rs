//! Continuation-graph lowering for functions that may suspend through the Host ABI.

mod body;
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
use crate::codegen::literals::LiteralPool;
use crate::codegen::source_map::SourceMap;
use crate::codegen::types::TypeRegistry;
use crate::codegen::{CompileDiagnostic, CompileDiagnostics};

use self::body::StepBody;
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

/// Lowers a sequential resumable function into a frame-driven Wasm step function.
#[allow(clippy::too_many_arguments)] // The linked compiler dependencies are intentionally explicit.
pub(super) fn compile_function<'source>(
    declaration: &FunctionDeclaration<'source>,
    key: &str,
    signatures: &HashMap<String, FunctionSignature>,
    runtime: &HashMap<String, u32>,
    literals: &LiteralPool,
    source_map: &SourceMap<'source>,
    frame_layouts: &HashMap<String, FrameLayout>,
    lifted: &[LiftedFunction<'source>],
    methods: &MethodRegistry,
    types: &TypeRegistry,
    registers_address: u32,
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
        registers_address,
        return_contract: &signature.return_type,
        // Locals 1..=9 are i32 scalars, 10..=13 hold ValueRefs and 14..=15 frame block addresses.
        function: StepBody::new(Function::new([
            (9, ValType::I32),
            (4, crate::codegen::VALUE_TYPE),
            (2, ValType::I32),
        ])),
        scratch_local: 10,
        value_local: 11,
        operand_local: 12,
        int_result_local: 13,
        completion_local: 1,
        literal_buffer_local: 3,
        variadic_length_local: 7,
        variadic_index_local: 8,
        state_local: 9,
        block_local: 14,
        child_block_local: 15,
        used_slots: 0,
        loop_depth: 0,
        state_count: 0,
        current_state: 0,
    };
    compiler.emit_dispatch(&graph.operations, declaration.span)?;
    // Generated code addresses slots without bounds checks, so the allocated frame must cover
    // every slot the body touches.
    let capacity = frame_layouts.get(key).map_or(0, |layout| layout.slot_count);
    if compiler.used_slots > capacity || graph.slot_count > capacity {
        return Err(diagnostics(CompileDiagnostic::new(
            "E0999",
            declaration.span,
            "continuation frame layout does not cover every slot",
        )));
    }
    compiler.function.instruction(&Instruction::Unreachable);
    compiler.function.instruction(&Instruction::End);
    Ok(CompiledContinuation {
        function: compiler.function.finish(),
    })
}
