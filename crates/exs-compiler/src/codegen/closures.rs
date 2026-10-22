//! Closures, the Cells of captured bindings and variadic arguments.
//!
//! Every binding whose name some closure of the module captures lives in a Cell, so
//! closures and their creators share it. A closure is lifted into a function whose leading
//! parameters are the captured Cells. Calls of a binding go through one generated dispatcher
//! that selects the closure's function by its dense index, checks the argument count and packs
//! variadic arguments. Closures that may suspend run as resumable frames instead.

use core::mem::{offset_of, size_of};

use exs_layout::TYPE_FN;
use exs_layout::{CellObject, ClosureObject};
use wasm_encoder::{BlockType, Function, Instruction};

use super::collections::{u32_field, value_field};
use super::entry::EntryLayout;
use super::function::{FunctionCompiler, error};
use crate::ast::Expression;
use crate::codegen::{VALUE_TYPE, immediate};
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Byte offset of the value of a Cell.
const CELL_VALUE: u64 = offset_of!(CellObject, value) as u64;
/// Byte offset of the closure index in a closure's header word.
pub(super) const CLOSURE_INDEX: u64 = 4;
/// Byte offset of the first capture slot of a closure.
pub(super) const CAPTURES: u64 = size_of::<ClosureObject>() as u64;

/// Returns the key identifying a closure expression by its source span.
pub(super) fn closure_span_key(span: SourceSpan<'_>) -> (String, u32, u32) {
    (span.source_id.to_owned(), span.start_byte, span.end_byte)
}

/// Returns whether a callee with `arity` fixed parameters, `variadic` or not, takes `count`
/// arguments.
pub(super) const fn accepts(arity: u32, variadic: bool, count: usize) -> bool {
    if variadic {
        count >= arity as usize
    } else {
        count == arity as usize
    }
}

/// Returns the description of the arguments a callee takes, such as `2` or `at least 1`.
pub(super) fn arity_description(arity: u32, variadic: bool) -> String {
    if variadic {
        format!("at least {arity}")
    } else {
        arity.to_string()
    }
}

/// A closure expression of the module.
pub(super) struct ClosureInfo {
    /// Dense index of the closure among the reached closures, its dispatcher case.
    pub(super) index: u32,
    /// Names of the captured bindings in the capture order.
    pub(super) captures: Vec<String>,
}

/// One reached closure as the dispatcher calls it.
pub(super) struct DispatchCase {
    /// Wasm index of the closure's lifted function.
    pub(super) function: u32,
    /// Number of captured Cells.
    pub(super) captures: u32,
    /// Number of fixed parameters.
    pub(super) arity: u32,
    /// Whether a trailing parameter collects the remaining arguments.
    pub(super) variadic: bool,
    /// Whether the closure may suspend, so it runs as frames and never through the
    /// dispatcher.
    pub(super) suspends: bool,
}

