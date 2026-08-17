//! Lowering of the multi-argument Wasm entry wrapper.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, Instruction, ValType};

use super::function::FunctionSignature;
use super::{diagnostics, module_span};
use crate::ast::Module;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics};

/// Lowers the stable Wasm entry point that invokes the source `main` function.
pub(super) fn compile_start<'a>(
    module: &Module<'a>,
    main: &FunctionSignature,
    runtime: &HashMap<String, u32>,
) -> Result<Function, CompileDiagnostics<'a>> {
    let fixed_parameter_count = u32::try_from(main.arity).map_err(|_| {
        diagnostics(CompileDiagnostic::new(
            "E0212",
            module_span(module),
            "too many main parameters for the Wasm i32 ABI",
        ))
    })?;
    let parameter_count = u32::try_from(main.wasm_arity()).map_err(|_| {
        diagnostics(CompileDiagnostic::new(
            "E0212",
            module_span(module),
            "too many main parameters for the Wasm i32 ABI",
        ))
    })?;
    let root_slot_count = parameter_count.checked_add(2).ok_or_else(|| {
        diagnostics(CompileDiagnostic::new(
            "E0212",
            module_span(module),
            "too many main root slots",
        ))
    })?;
    let local_count = parameter_count.checked_add(6).ok_or_else(|| {
        diagnostics(CompileDiagnostic::new(
            "E0212",
            module_span(module),
            "too many main entry locals",
        ))
    })?;
    let root_frame_local = 2_u32;
    let arguments_local = 3_u32;
    let argument_count_local = 4_u32;
    let first_parameter_local = 5_u32;
    let result_local = first_parameter_local
        .checked_add(parameter_count)
        .ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                module_span(module),
                "too many main parameters for the Wasm i32 ABI",
            ))
        })?;
    let variadic_list_local = result_local + 1;
    let variadic_index_local = result_local + 2;
    let mut function = Function::new([(local_count, ValType::I32)]);

    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::LocalGet(1));
    call_runtime(&mut function, runtime, "__exs_rt_decode_input", module)?;
    function.instruction(&Instruction::LocalSet(arguments_local));

    function.instruction(&Instruction::I32Const(root_slot_count.cast_signed()));
    call_runtime(&mut function, runtime, "__exs_rt_root_push", module)?;
    function.instruction(&Instruction::LocalSet(root_frame_local));
    set_root_slot(
        &mut function,
        runtime,
        module,
        root_frame_local,
        0,
        arguments_local,
    )?;

    function.instruction(&Instruction::LocalGet(arguments_local));
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_input_argument_count",
        module,
    )?;
    function.instruction(&Instruction::LocalSet(argument_count_local));
    if !main.variadic {
        function.instruction(&Instruction::LocalGet(argument_count_local));
        function.instruction(&Instruction::I32Const(fixed_parameter_count.cast_signed()));
        function.instruction(&Instruction::I32GtU);
        function.instruction(&Instruction::If(BlockType::Empty));
        function.instruction(&Instruction::LocalGet(arguments_local));
        call_runtime(&mut function, runtime, "__exs_rt_input_arity_error", module)?;
        function.instruction(&Instruction::LocalSet(result_local));
        set_root_slot(
            &mut function,
            runtime,
            module,
            root_frame_local,
            root_slot_count - 1,
            result_local,
        )?;
        function.instruction(&Instruction::LocalGet(result_local));
        call_runtime(&mut function, runtime, "__exs_rt_set_result", module)?;
        function.instruction(&Instruction::LocalGet(root_frame_local));
        call_runtime(&mut function, runtime, "__exs_rt_root_pop", module)?;
        function.instruction(&Instruction::I32Const(exs_abi::STATUS_COMPLETE));
        function.instruction(&Instruction::Return);
        function.instruction(&Instruction::End);
    }

    for index in 0..fixed_parameter_count {
        let parameter_local = first_parameter_local + index;
        function.instruction(&Instruction::LocalGet(arguments_local));
        function.instruction(&Instruction::I32Const(index.cast_signed()));
        call_runtime(&mut function, runtime, "__exs_rt_input_argument", module)?;
        function.instruction(&Instruction::LocalSet(parameter_local));
        set_root_slot(
            &mut function,
            runtime,
            module,
            root_frame_local,
            index + 1,
            parameter_local,
        )?;
    }
    if main.variadic {
        function.instruction(&Instruction::Call(
            *runtime.get("__exs_rt_list_new").ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0209",
                    module_span(module),
                    "missing runtime List constructor",
                ))
            })?,
        ));
        function.instruction(&Instruction::LocalSet(variadic_list_local));
        set_root_slot(
            &mut function,
            runtime,
            module,
            root_frame_local,
            fixed_parameter_count + 1,
            variadic_list_local,
        )?;
        function.instruction(&Instruction::I32Const(fixed_parameter_count.cast_signed()));
        function.instruction(&Instruction::LocalSet(variadic_index_local));
        function.instruction(&Instruction::Block(BlockType::Empty));
        function.instruction(&Instruction::Loop(BlockType::Empty));
        function.instruction(&Instruction::LocalGet(variadic_index_local));
        function.instruction(&Instruction::LocalGet(argument_count_local));
        function.instruction(&Instruction::I32GeU);
        function.instruction(&Instruction::BrIf(1));
        function.instruction(&Instruction::LocalGet(variadic_list_local));
        function.instruction(&Instruction::LocalGet(arguments_local));
        function.instruction(&Instruction::LocalGet(variadic_index_local));
        call_runtime(&mut function, runtime, "__exs_rt_input_argument", module)?;
        call_runtime(&mut function, runtime, "__exs_rt_append", module)?;
        function.instruction(&Instruction::Drop);
        function.instruction(&Instruction::LocalGet(variadic_index_local));
        function.instruction(&Instruction::I32Const(1));
        function.instruction(&Instruction::I32Add);
        function.instruction(&Instruction::LocalSet(variadic_index_local));
        function.instruction(&Instruction::Br(0));
        function.instruction(&Instruction::End);
        function.instruction(&Instruction::End);
        let parameter_local = first_parameter_local + fixed_parameter_count;
        function.instruction(&Instruction::LocalGet(variadic_list_local));
        function.instruction(&Instruction::LocalSet(parameter_local));
        set_root_slot(
            &mut function,
            runtime,
            module,
            root_frame_local,
            fixed_parameter_count + 1,
            parameter_local,
        )?;
    }
    for index in 0..parameter_count {
        function.instruction(&Instruction::LocalGet(first_parameter_local + index));
    }
    function.instruction(&Instruction::Call(main.index));
    function.instruction(&Instruction::LocalSet(result_local));
    set_root_slot(
        &mut function,
        runtime,
        module,
        root_frame_local,
        root_slot_count - 1,
        result_local,
    )?;
    function.instruction(&Instruction::LocalGet(result_local));
    call_runtime(&mut function, runtime, "__exs_rt_set_result", module)?;
    function.instruction(&Instruction::LocalGet(root_frame_local));
    call_runtime(&mut function, runtime, "__exs_rt_root_pop", module)?;
    function.instruction(&Instruction::I32Const(exs_abi::STATUS_COMPLETE));
    function.instruction(&Instruction::End);
    Ok(function)
}

/// Roots one ValueRef local in the generated entry wrapper.
fn set_root_slot<'a>(
    function: &mut Function,
    runtime: &HashMap<String, u32>,
    module: &Module<'a>,
    frame_local: u32,
    slot: u32,
    value_local: u32,
) -> Result<(), CompileDiagnostics<'a>> {
    function.instruction(&Instruction::LocalGet(frame_local));
    function.instruction(&Instruction::I32Const(slot.cast_signed()));
    function.instruction(&Instruction::LocalGet(value_local));
    call_runtime(function, runtime, "__exs_rt_root_set", module)
}

/// Emits one linked runtime call required by the generated entry wrapper.
fn call_runtime<'a>(
    function: &mut Function,
    runtime: &HashMap<String, u32>,
    name: &str,
    module: &Module<'a>,
) -> Result<(), CompileDiagnostics<'a>> {
    let index = runtime.get(name).copied().ok_or_else(|| {
        diagnostics(CompileDiagnostic::new(
            "E0999",
            module_span(module),
            format!("runtime template does not export {name}"),
        ))
    })?;
    function.instruction(&Instruction::Call(index));
    Ok(())
}
