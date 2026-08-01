//! Call-boundary and utility Wasm emission for continuation frames.

use exs_abi::{STATUS_COMPLETE, STATUS_PENDING, STATUS_READY};
use exs_value::is_valid_int;
use wasm_encoder::{BlockType, Instruction, ValType};

use crate::ast::{BinaryOperator, Expression};
use crate::codegen::diagnostics;
use crate::codegen::function::InstanceMethod;
use crate::codegen::types;
use crate::codegen::types::TypeContract;
use crate::codegen::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

use super::entry::runtime_index;
use super::graph::expression_span;
use super::step::StepCompiler;

impl<'source, 'context> StepCompiler<'source, 'context> {
    /// Emits nominal instance dispatch, including suspendable child targets and runtime fallback.
    #[allow(clippy::too_many_arguments)] // This directly mirrors the already-evaluated source call.
    pub(super) fn instance_call(
        &mut self,
        next: u32,
        receiver: u32,
        method: &str,
        method_span: SourceSpan<'source>,
        arguments: &[u32],
        targets: &[InstanceMethod],
        destination: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.instance_call_target(
            next,
            receiver,
            method,
            method_span,
            arguments,
            targets,
            0,
            destination,
            span,
        )
    }

    /// Emits one branch of the static nominal method-target chain.
    #[allow(clippy::too_many_arguments)] // Recursion keeps the generated Wasm branch chain local.
    pub(super) fn instance_call_target(
        &mut self,
        next: u32,
        receiver: u32,
        method: &str,
        method_span: SourceSpan<'source>,
        arguments: &[u32],
        targets: &[InstanceMethod],
        index: usize,
        destination: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let Some(target) = targets.get(index) else {
            return self.runtime_method_call(
                next,
                receiver,
                method,
                method_span,
                arguments,
                destination,
                span,
            );
        };
        self.get_slot(receiver, span)?;
        self.function
            .instruction(&Instruction::I32Const(target.type_id.cast_signed()));
        self.call_runtime("__exs_rt_object_is_type", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        if target.signature.arity != arguments.len() + 1 {
            self.get_slot(receiver, span)?;
            self.call_runtime("__exs_rt_method_arity_error", span)?;
            self.set_slot(destination, span)?;
            self.ready(next, span)?;
        } else if let Some(layout) = self
            .frame_layouts
            .values()
            .find(|layout| layout.function_id == target.signature.function_id)
            .copied()
        {
            let mut child_arguments = Vec::with_capacity(arguments.len() + 1);
            child_arguments.push(receiver);
            child_arguments.extend_from_slice(arguments);
            self.child_call(next, layout, &child_arguments, destination, span)?;
        } else {
            self.get_slot(receiver, span)?;
            for argument in arguments {
                self.get_slot(*argument, span)?;
            }
            self.set_call_site(span)?;
            self.function
                .instruction(&Instruction::Call(target.signature.index));
            self.set_slot(destination, span)?;
            self.ready(next, span)?;
        }
        self.function.instruction(&Instruction::Else);
        self.instance_call_target(
            next,
            receiver,
            method,
            method_span,
            arguments,
            targets,
            index + 1,
            destination,
            span,
        )?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Calls the runtime built-in method dispatcher after all source operands were evaluated.
    #[allow(clippy::too_many_arguments)] // Runtime fallback receives the same complete call context.
    pub(super) fn runtime_method_call(
        &mut self,
        next: u32,
        receiver: u32,
        method: &str,
        method_span: SourceSpan<'source>,
        arguments: &[u32],
        destination: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.call_runtime("__exs_rt_list_new", span)?;
        // The method-name allocation below can collect values held only in Wasm locals. Keep
        // this temporary argument List in the destination's durable async-frame slot instead.
        self.set_slot(destination, span)?;
        for argument in arguments {
            self.get_slot(destination, span)?;
            self.get_slot(*argument, span)?;
            self.call_runtime("__exs_rt_append", span)?;
            self.function.instruction(&Instruction::Drop);
        }
        self.string(method, method_span)?;
        self.function.instruction(&Instruction::LocalSet(2));
        self.get_slot(receiver, span)?;
        self.function.instruction(&Instruction::LocalGet(2));
        self.get_slot(destination, span)?;
        self.call_runtime("__exs_rt_call_method", span)?;
        self.set_slot(destination, span)?;
        self.ready(next, span)
    }

    /// Stores a value in a durable frame slot.
    pub(super) fn set_slot(
        &mut self,
        slot: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.function
            .instruction(&Instruction::LocalSet(self.scratch_local));
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::I32Const(slot.cast_signed()));
        self.function
            .instruction(&Instruction::LocalGet(self.scratch_local));
        self.function.instruction(&Instruction::Call(
            self.runtime_index("__exs_rt_async_frame_set_slot", span)?,
        ));
        Ok(())
    }

    /// Emits the source call-site position consumed by a child frame-stack push.
    pub(super) fn set_call_site(
        &mut self,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let position = self.source_map.id(span).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0214",
                span,
                "missing source-map position for generated function call",
            ))
        })?;
        self.function
            .instruction(&Instruction::I32Const(position.cast_signed()));
        self.function.instruction(&Instruction::Call(
            self.runtime_index("__exs_rt_set_call_site", span)?,
        ));
        Ok(())
    }

    /// Loads a durable frame slot onto the Wasm stack.
    pub(super) fn get_slot(
        &mut self,
        slot: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::I32Const(slot.cast_signed()));
        self.call_runtime("__exs_rt_async_frame_get_slot", span)
    }

    /// Advances to the next state and returns runnable status.
    pub(super) fn ready(
        &mut self,
        next: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::I32Const(next.cast_signed()));
        self.call_runtime("__exs_rt_async_frame_set_state", span)?;
        self.function
            .instruction(&Instruction::I32Const(STATUS_READY));
        self.function.instruction(&Instruction::Return);
        Ok(())
    }

    /// Stores the host-resume state and returns pending status.
    pub(super) fn ready_pending(
        &mut self,
        resume: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::I32Const(resume.cast_signed()));
        self.call_runtime("__exs_rt_async_frame_set_state", span)?;
        self.call_runtime("__exs_rt_scheduler_status", span)?;
        self.function.instruction(&Instruction::Return);
        Ok(())
    }

    /// Completes this async frame and returns root-complete or caller-runnable status.
    pub(super) fn complete(
        &mut self,
        value: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.get_slot(value, span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.scratch_local));
        self.validate_local_return(span)?;
        self.complete_local(span)
    }

    /// Completes this async frame with the value held in the scratch local.
    pub(super) fn complete_local(
        &mut self,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.function.instruction(&Instruction::LocalGet(0));
        self.call_runtime("__exs_rt_async_frame_pop_trace", span)?;
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::LocalGet(self.scratch_local));
        self.call_runtime("__exs_rt_async_frame_complete", span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.scratch_local));
        self.function
            .instruction(&Instruction::LocalGet(self.scratch_local));
        self.function.instruction(&Instruction::I32Const(1));
        self.function.instruction(&Instruction::I32Eq);
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        self.function
            .instruction(&Instruction::I32Const(STATUS_COMPLETE));
        self.function.instruction(&Instruction::Else);
        self.function
            .instruction(&Instruction::LocalGet(self.scratch_local));
        self.function.instruction(&Instruction::I32Const(2));
        self.function.instruction(&Instruction::I32Eq);
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        self.function
            .instruction(&Instruction::I32Const(STATUS_PENDING));
        self.function.instruction(&Instruction::Else);
        self.function
            .instruction(&Instruction::I32Const(STATUS_READY));
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::Return);
        Ok(())
    }

    /// Completes early when one durable slot contains a recoverable language Error.
    pub(super) fn complete_if_error(
        &mut self,
        value: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.get_slot(value, span)?;
        self.call_runtime("__exs_rt_is_error", span)?;
        self.call_runtime("__exs_rt_condition", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.complete(value, span)?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Branches after validating a source Boolean and returns Error values early.
    pub(super) fn branch_on_value(
        &mut self,
        value: u32,
        checked: u32,
        when_true: u32,
        when_false: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.get_slot(value, span)?;
        self.call_runtime("__exs_rt_condition_value", span)?;
        self.set_slot(checked, span)?;
        self.complete_if_error(checked, span)?;
        self.get_slot(checked, span)?;
        self.call_runtime("__exs_rt_condition", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.ready(when_true, span)?;
        self.function.instruction(&Instruction::Else);
        self.ready(when_false, span)?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Checks one frame slot against a parameter contract or completes with TypeError.
    pub(super) fn validate_slot_or_complete(
        &mut self,
        slot: u32,
        contract: &TypeContract,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.validate_slot_matches(slot, contract, span)?;
        self.function.instruction(&Instruction::LocalGet(2));
        self.function.instruction(&Instruction::I32Eqz);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.get_slot(slot, span)?;
        self.function
            .instruction(&Instruction::I32Const(i32::from(types::permits_error(
                self.return_contract,
            ))));
        self.call_runtime("__exs_rt_type_mismatch", span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.scratch_local));
        self.complete_local(span)?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Replaces the scratch value with a TypeError when it violates the return contract.
    pub(super) fn validate_local_return(
        &mut self,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.validate_scratch_matches(self.return_contract, span)?;
        self.function.instruction(&Instruction::LocalGet(2));
        self.function.instruction(&Instruction::I32Eqz);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.function
            .instruction(&Instruction::LocalGet(self.scratch_local));
        self.function
            .instruction(&Instruction::I32Const(i32::from(types::permits_error(
                self.return_contract,
            ))));
        self.call_runtime("__exs_rt_type_mismatch", span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.scratch_local));
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Writes the contract match result for one frame slot into local two.
    pub(super) fn validate_slot_matches(
        &mut self,
        slot: u32,
        contract: &TypeContract,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.get_slot(slot, span)?;
        self.function
            .instruction(&Instruction::I32Const(contract.builtin_mask.cast_signed()));
        self.call_runtime("__exs_rt_type_matches", span)?;
        self.function.instruction(&Instruction::LocalSet(2));
        for type_id in &contract.nominal_type_ids {
            self.function.instruction(&Instruction::LocalGet(2));
            self.get_slot(slot, span)?;
            self.function
                .instruction(&Instruction::I32Const(type_id.cast_signed()));
            self.call_runtime("__exs_rt_object_is_type", span)?;
            self.function.instruction(&Instruction::I32Or);
            self.function.instruction(&Instruction::LocalSet(2));
        }
        for type_id in &contract.enum_type_ids {
            self.function.instruction(&Instruction::LocalGet(2));
            self.get_slot(slot, span)?;
            self.string(type_id, span)?;
            self.call_runtime("__exs_rt_enum_is_type", span)?;
            self.function.instruction(&Instruction::I32Or);
            self.function.instruction(&Instruction::LocalSet(2));
        }
        Ok(())
    }

    /// Writes the contract match result for the scratch local into local two.
    pub(super) fn validate_scratch_matches(
        &mut self,
        contract: &TypeContract,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.function
            .instruction(&Instruction::LocalGet(self.scratch_local));
        self.function
            .instruction(&Instruction::I32Const(contract.builtin_mask.cast_signed()));
        self.call_runtime("__exs_rt_type_matches", span)?;
        self.function.instruction(&Instruction::LocalSet(2));
        for type_id in &contract.nominal_type_ids {
            self.function.instruction(&Instruction::LocalGet(2));
            self.function
                .instruction(&Instruction::LocalGet(self.scratch_local));
            self.function
                .instruction(&Instruction::I32Const(type_id.cast_signed()));
            self.call_runtime("__exs_rt_object_is_type", span)?;
            self.function.instruction(&Instruction::I32Or);
            self.function.instruction(&Instruction::LocalSet(2));
        }
        for type_id in &contract.enum_type_ids {
            self.function.instruction(&Instruction::LocalGet(2));
            self.function
                .instruction(&Instruction::LocalGet(self.scratch_local));
            self.string(type_id, span)?;
            self.call_runtime("__exs_rt_enum_is_type", span)?;
            self.function.instruction(&Instruction::I32Or);
            self.function.instruction(&Instruction::LocalSet(2));
        }
        Ok(())
    }

    /// Emits a scalar literal construction.
    pub(super) fn literal(
        &mut self,
        expression: &Expression<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        match expression {
            Expression::Integer(value, span) => {
                if !is_valid_int(*value) {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0206",
                        *span,
                        "integer literal is outside the ExS 56-bit range",
                    )));
                }
                self.function.instruction(&Instruction::I64Const(*value));
                self.call_runtime("__exs_rt_int_new", *span)
            }
            Expression::Float(value, span) => {
                self.function
                    .instruction(&Instruction::F64Const((*value).into()));
                self.call_runtime("__exs_rt_float_new", *span)
            }
            Expression::String(value, span) => self.string(value, *span),
            Expression::Bool(value, span) => {
                self.function
                    .instruction(&Instruction::I32Const(i32::from(*value)));
                self.call_runtime("__exs_rt_bool_new", *span)
            }
            Expression::None(span) => self.call_runtime("__exs_rt_none_new", *span),
            _ => Err(diagnostics(CompileDiagnostic::new(
                "E0999",
                expression_span(expression),
                "invalid scalar continuation literal",
            ))),
        }
    }

    /// Emits one checked ExS integer construction.
    pub(super) fn integer(
        &mut self,
        value: i64,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        if !is_valid_int(value) {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0206",
                span,
                "integer literal is outside the ExS 56-bit range",
            )));
        }
        self.function.instruction(&Instruction::I64Const(value));
        self.call_runtime("__exs_rt_int_new", span)
    }

    /// Emits one compiler-owned passive-data string construction.
    pub(super) fn string(
        &mut self,
        value: &str,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let data_index = self.literals.get(value).copied().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0211",
                span,
                "missing compiler string literal data segment",
            ))
        })?;
        let length = i32::try_from(value.len()).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0211",
                span,
                "string literal is too large for Wasm linear memory",
            ))
        })?;
        self.function.instruction(&Instruction::I32Const(length));
        self.call_runtime("__exs_rt_literal_buffer_alloc", span)?;
        self.function
            .instruction(&Instruction::LocalTee(self.literal_buffer_local));
        self.function.instruction(&Instruction::I32Const(0));
        self.function.instruction(&Instruction::I32Const(length));
        self.function
            .instruction(&Instruction::MemoryInit { mem: 0, data_index });
        self.function
            .instruction(&Instruction::LocalGet(self.literal_buffer_local));
        self.function.instruction(&Instruction::I32Const(length));
        self.call_runtime("__exs_rt_string_new", span)
    }

    /// Emits a named runtime ABI call after setting its source position.
    pub(super) fn call_runtime(
        &mut self,
        name: &str,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        if name != "__exs_rt_set_source_position" {
            let position = self.source_map.id(span).ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0214",
                    span,
                    "missing source-map position for generated runtime call",
                ))
            })?;
            self.function
                .instruction(&Instruction::I32Const(position.cast_signed()));
            self.function.instruction(&Instruction::Call(
                self.runtime_index("__exs_rt_set_source_position", span)?,
            ));
        }
        self.function
            .instruction(&Instruction::Call(self.runtime_index(name, span)?));
        Ok(())
    }

    /// Resolves one stable runtime ABI export index.
    pub(super) fn runtime_index(
        &self,
        name: &str,
        span: SourceSpan<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        runtime_index(self.runtime, name, span)
    }
}

/// Returns the runtime ABI operation implementing one binary source operator.
pub(super) fn binary_operation(operator: BinaryOperator) -> &'static str {
    match operator {
        BinaryOperator::Add => "__exs_rt_add",
        BinaryOperator::Subtract => "__exs_rt_sub",
        BinaryOperator::Multiply => "__exs_rt_mul",
        BinaryOperator::Equal => "__exs_rt_eq",
        BinaryOperator::NotEqual => "__exs_rt_ne",
        BinaryOperator::LessThan => "__exs_rt_lt",
        BinaryOperator::LessOrEqual => "__exs_rt_le",
        BinaryOperator::GreaterThan => "__exs_rt_gt",
        BinaryOperator::GreaterOrEqual => "__exs_rt_ge",
        BinaryOperator::And | BinaryOperator::Or => unreachable!(),
    }
}
