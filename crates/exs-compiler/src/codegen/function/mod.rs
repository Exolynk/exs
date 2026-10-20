//! Direct lowering of ExS function bodies to WebAssembly functions.

mod analysis;
mod control;
mod lowering;
mod method;
mod runtime;
mod signature;

use std::collections::HashMap;

use wasm_encoder::{Function, Instruction, ValType};

use crate::ast::FunctionDeclaration;
use crate::codegen::VALUE_TYPE;
use crate::codegen::diagnostics;
use crate::codegen::literals::LiteralPool;
use crate::codegen::source_map::SourceMap;
use crate::codegen::types::{TypeContract, TypeRegistry};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics};

pub(super) use analysis::contains_statements;
pub(super) use method::{InstanceMethod, MethodRegistry};
pub(super) use signature::{
    FunctionSignature, LiftedFunction, add_program_types, build_signatures, validate,
};

/// Number of Wasm i32 scalar locals declared after the ValueRef locals, starting with the root
/// frame base address.
const SCALAR_LOCAL_COUNT: u32 = 6;

/// Immutable module-wide dependencies used while lowering one direct function.
pub(super) struct FunctionCompilerContext<'a, 'module> {
    /// Linked direct function signatures.
    pub(super) signatures: &'module HashMap<String, FunctionSignature>,
    /// Runtime-template export indexes.
    pub(super) runtime: &'module HashMap<String, u32>,
    /// Compiler literal data indexes.
    pub(super) literals: &'module LiteralPool,
    /// Generated source-position metadata.
    pub(super) source_map: &'module SourceMap<'a>,
    /// Module nominal type declarations.
    pub(super) types: &'module TypeRegistry,
    /// Module implementation method dispatch table.
    pub(super) methods: &'module MethodRegistry,
    /// Linear-memory address of the runtime register block.
    pub(super) registers_address: u32,
}

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
    signature_key: String,
    signatures: &'module HashMap<String, FunctionSignature>,
    runtime: &'module HashMap<String, u32>,
    literals: &'module LiteralPool,
    source_map: &'module SourceMap<'a>,
    types: &'module TypeRegistry,
    methods: &'module MethodRegistry,
    function: Function,
    scopes: Vec<HashMap<String, u32>>,
    loops: Vec<LoopContext>,
    next_local: u32,
    /// Reused local that holds values while validating return contracts.
    return_value_local: u32,
    /// Reused unrooted ValueRef local for inline conditions and Int arithmetic results.
    immediate_scratch_local: u32,
    /// Reused Wasm i32 local that combines built-in and nominal type-match results.
    type_match_local: u32,
    /// Reused Wasm i32 local holding the runtime literal buffer address while it is filled.
    literal_pointer_local: u32,
    /// Wasm i32 local holding the packed variadic List length during parameter validation.
    variadic_length_local: u32,
    /// Wasm i32 local holding the current variadic List index during parameter validation.
    variadic_index_local: u32,
    /// First Wasm local holding a ValueRef temporary; earlier non-parameter locals are i32 scalars.
    first_value_local: u32,
    /// Scalar i32 local holding this function's error-trace entry address during entry.
    trace_entry_local: u32,
    root_frame_local: u32,
    control_depth: u32,
    function_id: u32,
    return_type: TypeContract,
    /// Linear-memory address of the runtime register block.
    registers_address: u32,
    /// Spans of `for` statements over Range variables that count in Wasm locals.
    unobserved_range_loops: Vec<crate::diagnostic::SourceSpan<'a>>,
}

impl<'a, 'module> FunctionCompiler<'a, 'module> {
    /// Prepares direct function lowering with enough ValueRef local slots.
    pub(super) fn new(
        declaration: &'module FunctionDeclaration<'a>,
        signature_key: &str,
        context: FunctionCompilerContext<'a, 'module>,
    ) -> Result<Self, CompileDiagnostics<'a>> {
        let parameter_count = u32::try_from(declaration.parameters.len()).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                declaration.span,
                "too many parameters for one function",
            ))
        })?;
        // Scalar i32 locals directly follow the parameters so their indices do not depend on how
        // many ValueRef temporaries the body needs; those are declared once compilation ends.
        let root_frame_local = parameter_count;
        let first_value_local = parameter_count + SCALAR_LOCAL_COUNT;
        let mut parameters = HashMap::new();
        for (index, parameter) in declaration.parameters.iter().enumerate() {
            parameters.insert(parameter.name.name.clone(), index as u32);
        }
        let mut compiler = Self {
            declaration,
            signature_key: signature_key.to_owned(),
            signatures: context.signatures,
            runtime: context.runtime,
            literals: context.literals,
            source_map: context.source_map,
            types: context.types,
            methods: context.methods,
            registers_address: context.registers_address,
            unobserved_range_loops: crate::codegen::range_variables::unobserved_range_loops(
                declaration,
            ),
            // The body is emitted without local declarations; `compile` prepends them.
            function: Function::new([]),
            scopes: vec![parameters],
            loops: Vec::new(),
            // Reserve one reusable local for return contracts so multiple return paths do not
            // consume additional statically declared Wasm locals.
            next_local: first_value_local + 2,
            return_value_local: first_value_local,
            immediate_scratch_local: first_value_local + 1,
            first_value_local,
            type_match_local: root_frame_local + 1,
            literal_pointer_local: root_frame_local + 2,
            variadic_length_local: root_frame_local + 3,
            variadic_index_local: root_frame_local + 4,
            trace_entry_local: root_frame_local + 5,
            root_frame_local,
            control_depth: 0,
            function_id: context
                .signatures
                .get(signature_key)
                .map_or(0, |signature| signature.function_id),
            return_type: context
                .signatures
                .get(signature_key)
                .map(|signature| signature.return_type.clone())
                .ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0999",
                        declaration.name.span,
                        "missing function signature during lowering",
                    ))
                })?,
        };
        compiler.store_parameters()?;
        Ok(compiler)
    }

    /// Compiles this function body, including the implicit None return path.
    pub(super) fn compile(&mut self) -> Result<Function, CompileDiagnostics<'a>> {
        self.compile_block(&self.declaration.body, false)?;
        self.emit_none();
        self.validate_return_type(self.declaration.span)?;
        self.return_stack_value()?;
        self.function.instruction(&Instruction::End);
        // Only now is the number of ValueRef temporaries known, which sizes both the declared
        // locals and the root frame pushed on entry.
        let value_locals = self.next_local - self.first_value_local;
        let mut function = Function::new([
            (SCALAR_LOCAL_COUNT, ValType::I32),
            (value_locals, VALUE_TYPE),
        ]);
        let slot_count = self.root_frame_local + value_locals;
        for instruction in self.function_entry(slot_count)? {
            function.instruction(&instruction);
        }
        let body = std::mem::replace(&mut self.function, Function::new([])).into_raw_body();
        // A body built without locals starts with one byte encoding a zero local-group count.
        function.raw(body.get(1..).unwrap_or_default().iter().copied());
        Ok(function)
    }
}
