//! Lowering of one source function for the v2 backend.
//!
//! Wasm locals: the parameters (one `ValueRef` each), then one `i32` holding the function's
//! root-frame base, one `i32` holding its trace entry, then `i64` locals. A local holding a
//! `ValueRef` owns a root-frame slot that is written whenever the local is, so the collector
//! sees every live value; scalar `i64` locals such as loop counters own no slot. The frame and
//! the trace entry are pushed on entry by bumping the root and trace stacks in the VM state
//! block and popped before every return.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use exs_layout::VmState;
use wasm_encoder::{BlockType, Function, Instruction, MemArg, ValType};

use super::errors::emit_push_trace;
use super::protocols::Protocols;
use crate::ast::FunctionDeclaration;
use crate::codegen::source_map::SourceMap;
use crate::codegen::types::{TypeContract, TypeRegistry};
use crate::codegen::{VALUE_TYPE, immediate};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// Compile-time facts about one compiled source function.
pub(super) struct FunctionInfo {
    /// Wasm function index.
    pub(super) index: u32,
    /// Identifier recorded in Error traces, the function's position in the module like v1.
    pub(super) function_id: u32,
    /// Number of parameters.
    pub(super) arity: u32,
    /// Contract of each parameter.
    pub(super) parameters: Vec<TypeContract>,
    /// Contract of the returned value.
    pub(super) return_contract: TypeContract,
}

/// Module-wide inputs of function lowering.
pub(super) struct Context<'m> {
    /// Wasm indices of the core template's `__exs_core_*` exports.
    pub(super) core: &'m HashMap<String, u32>,
    /// Every compiled source function by name.
    pub(super) functions: &'m HashMap<String, FunctionInfo>,
    /// Index of the generated `(i64) -> i64` helper that boxes one Int.
    pub(super) int_new: u32,
    /// Index of the generated `(i64) -> i64` helper that boxes one Float from its bits.
    pub(super) float_new: u32,
    /// Raw `ValueRef` of every static String literal.
    pub(super) strings: &'m HashMap<String, u64>,
    /// Address of the VM state block.
    pub(super) vm: u32,
    /// Resolved source types.
    pub(super) types: &'m TypeRegistry,
    /// Address of the static key array of every Object literal key sequence.
    pub(super) object_keys: &'m HashMap<Vec<String>, u32>,
    /// Index of the next mutable `i32` global, appended after the template's globals.
    pub(super) next_global: Cell<u32>,
    /// Source position ids of the module, shared with runtime v1.
    pub(super) source_map: &'m SourceMap<'m>,
    /// Address of the static source-position arrays of `+` chains.
    pub(super) positions_base: u32,
    /// The `u32` source positions of every `+` chain, back to back.
    pub(super) positions: RefCell<Vec<u32>>,
    /// Nominal types with standard-trait implementations the backend must not bypass.
    pub(super) protocols: &'m Protocols,
}

/// Lowering state of one function body.
pub(super) struct FunctionCompiler<'a, 'm> {
    /// Module-wide inputs.
    pub(super) context: &'m Context<'m>,
    /// The source declaration.
    pub(super) declaration: &'m FunctionDeclaration<'a>,
    /// Body instructions, emitted without local declarations.
    pub(super) function: Function,
    /// Visible bindings, innermost last.
    pub(super) scopes: Vec<HashMap<String, u32>>,
    /// Root-frame slot of every `ValueRef` local.
    slots: HashMap<u32, u32>,
    /// Number of parameters.
    parameter_count: u32,
    /// Next free local index.
    next_local: u32,
    /// Unrooted scratch local for short-lived values such as Int results.
    pub(super) scratch: u32,
    /// Break and continue control depths of the enclosing loops.
    pub(super) loops: Vec<(u32, u32)>,
    /// Current structured-control depth inside the body.
    pub(super) depth: u32,
    /// First of two consecutive rooted locals passed to two-operand core calls.
    pub(super) pair: Option<u32>,
}

impl<'a, 'm> FunctionCompiler<'a, 'm> {
    /// Prepares lowering of `declaration`.
    pub(super) fn new(context: &'m Context<'m>, declaration: &'m FunctionDeclaration<'a>) -> Self {
        let parameter_count = declaration.parameters.len() as u32;
        let mut scope = HashMap::new();
        let mut slots = HashMap::new();
        for (index, parameter) in (0_u32..).zip(&declaration.parameters) {
            scope.insert(parameter.name.name.clone(), index);
            slots.insert(index, index);
        }
        Self {
            context,
            declaration,
            function: Function::new([]),
            scopes: vec![scope],
            slots,
            parameter_count,
            // Local `parameter_count` is the frame base, then the trace entry and the scratch
            // local.
            next_local: parameter_count + 3,
            scratch: parameter_count + 2,
            loops: Vec::new(),
            depth: 0,
            pair: None,
        }
    }

    /// Returns the local holding the root-frame base address.
    pub(super) const fn frame_local(&self) -> u32 {
        self.parameter_count
    }

