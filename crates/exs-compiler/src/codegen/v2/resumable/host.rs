//! Host calls of resumable code: a call either finishes at once or suspends the task until
//! the runner resumes it, and the results of stream and time calls become prelude values.

use exs_abi::{HOST_CALL_PENDING, HOST_CALL_READY, STATUS_PENDING, TYPE_ERROR, TYPE_OBJECT};
use exs_value::ValueRef;
use wasm_encoder::{BlockType, Instruction};

use super::step::StepEmitter;
use crate::ast::Expression;
use crate::codegen::continuation::{HostTimeField, Operation};
use crate::codegen::immediate;
use crate::codegen::types::{NominalType, TypeContract};
use crate::codegen::v2::collections::value_field;
use crate::codegen::v2::function::{error, unsupported};
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Byte offset of the first field slot of a typed Object.
const TYPED_SLOTS: u64 = core::mem::size_of::<exs_layout::TypedObject>() as u64;

impl<'a, 'm> StepEmitter<'a, 'm> {
    /// Emits the host and parallel operations, ending in a state transition, a suspension or
    /// a return.
    pub(super) fn host_operation(
        &mut self,
        state: u32,
        operation: &Operation<'a, '_>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let next = state + 1;
        match operation {
            Operation::HostCall {
                name,
                arguments,
                argument_list,
                span,
                ..
            } => self.host_call(next, *name, arguments, *argument_list, *span),
            Operation::HostResume { destination, span } => {
                self.compiler.push_vm();
                self.compiler
                    .core_call("__exs_core_host_call_take", *span)?;
                self.set_slot(*destination);
                self.return_if_fatal(*destination);
                self.ready(next, *span)
            }
            Operation::HostStream {
                handle,
                type_id,
                handle_contract,
                destination,
                span,
            } => {
                let fields = [(*handle, "handle", handle_contract)];
                self.host_object(next, *handle, *type_id, &fields, *destination, *span)
            }
            Operation::HostTime {
                value,
                type_id,
                fields,
                destination,
                span,
            } => self.host_time(next, *value, *type_id, fields, *destination, *span),
            Operation::ParallelStart { span, .. }
            | Operation::ParallelDynamicStart { span, .. }
            | Operation::ParallelTake { span, .. } => Err(unsupported(*span, "parallel tasks")),
            Operation::Assert { span, .. } => Err(unsupported(*span, "resumable assertions")),
            _ => Err(error(
                "E0999",
                self.compiler.declaration.span,
                "unhandled continuation operation".to_owned(),
            )),
        }
    }

    /// Starts the host call of the function named in durable slot `name` with the List of
    /// the durable `arguments`: a ready result continues at once with the `HostResume` state
    /// `next`, a pending one suspends the task until the runner resumes it there.
    fn host_call(
        &mut self,
        next: u32,
        name: u32,
        arguments: &[u32],
        argument_list: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.synthesize(arguments, span, |elements| Expression::List {
            elements,
            span,
        })?;
        self.set_slot(argument_list);
        self.compiler.set_position(span)?;
        self.compiler.push_vm();
        self.get_slot(name);
        self.get_slot(argument_list);
        self.compiler
            .core_call("__exs_core_host_call_start", span)?;
        let status = self.compiler.scalar_local();
        self.instruction(Instruction::I64ExtendI32U);
        self.instruction(Instruction::LocalTee(status));
        self.instruction(Instruction::I64Const(i64::from(HOST_CALL_READY)));
        self.instruction(Instruction::I64Eq);
        self.instruction(Instruction::If(BlockType::Empty));
        self.ready(next, span)?;
        self.instruction(Instruction::End);
        self.instruction(Instruction::LocalGet(status));
        self.instruction(Instruction::I64Const(i64::from(HOST_CALL_PENDING)));
        self.instruction(Instruction::I64Eq);
        self.instruction(Instruction::If(BlockType::Empty));
        self.store_state(next);
        self.leave(STATUS_PENDING);
        self.instruction(Instruction::End);
        // Like v1, a fatal runner failure traps.
        self.instruction(Instruction::Unreachable);
        Ok(())
    }