impl<'a> FunctionCompiler<'a, '_> {
    /// Returns whether the binding `name` lives in a Cell, because a closure captures it.
    pub(super) fn is_cell(&self, name: &str) -> bool {
        self.context.captured.contains(name)
    }

    /// Binds `name` in the innermost scope to the value in `local`, moving the value into a
    /// new Cell first when the binding is captured.
    pub(super) fn define(&mut self, name: &str, local: u32) -> Result<(), CompileDiagnostics<'a>> {
        if self.is_cell(name) {
            self.wrap_in_cell(local)?;
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_owned(), local);
        }
        Ok(())
    }

    /// Replaces the value in `local` with a new Cell holding it.
    pub(super) fn wrap_in_cell(&mut self, local: u32) -> Result<(), CompileDiagnostics<'a>> {
        self.push_vm();
        self.instruction(Instruction::LocalGet(local));
        let span = self.declaration.span;
        self.core_call("__exs_core_cell_new", span)?;
        self.store(local);
        Ok(())
    }

    /// Pushes the current value of the binding `name` held in `local`.
    pub(super) fn push_binding(&mut self, name: &str, local: u32) {
        self.instruction(Instruction::LocalGet(local));
        if self.is_cell(name) {
            self.instruction(Instruction::I32WrapI64);
            self.instruction(Instruction::I64Load(value_field(CELL_VALUE)));
        }
    }

    /// Pops the stack's value into the binding `name` held in `local`.
    pub(super) fn assign_binding(&mut self, name: &str, local: u32) {
        if self.is_cell(name) {
            let value = self.scratch;
            self.instruction(Instruction::LocalSet(value));
            self.push_address(local);
            self.instruction(Instruction::LocalGet(value));
            self.instruction(Instruction::I64Store(value_field(CELL_VALUE)));
        } else {
            self.store(local);
        }
    }

    /// Creates the closure lifted from the closure expression at `span`, capturing the Cells
    /// of its captured bindings.
    pub(super) fn compile_closure(
        &mut self,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let key = closure_span_key(span);
        let context = self.context;
        let (index, captures) = context
            .closures
            .get(&key)
            .map(|closure| (closure.index, closure.captures.as_slice()))
            .ok_or_else(|| error("E0999", span, "missing closure".to_owned()))?;
        let count = captures.len() as u32;
        let block = self.consecutive_locals(count.max(1));
        for (offset, name) in (0..count).zip(captures) {
            let local = self
                .lookup(name)
                .ok_or_else(|| error("E0999", span, format!("missing capture `{name}`")))?;
            self.instruction(Instruction::LocalGet(local));
            self.store(block + offset);
        }
        self.push_vm();
        self.instruction(Instruction::I32Const(index.cast_signed()));
        self.push_slot_address(block);
        self.instruction(Instruction::I32Const(count.cast_signed()));
        self.core_call("__exs_core_closure_new", span)
    }

    /// Calls the closure in the binding `name` with `arguments`: a value that is no
    /// closure returns the TypeError, a wrong argument count the ArityError.
    pub(super) fn compile_closure_call(
        &mut self,
        name: &str,
        local: u32,
        arguments: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let count = u32::try_from(arguments.len())
            .map_err(|_| error("E0212", span, "too many closure arguments".to_owned()))?;
        let block = self.consecutive_locals(count.max(1));
        for (offset, argument) in (0..count).zip(arguments) {
            self.compile_expression(argument)?;
            self.store(block + offset);
        }
        // The binding is read after the arguments are evaluated.
        self.push_binding(name, local);
        let callee = self.store_new();
        self.set_position(span)?;
        let Some(dispatcher) = self.context.closure_dispatcher else {
            self.push_vm();
            self.instruction(Instruction::LocalGet(callee));
            return self.core_call("__exs_core_not_callable", span);
        };
        self.instruction(Instruction::LocalGet(callee));
        immediate::type_mask_test(&mut self.function, TYPE_FN);
        self.instruction(Instruction::If(BlockType::Result(VALUE_TYPE)));
        self.instruction(Instruction::LocalGet(callee));
        self.push_slot_address(block);
        self.instruction(Instruction::I32Const(count.cast_signed()));
        self.direct_call(dispatcher, span)?;
        self.instruction(Instruction::Else);
        self.push_vm();
        self.instruction(Instruction::LocalGet(callee));
        self.core_call("__exs_core_not_callable", span)?;
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Pushes the fixed arguments among the `count` consecutive argument locals at `block` for
    /// a callee of `arity` fixed parameters, followed by a new List of the rest when it is
    /// variadic.
    pub(super) fn push_arguments(
        &mut self,
        block: u32,
        count: u32,
        arity: u32,
        variadic: bool,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        for offset in 0..arity.min(count) {
            self.instruction(Instruction::LocalGet(block + offset));
        }
        if variadic {
            self.push_vm();
            self.push_slot_address(block);
            self.instruction(Instruction::I32Const((arity * 8).cast_signed()));
            self.instruction(Instruction::I32Add);
            self.instruction(Instruction::I32Const(
                count.saturating_sub(arity).cast_signed(),
            ));
            self.core_call("__exs_core_list_new", span)?;
        }
        Ok(())
    }

    /// Checks every element of the variadic List in `local` against `contract`, like the
    /// parameter validation.
    pub(super) fn check_variadic(
        &mut self,
        local: u32,
        contract: &crate::codegen::types::TypeContract,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if contract.builtin_mask == exs_layout::TYPE_ANY {
            return Ok(());
        }
        let index = self.scalar_local();
        let element = self.value_local();
        self.instruction(Instruction::I64Const(0));
        self.instruction(Instruction::LocalSet(index));
        self.instruction(Instruction::Block(BlockType::Empty));
        let done = self.depth;
        self.instruction(Instruction::Loop(BlockType::Empty));
        let head = self.depth;
        self.instruction(Instruction::LocalGet(index));
        self.instruction(Instruction::I32WrapI64);
        self.push_address(local);
        self.instruction(Instruction::I32Load(u32_field(LIST_LEN)));
        self.instruction(Instruction::I32GeU);
        self.instruction(Instruction::BrIf(self.depth - done));
        self.push_address(local);
        self.instruction(Instruction::I32Load(u32_field(LIST_ITEMS)));
        self.instruction(Instruction::LocalGet(index));
        self.instruction(Instruction::I32WrapI64);
        self.instruction(Instruction::I32Const(3));
        self.instruction(Instruction::I32Shl);
        self.instruction(Instruction::I32Add);
        self.instruction(Instruction::I64Load(value_field(BUFFER_SLOTS)));
        self.store(element);
        self.check_contract(element, contract, span)?;
        self.instruction(Instruction::LocalGet(index));
        self.instruction(Instruction::I64Const(1));
        self.instruction(Instruction::I64Add);
        self.instruction(Instruction::LocalSet(index));
        self.branch_to(head);
        self.instruction(Instruction::End);
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Wraps the declared parameters, the locals from `first` on, whose names closures
    /// capture in Cells.
    pub(super) fn wrap_captured_parameters(
        &mut self,
        first: u32,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let names: Vec<(u32, String)> = (first..)
            .zip(&self.declaration.parameters)
            .map(|(local, parameter)| (local, parameter.name.name.clone()))
            .collect();
        for (local, name) in names {
            if self.is_cell(&name) {
                self.wrap_in_cell(local)?;
            }
        }
        Ok(())
    }
}

/// Byte offset of the element count of a List.
const LIST_LEN: u64 = offset_of!(exs_layout::ListObject, len) as u64;
/// Byte offset of the element Buffer of a List.
const LIST_ITEMS: u64 = offset_of!(exs_layout::ListObject, items) as u64;
/// Byte offset of the first slot of a Buffer.
const BUFFER_SLOTS: u64 = size_of::<exs_layout::BufferObject>() as u64;

/// Builds the closure dispatcher `(closure, slots, count) -> value`: it selects the case of
/// the closure's index, returns the ArityError for a wrong argument count, and otherwise
/// calls the lifted function with the captured Cells, the fixed arguments from the rooted
/// `slots` and, for a variadic closure, a new List of the rest.
pub(super) fn closure_dispatcher<'a>(
    layout: &EntryLayout<'_>,
    cases: &[DispatchCase],
    span: SourceSpan<'a>,
) -> Result<Function, CompileDiagnostics<'a>> {
    let (closure, slots, count) = (0, 1, 2);
    let mut function = Function::new([]);
    let total = cases.len() as u32;
    function.instruction(&Instruction::Block(BlockType::Empty));
    for _ in cases {
        function.instruction(&Instruction::Block(BlockType::Empty));
    }
    function.instruction(&Instruction::LocalGet(closure));
    function.instruction(&Instruction::I32WrapI64);
    function.instruction(&Instruction::I32Load(u32_field(CLOSURE_INDEX)));
    let targets: Vec<u32> = (0..total).collect();
    function.instruction(&Instruction::BrTable(targets.into(), total));
    for case in cases {
        function.instruction(&Instruction::End);
        if case.suspends {
            function.instruction(&Instruction::Unreachable);
            continue;
        }
        function.instruction(&Instruction::LocalGet(count));
        function.instruction(&Instruction::I32Const(case.arity.cast_signed()));
        function.instruction(&if case.variadic {
            Instruction::I32LtU
        } else {
            Instruction::I32Ne
        });
        function.instruction(&Instruction::If(BlockType::Empty));
        function.instruction(&Instruction::I32Const(layout.vm.cast_signed()));
        function.instruction(&Instruction::Call(
            layout.core("__exs_core_closure_arity_error", span)?,
        ));
        function.instruction(&Instruction::Return);
        function.instruction(&Instruction::End);
        for capture in 0..u64::from(case.captures) {
            function.instruction(&Instruction::LocalGet(closure));
            function.instruction(&Instruction::I32WrapI64);
            function.instruction(&Instruction::I64Load(value_field(CAPTURES + capture * 8)));
        }
        for argument in 0..u64::from(case.arity) {
            function.instruction(&Instruction::LocalGet(slots));
            function.instruction(&Instruction::I64Load(value_field(argument * 8)));
        }
        if case.variadic {
            function.instruction(&Instruction::I32Const(layout.vm.cast_signed()));
            function.instruction(&Instruction::LocalGet(slots));
            function.instruction(&Instruction::I32Const((case.arity * 8).cast_signed()));
            function.instruction(&Instruction::I32Add);
            function.instruction(&Instruction::LocalGet(count));
            function.instruction(&Instruction::I32Const(case.arity.cast_signed()));
            function.instruction(&Instruction::I32Sub);
            function.instruction(&Instruction::Call(
                layout.core("__exs_core_list_new", span)?,
            ));
        }
        function.instruction(&Instruction::Call(case.function));
        function.instruction(&Instruction::Return);
    }
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);
    Ok(function)
}