    /// Returns the local holding the address of this function's trace entry.
    pub(super) const fn trace_local(&self) -> u32 {
        self.parameter_count + 1
    }

    /// Allocates one `ValueRef` local with its own root slot.
    pub(super) fn value_local(&mut self) -> u32 {
        let local = self.next_local;
        self.next_local += 1;
        let slot = self.slots.len() as u32;
        self.slots.insert(local, slot);
        local
    }

    /// Allocates one unrooted `i64` local.
    pub(super) fn scalar_local(&mut self) -> u32 {
        let local = self.next_local;
        self.next_local += 1;
        local
    }

    /// Allocates `count` rooted locals whose root slots are consecutive.
    pub(super) fn consecutive_locals(&mut self, count: u32) -> u32 {
        let first = self.value_local();
        for _ in 1..count {
            self.value_local();
        }
        first
    }

    /// Pushes the root-slot address of the rooted local `local`.
    pub(super) fn push_slot_address(&mut self, local: u32) {
        let slot = self.slots.get(&local).copied().unwrap_or_default();
        self.instruction(Instruction::LocalGet(self.frame_local()));
        self.instruction(Instruction::I32Const((slot * 8).cast_signed()));
        self.instruction(Instruction::I32Add);
    }

    /// Pops the stack's `ValueRef` into a new rooted local.
    pub(super) fn store_new(&mut self) -> u32 {
        let local = self.value_local();
        self.store(local);
        local
    }

    /// Pops the stack's `ValueRef` into `local` and its root slot.
    pub(super) fn store(&mut self, local: u32) {
        self.instruction(Instruction::LocalSet(local));
        if let Some(slot) = self.slots.get(&local).copied() {
            self.instruction(Instruction::LocalGet(self.frame_local()));
            self.instruction(Instruction::LocalGet(local));
            self.instruction(Instruction::I64Store(slot_memory(slot)));
        }
    }

    /// Returns the local bound to `name`.
    pub(super) fn lookup(&self, name: &str) -> Option<u32> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    /// Emits one instruction.
    pub(super) fn instruction(&mut self, instruction: Instruction<'_>) {
        match instruction {
            Instruction::Block(_) | Instruction::Loop(_) | Instruction::If(_) => self.depth += 1,
            Instruction::End => self.depth = self.depth.saturating_sub(1),
            _ => {}
        }
        self.function.instruction(&instruction);
    }

    /// Calls the core template export `name`.
    pub(super) fn core_call(
        &mut self,
        name: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let index = self.context.core.get(name).copied().ok_or_else(|| {
            error(
                "E1001",
                span,
                format!("the v2 core template does not export `{name}`"),
            )
        })?;
        self.instruction(Instruction::Call(index));
        Ok(())
    }

    /// Allocates one mutable `i32` global initialized to 0, such as an inline-cache entry.
    pub(super) fn new_global(&self) -> u32 {
        let index = self.context.next_global.get();
        self.context.next_global.set(index + 1);
        index
    }

    /// Pushes the VM state block address as an `i32`.
    pub(super) fn push_vm(&mut self) {
        self.instruction(Instruction::I32Const(self.context.vm.cast_signed()));
    }

