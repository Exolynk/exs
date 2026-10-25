//! Language Error lowering: origins, traces and early returns.
//!
//! Errors are values; a function only returns one early for `?`, for a
//! condition that is no Bool, for a declared-type violation and after a direct call that
//! produced a fatal Error. Before every core call that may create an Error, generated code
//! stores the source position reported as its origin, and every function keeps its entry on
//! the trace stack while it runs.

use core::mem::offset_of;

use exs_layout::{ERROR_FATAL, ValueRef, VmState};
use exs_layout::{TYPE_ERROR, TYPE_NONE};
use wasm_encoder::{BlockType, Function, Instruction, MemArg};

use super::collections::u32_field;
use super::function::{FunctionCompiler, error, vm_field};
use super::wire_schema::needs_decode;
use crate::ast::{Expression, Identifier};
use crate::codegen::immediate;
use crate::codegen::types::{TypeContract, permits_error};
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Byte offset of the severity in an Error's header word.
const SEVERITY: u64 = 4;

/// Returns the offset of one `u32` field of the VM's Error state.
macro_rules! errors_offset {
    ($field:ident) => {
        offset_of!(VmState, errors.$field) as u32
    };
}

/// Emits the trace push of a function's entry: `function_id` with the pending call site,
/// leaving the entry's address in `trace_local`. Traps when the trace stack is full.
pub(super) fn emit_push_trace(
    function: &mut Function,
    vm: u32,
    trace_local: u32,
    function_id: u32,
) {
    let top = vm_field(vm, errors_offset!(trace_top));
    let pending = vm_field(vm, errors_offset!(pending_call_site));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::I32Load(top));
    function.instruction(&Instruction::LocalTee(trace_local));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::I32Load(vm_field(
        vm,
        errors_offset!(trace_limit),
    )));
    function.instruction(&Instruction::I32GeU);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::LocalGet(trace_local));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::I32Load(pending));
    function.instruction(&Instruction::I64ExtendI32U);
    function.instruction(&Instruction::I64Const(32));
    function.instruction(&Instruction::I64Shl);
    function.instruction(&Instruction::I64Const(i64::from(function_id)));
    function.instruction(&Instruction::I64Or);
    function.instruction(&Instruction::I64Store(MemArg {
        offset: 0,
        align: 3,
        memory_index: 0,
    }));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::I32Store(pending));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::LocalGet(trace_local));
    function.instruction(&Instruction::I32Const(8));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::I32Store(top));
}

/// The message of E0218, the diagnostic for `?` in a function it cannot return from.
pub(super) const PROPAGATION_REQUIREMENT: &str =
    "? requires the current function return type to include Error, None or Any";

/// Returns whether `?` may return from a function whose return type has the built-in `mask`:
/// it must admit Errors or None.
pub(super) const fn permits_propagation(mask: u32) -> bool {
    mask & (TYPE_ERROR | TYPE_NONE) != 0
}

