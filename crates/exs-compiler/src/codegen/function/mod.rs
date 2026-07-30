//! Direct lowering of ExS function bodies to WebAssembly functions.

mod analysis;
mod control;
mod lowering;
mod runtime;
mod signature;

use std::collections::HashMap;

use wasm_encoder::{Function, Instruction, ValType};

use crate::ast::FunctionDeclaration;
use crate::codegen::diagnostics;
use crate::codegen::source_map::SourceMap;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics};

use analysis::{ROOT_FRAME_RESERVED_LOCALS, count_expressions_block, count_lets};
pub(super) use signature::{FunctionSignature, add_program_types, build_signatures};

/// Structured Wasm targets and lexical cleanup data for one active source loop.
#[derive(Clone, Copy)]
struct LoopContext {
    /// Control-stack depth of the enclosing block exited by break.
    break_depth: u32,
    /// Control-stack depth reached by continue.
    continue_depth: u32,
    /// First lexical scope whose roots must be cleared before a loop branch.
    cleanup_scope_start: usize,
}

/// Lowers one direct ExS function to a Wasm function.
pub(super) struct FunctionCompiler<'a, 'module> {
    declaration: &'module FunctionDeclaration<'a>,
    signatures: &'module HashMap<String, FunctionSignature>,
    runtime: &'module HashMap<String, u32>,
    literals: &'module HashMap<String, u32>,
    source_map: &'module SourceMap<'a>,
    function: Function,
    scopes: Vec<HashMap<String, u32>>,
    loops: Vec<LoopContext>,
    next_local: u32,
    /// Reused local that holds values while validating return contracts.
    return_value_local: u32,
    root_frame_local: u32,
    control_depth: u32,
    function_id: u32,
    return_type: u32,
}

impl<'a, 'module> FunctionCompiler<'a, 'module> {
    /// Prepares direct function lowering with enough ValueRef local slots.
    pub(super) fn new(
        declaration: &'module FunctionDeclaration<'a>,
        signatures: &'module HashMap<String, FunctionSignature>,
        runtime: &'module HashMap<String, u32>,
        literals: &'module HashMap<String, u32>,
        source_map: &'module SourceMap<'a>,
    ) -> Result<Self, CompileDiagnostics<'a>> {
        let expression_locals = count_expressions_block(&declaration.body)
            .checked_mul(3)
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0212",
                    declaration.span,
                    "too many expression temporaries for one function",
                ))
            })?;
        let local_count = count_lets(&declaration.body)
            .checked_add(expression_locals)
            .and_then(|count| count.checked_add(ROOT_FRAME_RESERVED_LOCALS))
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0212",
                    declaration.span,
                    "too many locals for one function",
                ))
            })?;
        let parameter_count = u32::try_from(declaration.parameters.len()).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                declaration.span,
                "too many parameters for one function",
            ))
        })?;
        let root_slot_count = parameter_count.checked_add(local_count).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                declaration.span,
                "too many root slots for one function",
            ))
        })?;
        let root_frame_local = root_slot_count;
        let mut parameters = HashMap::new();
        for (index, parameter) in declaration.parameters.iter().enumerate() {
            parameters.insert(parameter.name.name.clone(), index as u32);
        }
        let mut compiler = Self {
            declaration,
            signatures,
            runtime,
            literals,
            source_map,
            function: Function::new([(local_count + 1, ValType::I32)]),
            scopes: vec![parameters],
            loops: Vec::new(),
            // Reserve one reusable local for return contracts so multiple return paths do not
            // consume additional statically declared Wasm locals.
            next_local: parameter_count + 1,
            return_value_local: parameter_count,
            root_frame_local,
            control_depth: 0,
            function_id: signatures
                .get(&declaration.name.name)
                .map_or(0, |signature| signature.function_id),
            return_type: signatures
                .get(&declaration.name.name)
                .map_or(exs_abi::TYPE_ANY, |signature| signature.return_type),
        };
        compiler.initialize_root_frame(root_slot_count)?;
        Ok(compiler)
    }

    /// Compiles this function body, including the implicit None return path.
    pub(super) fn compile(&mut self) -> Result<Function, CompileDiagnostics<'a>> {
        self.compile_block(&self.declaration.body, false)?;
        self.runtime_call("__exs_rt_none_new", self.declaration.span)?;
        self.validate_return_type(self.declaration.span)?;
        self.return_stack_value()?;
        self.function.instruction(&Instruction::End);
        let placeholder = Function::new([]);
        Ok(std::mem::replace(&mut self.function, placeholder))
    }
}
