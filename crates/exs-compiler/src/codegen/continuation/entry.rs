//! Runner-facing wrappers and dispatch generation for resumable functions.

use std::collections::HashMap;

use exs_abi::{STATUS_COMPLETE, STATUS_PENDING, STATUS_READY};
use wasm_encoder::{BlockType, Function, Instruction, ValType};

use super::FrameLayout;
use crate::codegen::diagnostics;
use crate::codegen::function::FunctionSignature;
use crate::codegen::{CompileDiagnostic, CompileDiagnostics, SourceSpan, module_span};

/// Generates the root entry wrapper for a resumable `main` function.
pub(crate) fn compile_start<'a>(
    module: &crate::ast::Module<'a>,
    main: &FunctionSignature,
    frame_slot_count: u32,
    dispatcher: u32,
    runtime: &HashMap<String, u32>,
) -> Result<Function, CompileDiagnostics<'a>> {
    let mut function = Function::new([(5, ValType::I32)]);
    let frame = 2_u32;
    let arguments = 3_u32;
    let count = 4_u32;
    let result = 5_u32;
    let status = 6_u32;
    let parameter_count = i32::try_from(main.arity).map_err(|_| {
        diagnostics(CompileDiagnostic::new(
            "E0212",
            module_span(module),
            "too many main parameters for the Wasm i32 ABI",
        ))
    })?;
    let frame_slot_count = i32::try_from(frame_slot_count).map_err(|_| {
        diagnostics(CompileDiagnostic::new(
            "E0212",
            module_span(module),
            "too many continuation frame slots for the Wasm i32 ABI",
        ))
    })?;

    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::LocalGet(1));
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_decode_input",
        module_span(module),
    )?;
    function.instruction(&Instruction::LocalSet(arguments));
    function.instruction(&Instruction::LocalGet(arguments));
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_input_argument_count",
        module_span(module),
    )?;
    function.instruction(&Instruction::LocalSet(count));
    function.instruction(&Instruction::LocalGet(count));
    function.instruction(&Instruction::I32Const(parameter_count));
    function.instruction(&Instruction::I32GtU);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(arguments));
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_input_arity_error",
        module_span(module),
    )?;
    function.instruction(&Instruction::LocalSet(result));
    function.instruction(&Instruction::LocalGet(result));
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_set_result",
        module_span(module),
    )?;
    function.instruction(&Instruction::I32Const(STATUS_COMPLETE));
    function.instruction(&Instruction::Return);
    function.instruction(&Instruction::End);

    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_execution_start",
        module_span(module),
    )?;
    function.instruction(&Instruction::I32Const(main.function_id.cast_signed()));
    function.instruction(&Instruction::I32Const(frame_slot_count));
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_async_frame_new",
        module_span(module),
    )?;
    function.instruction(&Instruction::LocalSet(frame));
    function.instruction(&Instruction::I32Const(main.function_id.cast_signed()));
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_frame_push",
        module_span(module),
    )?;
    for index in 0..parameter_count {
        function.instruction(&Instruction::LocalGet(frame));
        function.instruction(&Instruction::I32Const(index));
        function.instruction(&Instruction::LocalGet(arguments));
        function.instruction(&Instruction::I32Const(index));
        call_runtime(
            &mut function,
            runtime,
            "__exs_rt_input_argument",
            module_span(module),
        )?;
        call_runtime(
            &mut function,
            runtime,
            "__exs_rt_async_frame_set_slot",
            module_span(module),
        )?;
    }
    function.instruction(&Instruction::LocalGet(frame));
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_async_frame_set_current",
        module_span(module),
    )?;
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_scheduler_checkpoint",
        module_span(module),
    )?;
    emit_dispatch(
        &mut function,
        dispatcher,
        status,
        result,
        runtime,
        module_span(module),
    )
}

/// Generates the runner-facing export that resumes the active host-call continuation.
pub(crate) fn compile_resume<'a>(
    module: &crate::ast::Module<'a>,
    dispatcher: u32,
    runtime: &HashMap<String, u32>,
) -> Result<Function, CompileDiagnostics<'a>> {
    let mut function = Function::new([(3, ValType::I32)]);
    let status = 4_u32;
    let result = 5_u32;
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::LocalGet(1));
    function.instruction(&Instruction::LocalGet(2));
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_host_call_resume",
        module_span(module),
    )?;
    function.instruction(&Instruction::LocalSet(status));
    function.instruction(&Instruction::LocalGet(status));
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Else);
    function.instruction(&Instruction::I32Const(exs_abi::STATUS_CANCELLED));
    function.instruction(&Instruction::Return);
    function.instruction(&Instruction::End);
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_scheduler_checkpoint",
        module_span(module),
    )?;
    emit_dispatch(
        &mut function,
        dispatcher,
        status,
        result,
        runtime,
        module_span(module),
    )
}