impl<'a> FunctionCompiler<'a, '_> {
    /// Returns the source position id of `span`.
    pub(super) fn position(&self, span: SourceSpan<'a>) -> Result<u32, CompileDiagnostics<'a>> {
        self.context.source_map.id(span).ok_or_else(|| {
            error(
                "E0214",
                span,
                "missing source-map position for generated code".to_owned(),
            )
        })
    }

    /// Stores the position of `span` as the origin of Errors created next.
    pub(super) fn set_position(
        &mut self,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let position = self.position(span)?;
        self.store_vm_u32(errors_offset!(source_position), position);
        Ok(())
    }

    /// Stores the position of `span` as the call site the next function entry records.
    pub(super) fn set_call_site(
        &mut self,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let position = self.position(span)?;
        self.store_vm_u32(errors_offset!(pending_call_site), position);
        Ok(())
    }

    /// Stores the constant `value` in the `u32` VM field at `offset`.
    fn store_vm_u32(&mut self, offset: u32, value: u32) {
        self.instruction(Instruction::I32Const(0));
        self.instruction(Instruction::I32Const(value.cast_signed()));
        self.instruction(Instruction::I32Store(vm_field(self.context.vm, offset)));
    }

    /// Returns this function's declared return contract.
    pub(super) fn return_contract(&self) -> Option<&TypeContract> {
        self.context
            .functions
            .get(self.key)
            .map(|info| &info.return_contract)
    }

    /// Returns whether this function's return type admits Errors.
    pub(super) fn permits_error(&self) -> bool {
        self.return_contract().is_some_and(permits_error)
    }

    /// Returns the `ValueRef` on the stack from the function without a contract check.
    ///
    /// A step function completes its frame with the value instead and returns the dispatch
    /// status.
    pub(super) fn return_top(&mut self) {
        self.instruction(Instruction::LocalSet(self.scratch));
        self.pop_frame();
        if self.step {
            self.push_vm();
            self.instruction(Instruction::LocalGet(0));
            self.instruction(Instruction::LocalGet(self.scratch));
            if let Some(index) = self.context.core.get("__exs_core_frame_return").copied() {
                self.instruction(Instruction::Call(index));
            } else {
                self.instruction(Instruction::Unreachable);
            }
        } else {
            self.instruction(Instruction::LocalGet(self.scratch));
        }
        self.instruction(Instruction::Return);
    }

    /// Returns the TypeError for the value in `local` violating a declared type: recoverable
    /// when the return type admits Errors, fatal otherwise.
    pub(super) fn return_type_mismatch(
        &mut self,
        local: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.set_position(span)?;
        self.push_vm();
        self.instruction(Instruction::LocalGet(local));
        self.instruction(Instruction::I32Const(i32::from(self.permits_error())));
        self.core_call("__exs_core_type_mismatch", span)?;
        self.return_top();
        Ok(())
    }

    /// Returns the Error in `local`, or the TypeError for it when the return type admits no
    /// Errors.
    fn return_error(
        &mut self,
        local: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if self.permits_error() {
            self.instruction(Instruction::LocalGet(local));
            self.return_top();
            Ok(())
        } else {
            self.return_type_mismatch(local, span)
        }
    }

    /// Pushes 1 when the `ValueRef` in `local` is an Error, else 0.
    fn is_error(&mut self, local: u32) {
        self.instruction(Instruction::LocalGet(local));
        immediate::type_mask_test(&mut self.function, TYPE_ERROR);
    }

    /// Returns the value in `local` from the function when it is an Error.
    pub(super) fn return_if_error(
        &mut self,
        local: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.is_error(local);
        self.instruction(Instruction::If(BlockType::Empty));
        self.return_error(local, span)?;
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Returns the value in `local` unchanged when it is a fatal Error, as after direct calls.
    pub(super) fn return_if_fatal(&mut self, local: u32) {
        self.is_error(local);
        self.instruction(Instruction::If(BlockType::Empty));
        self.push_address(local);
        self.instruction(Instruction::I32Load(u32_field(SEVERITY)));
        self.instruction(Instruction::I32Const(ERROR_FATAL.cast_signed()));
        self.instruction(Instruction::I32Eq);
        self.instruction(Instruction::If(BlockType::Empty));
        self.instruction(Instruction::LocalGet(local));
        self.return_top();
        self.instruction(Instruction::End);
        self.instruction(Instruction::End);
    }

    /// Pops a `ValueRef` and pushes it as a Wasm `i32` condition; a value that is no Bool
    /// returns its TypeError from the function, with the origin `span`.
    pub(super) fn condition(&mut self, span: SourceSpan<'a>) -> Result<(), CompileDiagnostics<'a>> {
        let value = self.scratch;
        self.instruction(Instruction::LocalTee(value));
        immediate::is_bool_test(&mut self.function);
        self.instruction(Instruction::If(BlockType::Result(
            wasm_encoder::ValType::I32,
        )));
        self.instruction(Instruction::LocalGet(value));
        self.instruction(Instruction::I32WrapI64);
        self.instruction(Instruction::Else);
        self.set_position(span)?;
        self.push_vm();
        self.instruction(Instruction::LocalGet(value));
        self.core_call("__exs_core_condition_error", span)?;
        self.instruction(Instruction::LocalSet(value));
        self.return_error(value, span)?;
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Compiles `value?` by the function's return type: None returns None when the return
    /// type admits None and a MissingValue Error otherwise; an Error returns from the function,
    /// as a fatal TypeError when the return type admits no Errors; every other value passes
    /// through.
    pub(super) fn compile_propagate(
        &mut self,
        value: &Expression<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let mask = self
            .return_contract()
            .map_or(0, |contract| contract.builtin_mask);
        if !permits_propagation(mask) {
            return Err(error("E0218", span, PROPAGATION_REQUIREMENT.to_owned()));
        }
        self.compile_expression(value)?;
        let operand = self.store_new();
        self.instruction(Instruction::LocalGet(operand));
        immediate::type_mask_test(&mut self.function, TYPE_NONE);
        self.instruction(Instruction::If(BlockType::Empty));
        if mask & TYPE_NONE != 0 {
            immediate::push(&mut self.function, ValueRef::NONE);
        } else {
            self.set_position(span)?;
            self.push_vm();
            self.core_call("__exs_core_missing_value", span)?;
        }
        self.return_top();
        self.instruction(Instruction::End);
        self.return_if_error(operand, span)?;
        self.instruction(Instruction::LocalGet(operand));
        Ok(())
    }

    /// Compiles the `Error(kind, message, data)` constructor.
    pub(super) fn compile_error_constructor(
        &mut self,
        arguments: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if arguments.len() != 3 {
            return Err(error(
                "E0208",
                span,
                format!(
                    "constructor `Error` expects 3 arguments but received {}",
                    arguments.len()
                ),
            ));
        }
        let mut locals = Vec::with_capacity(3);
        for argument in arguments {
            self.compile_expression(argument)?;
            locals.push(self.store_new());
        }
        self.set_position(span)?;
        self.push_vm();
        for local in locals {
            self.instruction(Instruction::LocalGet(local));
        }
        self.core_call("__exs_core_error_new", span)
    }

    /// Decodes an annotated binding's value in `local` against `contract` like the wire
    /// decoding: an Error returns from the function, a value of another type returns a
    /// WireDecodeError, and Lists, Objects and enums may be converted.
    ///
    /// Contracts of built-in types only test the value's type flags inline.
    pub(super) fn check_binding(
        &mut self,
        local: u32,
        contract: &TypeContract,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if needs_decode(contract) {
            let schema = self.wire_schema(contract, span)?;
            self.set_position(span)?;
            self.push_vm();
            self.instruction(Instruction::LocalGet(local));
            self.instruction(Instruction::I32Const(schema.cast_signed()));
            self.core_call("__exs_core_wire_decode", span)?;
            self.store(local);
            return self.return_if_error(local, span);
        }
        self.return_if_error(local, span)?;
        self.instruction(Instruction::LocalGet(local));
        immediate::type_mask_test(&mut self.function, contract.builtin_mask);
        self.instruction(Instruction::I32Eqz);
        self.instruction(Instruction::If(BlockType::Empty));
        self.set_position(span)?;
        self.push_vm();
        self.instruction(Instruction::LocalGet(local));
        self.core_call("__exs_core_wire_mismatch", span)?;
        self.instruction(Instruction::LocalSet(self.scratch));
        self.return_error(self.scratch, span)?;
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Decodes the item in `local` of a `for` loop with an annotated `binding`, at the
    /// binding's position.
    pub(super) fn check_for_binding(
        &mut self,
        local: u32,
        binding: ForBinding<'a, '_>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        match binding.contract {
            Some(contract) => self.check_binding(local, contract, binding.identifier.span),
            None => Ok(()),
        }
    }
}

/// The binding of a `for` loop and its resolved annotation, if any.
#[derive(Clone, Copy)]
pub(super) struct ForBinding<'a, 'c> {
    /// The bound name.
    pub(super) identifier: &'c Identifier<'a>,
    /// The resolved contract of its annotation.
    pub(super) contract: Option<&'c TypeContract>,
}