    /// Returns from the function with the declared-type TypeError unless the `ValueRef` in
    /// `local` satisfies `contract`, like v1.
    ///
    /// Nominal Object types are tested through the type id in the reference's extra bits.
    pub(super) fn check_contract(
        &mut self,
        local: u32,
        contract: &TypeContract,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if !contract.enum_type_ids.is_empty() || contract.list_item.is_some() {
            return Err(unsupported(span, "enum and List element type contracts"));
        }
        if contract.builtin_mask == exs_abi::TYPE_ANY {
            return Ok(());
        }
        self.instruction(Instruction::LocalGet(local));
        immediate::type_mask_test(&mut self.function, contract.builtin_mask);
        for type_id in &contract.nominal_type_ids {
            self.instruction(Instruction::LocalGet(local));
            if !immediate::object_type_test(&mut self.function, *type_id) {
                return Err(unsupported(
                    span,
                    "contracts of more than 65534 nominal types",
                ));
            }
            self.instruction(Instruction::I32Or);
        }
        self.instruction(Instruction::I32Eqz);
        self.instruction(Instruction::If(BlockType::Empty));
        self.return_type_mismatch(local, span)?;
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Pops the root frame and returns the `ValueRef` on the stack after its contract check.
    pub(super) fn return_value(
        &mut self,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let value = self.scratch;
        self.instruction(Instruction::LocalSet(value));
        let contract = self
            .context
            .functions
            .get(&self.declaration.name.name)
            .map(|info| info.return_contract.clone())
            .ok_or_else(|| error("E0999", span, "missing v2 function signature".to_owned()))?;
        self.check_contract(value, &contract, span)?;
        self.pop_frame();
        self.instruction(Instruction::LocalGet(value));
        self.instruction(Instruction::Return);
        Ok(())
    }

    /// Restores the root and trace stacks to this function's entry state.
    pub(super) fn pop_frame(&mut self) {
        self.instruction(Instruction::I32Const(0));
        self.instruction(Instruction::LocalGet(self.frame_local()));
        self.instruction(Instruction::I32Store(vm_field(
            self.context.vm,
            root_stack_end_offset(),
        )));
        self.instruction(Instruction::I32Const(0));
        self.instruction(Instruction::LocalGet(self.trace_local()));
        self.instruction(Instruction::I32Store(vm_field(
            self.context.vm,
            core::mem::offset_of!(VmState, errors.trace_top) as u32,
        )));
    }

    /// Branches to the structured-control depth `target`.
    pub(super) fn branch_to(&mut self, target: u32) {
        self.instruction(Instruction::Br(self.depth - target));
    }

    /// Compiles the whole function into a Wasm body with its locals and frame prologue.
    pub(super) fn compile(mut self) -> Result<Function, CompileDiagnostics<'a>> {
        let parameters = self
            .context
            .functions
            .get(&self.declaration.name.name)
            .map(|info| info.parameters.clone())
            .ok_or_else(|| {
                error(
                    "E0999",
                    self.declaration.span,
                    "missing v2 function signature".to_owned(),
                )
            })?;
        for ((local, contract), parameter) in
            (0_u32..).zip(&parameters).zip(&self.declaration.parameters)
        {
            self.check_contract(local, contract, parameter.name.span)?;
        }
        self.compile_block(&self.declaration.body)?;
        immediate::push(&mut self.function, exs_value::ValueRef::NONE);
        self.return_value(self.declaration.span)?;
        self.instruction(Instruction::End);
        let i64_locals = self.next_local - self.parameter_count - 2;
        let mut function = Function::new([(2, ValType::I32), (i64_locals, VALUE_TYPE)]);
        let slot_count = self.slots.len() as u32;
        emit_push_frame(
            &mut function,
            self.context.vm,
            self.frame_local(),
            slot_count,
        );
        for index in 0..self.parameter_count {
            function.instruction(&Instruction::LocalGet(self.frame_local()));
            function.instruction(&Instruction::LocalGet(index));
            function.instruction(&Instruction::I64Store(slot_memory(index)));
        }
        let function_id = self
            .context
            .functions
            .get(&self.declaration.name.name)
            .map_or(0, |info| info.function_id);
        emit_push_trace(
            &mut function,
            self.context.vm,
            self.trace_local(),
            function_id,
        );
        let body = self.function.into_raw_body();
        // A body built without locals starts with one byte encoding a zero local-group count.
        function.raw(body.get(1..).unwrap_or_default().iter().copied());
        Ok(function)
    }
}

/// Emits the root-frame push of `slot_count` zeroed slots into `frame_local`.
///
/// Traps when the root stack is full.
pub(super) fn emit_push_frame(function: &mut Function, vm: u32, frame_local: u32, slot_count: u32) {
    let end = vm_field(vm, root_stack_end_offset());
    let limit = vm_field(vm, core::mem::offset_of!(VmState, root_stack.limit) as u32);
    let bytes = (slot_count * 8).cast_signed();
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::I32Load(end));
    function.instruction(&Instruction::LocalTee(frame_local));
    function.instruction(&Instruction::I32Const(bytes));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::I32Load(limit));
    function.instruction(&Instruction::I32GtU);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::LocalGet(frame_local));
    function.instruction(&Instruction::I32Const(bytes));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::I32Store(end));
    if slot_count > 0 {
        function.instruction(&Instruction::LocalGet(frame_local));
        function.instruction(&Instruction::I32Const(0));
        function.instruction(&Instruction::I32Const(bytes));
        function.instruction(&Instruction::MemoryFill(0));
    }
}

/// Byte offset of the root stack's next free slot inside the VM state block.
pub(super) fn root_stack_end_offset() -> u32 {
    core::mem::offset_of!(VmState, root_stack.range.end) as u32
}

/// Returns the memory operand of one `u32` VM field, addressed from 0.
pub(super) fn vm_field(vm: u32, offset: u32) -> MemArg {
    MemArg {
        offset: u64::from(vm + offset),
        align: 2,
        memory_index: 0,
    }
}

/// Returns the memory operand of root slot `slot` relative to the frame base.
pub(super) fn slot_memory(slot: u32) -> MemArg {
    MemArg {
        offset: u64::from(slot) * 8,
        align: 3,
        memory_index: 0,
    }
}

/// Builds one diagnostic collection.
pub(super) fn error<'a>(
    code: &'static str,
    span: SourceSpan<'a>,
    message: String,
) -> CompileDiagnostics<'a> {
    CompileDiagnostics::from(CompileDiagnostic::new(code, span, message))
}

/// Reports a construct the v2 backend does not support yet.
pub(super) fn unsupported<'a>(span: SourceSpan<'a>, what: &str) -> CompileDiagnostics<'a> {
    error(
        "E0600",
        span,
        format!("{what} are not supported by the v2 backend yet"),
    )
}