    /// Converts the runner's time result in durable slot `value` into the prelude value of
    /// `type_id`, like v1.
    fn host_time(
        &mut self,
        next: u32,
        value: u32,
        type_id: u32,
        fields: &[HostTimeField],
        destination: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let raw = self.load(value);
        self.instruction(Instruction::LocalGet(raw));
        immediate::type_mask_test(&mut self.compiler.function, TYPE_ERROR | TYPE_OBJECT);
        self.instruction(Instruction::I32Eqz);
        self.instruction(Instruction::If(BlockType::Empty));
        self.mismatch(raw, destination, next, span)?;
        self.instruction(Instruction::End);
        for field in fields {
            self.instruction(Instruction::LocalGet(raw));
            immediate::type_mask_test(&mut self.compiler.function, TYPE_ERROR);
            self.instruction(Instruction::I32Eqz);
            self.instruction(Instruction::If(BlockType::Empty));
            let name = field.name.as_str();
            self.synthesize(&[value], span, |mut operands| Expression::Index {
                receiver: Box::new(operands.remove(0)),
                index: Box::new(Expression::String(name.to_owned(), span)),
                span,
            })?;
            self.set_slot(field.slot);
            self.instruction(Instruction::End);
        }
        let slots: Vec<(u32, &str, &TypeContract)> = fields
            .iter()
            .map(|field| (field.slot, field.name.as_str(), &field.contract))
            .collect();
        self.host_object(next, value, type_id, &slots, destination, span)
    }

    /// Stores the Error in durable slot `value` unchanged, else the prelude Object of
    /// `type_id` holding the durable `fields` after checking their contracts like v1: the
    /// first mismatch stores its TypeError instead.
    fn host_object(
        &mut self,
        next: u32,
        value: u32,
        type_id: u32,
        fields: &[(u32, &str, &TypeContract)],
        destination: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.is_error(value);
        self.instruction(Instruction::If(BlockType::Empty));
        self.get_slot(value);
        self.set_slot(destination);
        self.ready(next, span)?;
        self.instruction(Instruction::End);
        let mut locals = Vec::with_capacity(fields.len());
        for (slot, _, contract) in fields {
            let local = self.load(*slot);
            self.compiler.contract_test(local, contract, span)?;
            self.instruction(Instruction::I32Eqz);
            self.instruction(Instruction::If(BlockType::Empty));
            self.mismatch(local, destination, next, span)?;
            self.instruction(Instruction::End);
            locals.push(local);
        }
        let nominal = self.nominal(type_id, span)?;
        let names: Vec<&str> = fields.iter().map(|(_, name, _)| *name).collect();
        let object = self.compiler.allocate_typed_object(nominal, &names, span)?;
        for (index, field) in (0_u64..).zip(&nominal.fields) {
            self.compiler.push_address(object);
            match names.iter().position(|name| *name == field.name) {
                Some(position) => self.instruction(Instruction::LocalGet(locals[position])),
                None => immediate::push(&mut self.compiler.function, ValueRef::NONE),
            }
            self.instruction(Instruction::I64Store(value_field(TYPED_SLOTS + index * 8)));
        }
        self.instruction(Instruction::LocalGet(object));
        self.set_slot(destination);
        self.ready(next, span)
    }

    /// Stores the recoverable TypeError for the value in the temporary `local` in durable
    /// slot `destination` and continues at `next`.
    fn mismatch(
        &mut self,
        local: u32,
        destination: u32,
        next: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compiler.set_position(span)?;
        self.compiler.push_vm();
        self.instruction(Instruction::LocalGet(local));
        self.instruction(Instruction::I32Const(1));
        self.compiler.core_call("__exs_core_type_mismatch", span)?;
        self.set_slot(destination);
        self.ready(next, span)
    }

    /// Returns the nominal type `type_id`.
    pub(super) fn nominal(
        &self,
        type_id: u32,
        span: SourceSpan<'a>,
    ) -> Result<&'m NominalType, CompileDiagnostics<'a>> {
        let context: &'m _ = self.compiler.context;
        context
            .types
            .nominal_types()
            .find(|nominal| nominal.id == type_id)
            .ok_or_else(|| error("E0999", span, format!("missing nominal type {type_id}")))
    }
}
