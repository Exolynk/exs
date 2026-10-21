//! Lowering of one source function for the v2 backend.
//!
//! Wasm locals: the parameters (one `ValueRef` each), then one `i32` holding the function's
//! root-frame base, one `i32` holding its trace entry, then `i64` locals. A local holding a
//! `ValueRef` owns a root-frame slot that is written whenever the local is, so the collector
//! sees every live value; scalar `i64` locals such as loop counters own no slot. The frame and
//! the trace entry are pushed on entry by bumping the root and trace stacks in the VM state
//! block and popped before every return.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use exs_layout::VmState;
use wasm_encoder::{BlockType, Function, Instruction, MemArg, ValType};

use super::closures::ClosureInfo;
use super::dispatch::Dispatch;
use super::errors::emit_push_trace;
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
    /// Number of fixed parameters, without captures and a variadic parameter.
    pub(super) arity: u32,
    /// Contract of each fixed parameter.
    pub(super) parameters: Vec<TypeContract>,
    /// Element contract of the trailing variadic parameter, if any.
    pub(super) variadic: Option<TypeContract>,
    /// Number of captured Cells a lifted closure takes before its parameters.
    pub(super) captures: u32,
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
    /// Raw `ValueRef` of every static Bytes literal, keyed by its source text.
    pub(super) bytes: &'m HashMap<String, u64>,
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
    /// Address of the 8-byte aligned static `u32` arrays of `+` chain positions, field orders
    /// and wire schemas.
    pub(super) positions_base: u32,
    /// The source positions of every `+` chain, the slot indices of every typed Object field
    /// order and the records of every wire schema, back to back.
    pub(super) positions: RefCell<Vec<u32>>,
    /// Address of every deduplicated, 8-byte aligned array stored in `positions`, such as
    /// field orders and wire schema records.
    pub(super) static_arrays: RefCell<HashMap<Vec<u32>, u32>>,
    /// The module's implementation methods and their dispatch.
    pub(super) dispatch: &'m Dispatch,
    /// Names of the bindings some closure captures, which live in Cells like v1.
    pub(super) captured: &'m HashSet<String>,
    /// Every closure expression by its source span.
    pub(super) closures: &'m HashMap<(String, u32, u32), ClosureInfo>,
    /// Index of the generated closure dispatcher, when the program creates closures.
    pub(super) closure_dispatcher: Option<u32>,
}

/// Lowering state of one function body.
pub(super) struct FunctionCompiler<'a, 'm> {
    /// Module-wide inputs.
    pub(super) context: &'m Context<'m>,
    /// The function key, the name of a top-level function, `Type::method` or `$closure:N`.
    pub(super) key: &'m str,
    /// Number of leading parameters holding captured Cells.
    captures: u32,
    /// Spans of the `for` loops over Range variables no source code observes, like v1.
    pub(super) unobserved_ranges: Vec<SourceSpan<'a>>,
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
    pub(super) fn new(
        context: &'m Context<'m>,
        key: &'m str,
        declaration: &'m FunctionDeclaration<'a>,
        captures: &'m [String],
    ) -> Self {
        let names = captures.iter().chain(
            declaration
                .parameters
                .iter()
                .map(|parameter| &parameter.name.name),
        );
        let mut scope = HashMap::new();
        let mut slots = HashMap::new();
        for (index, name) in (0_u32..).zip(names) {
            scope.insert(name.clone(), index);
            slots.insert(index, index);
        }
        let parameter_count = slots.len() as u32;
        Self {
            context,
            key,
            declaration,
            captures: captures.len() as u32,
            unobserved_ranges: crate::codegen::range_variables::unobserved_range_loops(declaration),
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

    /// Returns the address of a static copy of `words`, 8-byte aligned and shared by every
    /// array with the same words.
    pub(super) fn static_array(&self, words: Vec<u32>) -> u32 {
        let mut arrays = self.context.static_arrays.borrow_mut();
        if let Some(address) = arrays.get(&words) {
            return *address;
        }
        let mut positions = self.context.positions.borrow_mut();
        if positions.len() % 2 == 1 {
            positions.push(0);
        }
        let address = self.context.positions_base + positions.len() as u32 * 4;
        positions.extend(&words);
        arrays.insert(words, address);
        address
    }

    /// Pushes the VM state block address as an `i32`.
    pub(super) fn push_vm(&mut self) {
        self.instruction(Instruction::I32Const(self.context.vm.cast_signed()));
    }

    /// Pushes 1 when the `ValueRef` in `local` satisfies `contract`, else 0, like v1.
    ///
    /// Nominal Object and enum types are tested through the type id in the reference's extra
    /// bits. Like v1, `List<T>` element types are not checked.
    pub(super) fn contract_test(
        &mut self,
        local: u32,
        contract: &TypeContract,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.instruction(Instruction::LocalGet(local));
        immediate::type_mask_test(&mut self.function, contract.builtin_mask);
        for type_id in &contract.nominal_type_ids {
            self.instruction(Instruction::LocalGet(local));
            self.type_tag_test(*type_id, span)?;
            self.instruction(Instruction::I32Or);
        }
        for identity in &contract.enum_type_ids {
            let type_id = self.enum_type_id(identity, span)?;
            self.instruction(Instruction::LocalGet(local));
            self.type_tag_test(type_id, span)?;
            self.instruction(Instruction::I32Or);
        }
        Ok(())
    }

    /// Returns from the function with the declared-type TypeError unless the `ValueRef` in
    /// `local` satisfies `contract`, like v1.
    pub(super) fn check_contract(
        &mut self,
        local: u32,
        contract: &TypeContract,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if contract.builtin_mask == exs_abi::TYPE_ANY {
            return Ok(());
        }
        self.contract_test(local, contract, span)?;
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
            .get(self.key)
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
        let (parameters, variadic) = self
            .context
            .functions
            .get(self.key)
            .map(|info| (info.parameters.clone(), info.variadic.clone()))
            .ok_or_else(|| {
                error(
                    "E0999",
                    self.declaration.span,
                    "missing v2 function signature".to_owned(),
                )
            })?;
        let first = self.captures;
        for ((local, contract), parameter) in
            (first..).zip(&parameters).zip(&self.declaration.parameters)
        {
            self.check_contract(local, contract, parameter.name.span)?;
        }
        if let (Some(contract), Some(parameter)) = (&variadic, self.declaration.parameters.last()) {
            let local = first + parameters.len() as u32;
            self.check_variadic(local, contract, parameter.name.span)?;
        }
        self.wrap_captured_parameters(first)?;
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
            .get(self.key)
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