/// Generates the runner-facing export that cancels a suspended root execution.
pub(crate) fn compile_cancel<'a>(
    module: &crate::ast::Module<'a>,
    runtime: &HashMap<String, u32>,
) -> Result<Function, CompileDiagnostics<'a>> {
    let mut function = Function::new([]);
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_execution_cancel",
        module_span(module),
    )?;
    function.instruction(&Instruction::End);
    Ok(function)
}

/// Generates one module-wide step dispatcher selected by the active frame function id.
pub(crate) fn compile_dispatch<'a>(
    module: &crate::ast::Module<'a>,
    signatures: &HashMap<String, FunctionSignature>,
    suspendable: &HashMap<String, FrameLayout>,
    runtime: &HashMap<String, u32>,
) -> Result<Function, CompileDiagnostics<'a>> {
    let mut function = Function::new([(2, ValType::I32)]);
    function.instruction(&Instruction::Call(runtime_index(
        runtime,
        "__exs_rt_async_frame_current",
        module_span(module),
    )?));
    function.instruction(&Instruction::LocalSet(0));
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::Call(runtime_index(
        runtime,
        "__exs_rt_async_frame_function",
        module_span(module),
    )?));
    function.instruction(&Instruction::LocalSet(1));
    let mut targets = suspendable
        .iter()
        .filter_map(|(key, layout)| {
            signatures
                .get(key)
                .map(|signature| (layout.function_id, signature.index))
        })
        .collect::<Vec<_>>();
    targets.sort_unstable_by_key(|(function_id, _)| *function_id);
    emit_dispatch_target(&mut function, &targets, 0);
    function.instruction(&Instruction::End);
    Ok(function)
}

/// Emits nested Wasm conditionals selecting a generated suspendable step function.
fn emit_dispatch_target(function: &mut Function, targets: &[(u32, u32)], index: usize) {
    let Some((function_id, step)) = targets.get(index) else {
        function.instruction(&Instruction::Unreachable);
        return;
    };
    function.instruction(&Instruction::LocalGet(1));
    function.instruction(&Instruction::I32Const(function_id.cast_signed()));
    function.instruction(&Instruction::I32Eq);
    function.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::Call(*step));
    function.instruction(&Instruction::Else);
    emit_dispatch_target(function, targets, index + 1);
    function.instruction(&Instruction::End);
}

/// A sequence of operations executed through durable frame slots.
fn emit_dispatch<'a>(
    function: &mut Function,
    dispatcher: u32,
    status: u32,
    result: u32,
    runtime: &HashMap<String, u32>,
    span: SourceSpan<'a>,
) -> Result<Function, CompileDiagnostics<'a>> {
    function.instruction(&Instruction::Block(BlockType::Result(ValType::I32)));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    function.instruction(&Instruction::Call(dispatcher));
    function.instruction(&Instruction::LocalSet(status));
    function.instruction(&Instruction::LocalGet(status));
    function.instruction(&Instruction::I32Const(STATUS_READY));
    function.instruction(&Instruction::I32Eq);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Br(1));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::LocalGet(status));
    function.instruction(&Instruction::I32Const(STATUS_PENDING));
    function.instruction(&Instruction::I32Eq);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::I32Const(STATUS_PENDING));
    function.instruction(&Instruction::Br(2));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::LocalGet(status));
    function.instruction(&Instruction::I32Const(STATUS_COMPLETE));
    function.instruction(&Instruction::I32Ne);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);
    call_runtime(
        function,
        runtime,
        "__exs_rt_async_frame_take_completed",
        span,
    )?;
    function.instruction(&Instruction::LocalSet(result));
    function.instruction(&Instruction::LocalGet(result));
    call_runtime(function, runtime, "__exs_rt_set_result", span)?;
    function.instruction(&Instruction::I32Const(STATUS_COMPLETE));
    function.instruction(&Instruction::Br(1));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
    Ok(std::mem::replace(function, Function::new([])))
}

/// Emits one runtime call used by entry and resume wrappers.
fn call_runtime<'a>(
    function: &mut Function,
    runtime: &HashMap<String, u32>,
    name: &str,
    span: SourceSpan<'a>,
) -> Result<(), CompileDiagnostics<'a>> {
    function.instruction(&Instruction::Call(runtime_index(runtime, name, span)?));
    Ok(())
}

/// Resolves one stable runtime ABI export index.
pub(super) fn runtime_index<'a>(
    runtime: &HashMap<String, u32>,
    name: &str,
    span: SourceSpan<'a>,
) -> Result<u32, CompileDiagnostics<'a>> {
    runtime.get(name).copied().ok_or_else(|| {
        diagnostics(CompileDiagnostic::new(
            "E0209",
            span,
            format!("runtime template does not export `{name}`"),
        ))
    })
}
