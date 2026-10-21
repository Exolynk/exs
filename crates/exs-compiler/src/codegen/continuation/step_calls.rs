//! Call-boundary and utility Wasm emission for continuation frames.

use exs_abi::{
    ASYNC_FRAME_SLOTS_OFFSET, ASYNC_FRAME_STATE_OFFSET, STATUS_COMPLETE, STATUS_PENDING,
    STATUS_READY, TYPE_ERROR,
};
use exs_value::ValueRef;
use wasm_encoder::{BlockType, Instruction, MemArg, ValType};

use crate::ast::{BinaryOperator, Expression};
use crate::codegen::builtin_calls::builtin_call;
use crate::codegen::function::InstanceMethod;
use crate::codegen::source_map::requires_source_position;
use crate::codegen::trait_registry::TraitOperator;
use crate::codegen::types;
use crate::codegen::types::TypeContract;
use crate::codegen::{CompileDiagnostic, CompileDiagnostics, SourceSpan};
use crate::codegen::{diagnostics, immediate};

use super::entry::runtime_index;
use super::graph::{ComparisonFallback, expression_span};
use super::step::StepCompiler;

impl<'source, 'context> StepCompiler<'source, 'context> {
    /// Validates every value in one frame-backed packed variadic parameter List.
    pub(super) fn validate_variadic_slot_or_complete(
        &mut self,
        slot: u32,
        contract: &TypeContract,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.get_slot(slot, span)?;
        self.call_runtime("__exs_rt_list_length", span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.variadic_length_local));
        self.function.instruction(&Instruction::I32Const(0));
        self.function
            .instruction(&Instruction::LocalSet(self.variadic_index_local));
        self.function
            .instruction(&Instruction::Block(BlockType::Empty));
        self.function
            .instruction(&Instruction::Loop(BlockType::Empty));
        self.function
            .instruction(&Instruction::LocalGet(self.variadic_index_local));
        self.function
            .instruction(&Instruction::LocalGet(self.variadic_length_local));
        self.function.instruction(&Instruction::I32GeU);
        self.function.instruction(&Instruction::BrIf(1));
        self.get_slot(slot, span)?;
        self.function
            .instruction(&Instruction::LocalGet(self.variadic_index_local));
        self.call_runtime("__exs_rt_list_get", span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.scratch_local));
        self.validate_scratch_matches(contract, span)?;
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
        self.complete_local(span)?;
        self.function.instruction(&Instruction::End);
        self.function
            .instruction(&Instruction::LocalGet(self.variadic_index_local));
        self.function.instruction(&Instruction::I32Const(1));
        self.function.instruction(&Instruction::I32Add);
        self.function
            .instruction(&Instruction::LocalSet(self.variadic_index_local));
        self.function.instruction(&Instruction::Br(0));
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Emits standard operator-trait dispatch, including suspendable targets and runtime fallback.
    #[allow(clippy::too_many_arguments)] // The source operator and frame state are both required.
    pub(super) fn operator_call(
        &mut self,
        operator: TraitOperator,
        next: u32,
        left: u32,
        right: u32,
        targets: &[InstanceMethod],
        destination: u32,
        comparison: Option<ComparisonFallback>,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        // Comparisons store their final Bool where the bypassed OrderingTest state would.
        let fast_path_target = match immediate::IntOperation::from_trait(operator) {
            Some(operation) if operation.is_comparison() => {
                comparison.map(|comparison| (operation, comparison.destination, comparison.next))
            }
            Some(operation) => Some((operation, destination, next)),
            None => None,
        };
        if let Some((operation, result, after)) = fast_path_target {
            self.int_fast_path(operation, left, right, result, after, span)?;
        }
        self.operator_call_target(
            operator,
            next,
            left,
            right,
            targets,
            0,
            destination,
            comparison,
            fast_path_target.is_some(),
            span,
        )
    }

    /// Pushes one operand, from its local when the Int fast path already loaded it.
    fn load_operand(
        &mut self,
        slot: u32,
        local: u32,
        loaded: bool,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        if loaded {
            self.function.instruction(&Instruction::LocalGet(local));
            Ok(())
        } else {
            self.get_slot(slot, span)
        }
    }

    /// Emits one branch of the statically known standard operator target chain.
    #[allow(clippy::too_many_arguments)] // Recursion keeps the generated Wasm branch chain local.
    fn operator_call_target(
        &mut self,
        operator: TraitOperator,
        next: u32,
        left: u32,
        right: u32,
        targets: &[InstanceMethod],
        index: usize,
        destination: u32,
        comparison: Option<ComparisonFallback>,
        loaded: bool,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        // When `loaded`, the operand locals still hold both slots: only branches that return
        // run between the Int fast path and each use below.
        let Some(target) = targets.get(index) else {
            self.load_operand(left, self.value_local, loaded, span)?;
            self.load_operand(right, self.operand_local, loaded, span)?;
            // Built-in comparisons produce the final Bool or TypeError value directly and
            // skip the OrderingTest state, like synchronous lowering.
            if let Some(comparison) = comparison {
                self.function
                    .instruction(&Instruction::I32Const(comparison.test));
                self.call_runtime("__exs_rt_compare_test", span)?;
                self.set_slot(comparison.destination, span)?;
                return self.ready(comparison.next, span);
            }
            self.call_runtime(operator.runtime_export(), span)?;
            self.set_slot(destination, span)?;
            return self.ready(next, span);
        };
        self.load_operand(left, self.value_local, loaded, span)?;
        self.emit_object_type_test(target.type_id, span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        if let Some(layout) = self
            .frame_layouts
            .values()
            .find(|layout| layout.function_id == target.signature.function_id)
            .copied()
        {
            self.child_call(
                next,
                layout,
                target.signature.index,
                &[left, right],
                destination,
                span,
            )?;
        } else {
            self.get_slot(left, span)?;
            self.get_slot(right, span)?;
            self.set_call_site(span)?;
            self.function
                .instruction(&Instruction::Call(target.signature.index));
            self.set_slot(destination, span)?;
            self.complete_if_fatal_error(destination, span)?;
            self.ready(next, span)?;
        }
        self.function.instruction(&Instruction::Else);
        self.operator_call_target(
            operator,
            next,
            left,
            right,
            targets,
            index + 1,
            destination,
            comparison,
            loaded,
            span,
        )?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }

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
        fallback: Option<&crate::codegen::function::FunctionSignature>,
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
            fallback,
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
        fallback: Option<&crate::codegen::function::FunctionSignature>,
        index: usize,
        destination: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let Some(target) = targets.get(index) else {
            if let Some(fallback) = fallback {
                return self.standard_callback_call(
                    next,
                    receiver,
                    arguments,
                    fallback,
                    destination,
                    span,
                );
            }
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
        self.emit_object_type_test(target.type_id, span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        if !target.signature.accepts_arity(arguments.len() + 1) {
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
            let fixed_arity = target.signature.arity.saturating_sub(1);
            let mut child_arguments = Vec::with_capacity(target.signature.wasm_arity());
            child_arguments.push(receiver);
            child_arguments.extend(arguments.iter().take(fixed_arity));
            if target.signature.variadic {
                self.pack_variadic_tail(arguments, fixed_arity, destination, span)?;
                child_arguments.push(destination);
            }
            self.child_call(
                next,
                layout,
                target.signature.index,
                &child_arguments,
                destination,
                span,
            )?;
        } else {
            let fixed_arity = target.signature.arity.saturating_sub(1);
            self.get_slot(receiver, span)?;
            for argument in arguments.iter().take(fixed_arity) {
                self.get_slot(*argument, span)?;
            }
            if target.signature.variadic {
                self.pack_variadic_tail(arguments, fixed_arity, destination, span)?;
                self.get_slot(destination, span)?;
            }
            self.set_call_site(span)?;
            self.function
                .instruction(&Instruction::Call(target.signature.index));
            self.set_slot(destination, span)?;
            self.complete_if_fatal_error(destination, span)?;
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
            fallback,
            index + 1,
            destination,
            span,
        )?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Calls one private standard callback helper after nominal method dispatch misses.
    fn standard_callback_call(
        &mut self,
        next: u32,
        receiver: u32,
        arguments: &[u32],
        signature: &crate::codegen::function::FunctionSignature,
        destination: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        if !signature.accepts_arity(arguments.len() + 1) {
            self.get_slot(receiver, span)?;
            self.call_runtime("__exs_rt_method_arity_error", span)?;
            self.set_slot(destination, span)?;
            return self.ready(next, span);
        }
        if let Some(layout) = self
            .frame_layouts
            .values()
            .find(|layout| layout.function_id == signature.function_id)
            .copied()
        {
            let mut child_arguments = Vec::with_capacity(arguments.len() + 1);
            child_arguments.push(receiver);
            child_arguments.extend_from_slice(arguments);
            return self.child_call(
                next,
                layout,
                signature.index,
                &child_arguments,
                destination,
                span,
            );
        }
        self.get_slot(receiver, span)?;
        for argument in arguments {
            self.get_slot(*argument, span)?;
        }
        self.set_call_site(span)?;
        self.function
            .instruction(&Instruction::Call(signature.index));
        self.set_slot(destination, span)?;
        self.complete_if_fatal_error(destination, span)?;
        self.ready(next, span)
    }

    /// Packs a source argument suffix into a durable List slot for one variadic child call.
    fn pack_variadic_tail(
        &mut self,
        arguments: &[u32],
        fixed_arity: usize,
        destination: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.call_runtime("__exs_rt_list_new", span)?;
        self.set_slot(destination, span)?;
        for argument in arguments.iter().skip(fixed_arity) {
            self.get_slot(destination, span)?;
            self.get_slot(*argument, span)?;
            self.call_runtime("__exs_rt_append", span)?;
            self.function.instruction(&Instruction::Drop);
        }
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
        if let Some(call) = builtin_call(method, arguments.len()) {
            self.get_slot(receiver, span)?;
            if let Some(method) = call.method {
                self.function.instruction(&Instruction::I32Const(method));
            }
            for argument in arguments {
                self.get_slot(*argument, span)?;
            }
            self.call_runtime(call.export, span)?;
            self.set_slot(destination, span)?;
            return self.ready(next, span);
        }
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
        self.function
            .instruction(&Instruction::LocalSet(self.value_local));
        self.get_slot(receiver, span)?;
        self.function
            .instruction(&Instruction::LocalGet(self.value_local));
        self.get_slot(destination, span)?;
        self.call_runtime("__exs_rt_call_method", span)?;
        self.set_slot(destination, span)?;
        self.ready(next, span)
    }

    /// Stores a value in a durable frame slot.
    pub(super) fn set_slot(
        &mut self,
        slot: u32,
        _span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.function
            .instruction(&Instruction::LocalSet(self.scratch_local));
        self.function
            .instruction(&Instruction::LocalGet(self.block_local));
        self.function
            .instruction(&Instruction::LocalGet(self.scratch_local));
        let memory = self.slot_memory(slot);
        self.function.instruction(&Instruction::I64Store(memory));
        Ok(())
    }

    /// Returns the memory operand of one slot of this frame and records that it is used.
    pub(super) fn slot_memory(&mut self, slot: u32) -> MemArg {
        self.used_slots = self.used_slots.max(slot.saturating_add(1));
        slot_memory(slot)
    }

    /// Stores the state that runs when the driver next dispatches this frame.
    pub(super) fn store_state(&mut self, state: u32) {
        self.function
            .instruction(&Instruction::LocalGet(self.block_local));
        self.function
            .instruction(&Instruction::I32Const(state.cast_signed()));
        self.function.instruction(&Instruction::I32Store(MemArg {
            offset: u64::from(ASYNC_FRAME_STATE_OFFSET),
            align: 2,
            memory_index: 0,
        }));
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
        crate::codegen::registers::store_call_site(
            self.function.raw(),
            self.registers_address,
            position,
        );
        Ok(())
    }

    /// Loads a durable frame slot onto the Wasm stack.
    pub(super) fn get_slot(
        &mut self,
        slot: u32,
        _span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let memory = self.slot_memory(slot);
        self.function
            .instruction(&Instruction::LocalGet(self.block_local));
        self.function.instruction(&Instruction::I64Load(memory));
        Ok(())
    }

    /// Transfers control to another state of this frame without returning to the driver.
    ///
    /// A later state is reached by branching to the end of its dispatch block; an earlier
    /// state re-enters the dispatch loop through the state local.
    pub(super) fn ready(
        &mut self,
        next: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let depth = self.function.depth();
        let label = if next > self.current_state {
            self.loop_depth
                .checked_add(self.state_count)
                .and_then(|end| end.checked_sub(next))
                .and_then(|target| depth.checked_sub(target))
        } else {
            self.function
                .instruction(&Instruction::I32Const(next.cast_signed()));
            self.function
                .instruction(&Instruction::LocalSet(self.state_local));
            depth.checked_sub(self.loop_depth)
        };
        let label = label.ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "invalid continuation state transition",
            ))
        })?;
        self.function.instruction(&Instruction::Br(label));
        Ok(())
    }

    /// Persists the state to run when the driver next dispatches this frame and yields to it.
    pub(super) fn yield_ready(&mut self, next: u32) {
        self.store_state(next);
        self.function
            .instruction(&Instruction::I32Const(STATUS_READY));
        self.function.instruction(&Instruction::Return);
    }

    /// Consumes one scheduler checkpoint and yields to the driver when the task was rotated.
    ///
    /// The quantum is decremented inline; the runtime only runs when it would be exhausted.
    pub(super) fn checkpoint(
        &mut self,
        next: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        for instruction in crate::codegen::registers::checkpoint_countdown(self.registers_address) {
            self.function.instruction(&instruction);
        }
        self.call_runtime("__exs_rt_scheduler_checkpoint", span)?;
        self.function.instruction(&Instruction::I32Eqz);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.yield_ready(next);
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Stores the host-resume state and returns pending status.
    pub(super) fn ready_pending(
        &mut self,
        resume: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.store_state(resume);
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
        self.function
            .instruction(&Instruction::LocalGet(self.scratch_local));
        self.call_runtime("__exs_rt_async_return", span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.completion_local));
        self.function
            .instruction(&Instruction::LocalGet(self.completion_local));
        self.function.instruction(&Instruction::I32Const(1));
        self.function.instruction(&Instruction::I32Eq);
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        self.function
            .instruction(&Instruction::I32Const(STATUS_COMPLETE));
        self.function.instruction(&Instruction::Else);
        self.function
            .instruction(&Instruction::LocalGet(self.completion_local));
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
        immediate::type_mask_test(self.function.raw(), TYPE_ERROR);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.complete(value, span)?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Completes the current task when one durable slot contains a fatal language Error.
    pub(super) fn complete_if_fatal_error(
        &mut self,
        value: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        // Only Errors can be fatal, so other values skip the runtime test.
        self.get_slot(value, span)?;
        immediate::type_mask_test(self.function.raw(), TYPE_ERROR);
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        self.get_slot(value, span)?;
        self.call_runtime("__exs_rt_is_fatal_error", span)?;
        self.function.instruction(&Instruction::Else);
        self.function.instruction(&Instruction::I32Const(0));
        self.function.instruction(&Instruction::End);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.get_slot(value, span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.scratch_local));
        self.complete_local(span)?;
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
        // Every Bool is an immediate, so only non-Bool values take the runtime error path.
        self.get_slot(value, span)?;
        self.function
            .instruction(&Instruction::LocalTee(self.value_local));
        immediate::is_bool_test(self.function.raw());
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        self.function
            .instruction(&Instruction::LocalGet(self.value_local));
        self.function.instruction(&Instruction::I32WrapI64);
        self.function.instruction(&Instruction::Else);
        self.function
            .instruction(&Instruction::LocalGet(self.value_local));
        self.call_runtime("__exs_rt_condition_value", span)?;
        self.set_slot(checked, span)?;
        self.complete_if_error(checked, span)?;
        self.get_slot(checked, span)?;
        self.call_runtime("__exs_rt_condition", span)?;
        self.function.instruction(&Instruction::End);
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
        immediate::type_mask_test(self.function.raw(), contract.builtin_mask);
        self.function.instruction(&Instruction::LocalSet(2));
        for type_id in &contract.nominal_type_ids {
            self.function.instruction(&Instruction::LocalGet(2));
            self.get_slot(slot, span)?;
            self.emit_object_type_test(*type_id, span)?;
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
        immediate::type_mask_test(self.function.raw(), contract.builtin_mask);
        self.function.instruction(&Instruction::LocalSet(2));
        for type_id in &contract.nominal_type_ids {
            self.function.instruction(&Instruction::LocalGet(2));
            self.function
                .instruction(&Instruction::LocalGet(self.scratch_local));
            self.emit_object_type_test(*type_id, span)?;
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
                let value = i64::try_from(*value).map_err(|_| {
                    diagnostics(CompileDiagnostic::new(
                        "E0206",
                        *span,
                        "integer literal is outside the ExS signed 64-bit range",
                    ))
                })?;
                self.integer(value, *span)
            }
            Expression::Float(value, span) => {
                self.function
                    .instruction(&Instruction::F64Const((*value).into()));
                self.call_runtime("__exs_rt_float_new", *span)
            }
            Expression::String(value, span) => self.string(value, *span),
            Expression::Bytes(value, span) => self.bytes(value, *span),
            Expression::Bool(value, _) => {
                immediate::push(self.function.raw(), ValueRef::boolean(*value));
                Ok(())
            }
            Expression::None(_) => {
                immediate::push(self.function.raw(), ValueRef::NONE);
                Ok(())
            }
            _ => Err(diagnostics(CompileDiagnostic::new(
                "E0999",
                expression_span(expression),
                "invalid scalar continuation literal",
            ))),
        }
    }

    /// Emits one i64 ExS integer, as an immediate constant when it fits the `i32` range.
    pub(super) fn integer(
        &mut self,
        value: i64,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        if let Some(immediate) = ValueRef::small_int(value) {
            immediate::push(self.function.raw(), immediate);
            return Ok(());
        }
        self.function.instruction(&Instruction::I64Const(value));
        self.call_runtime("__exs_rt_int_new", span)
    }

    /// Replaces the ValueRef on the stack with the i32 result of one nominal Object type test.
    pub(super) fn emit_object_type_test(
        &mut self,
        type_id: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        if immediate::object_type_test(self.function.raw(), type_id) {
            return Ok(());
        }
        self.function
            .instruction(&Instruction::I32Const(type_id.cast_signed()));
        self.call_runtime("__exs_rt_object_is_type", span)
    }

    /// Converts the Bool ValueRef on the stack to a Wasm condition, trapping for non-Bools.
    pub(super) fn bool_condition(
        &mut self,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.function
            .instruction(&Instruction::LocalTee(self.value_local));
        immediate::is_bool_test(self.function.raw());
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        self.function
            .instruction(&Instruction::LocalGet(self.value_local));
        self.function.instruction(&Instruction::I32WrapI64);
        self.function.instruction(&Instruction::Else);
        self.function
            .instruction(&Instruction::LocalGet(self.value_local));
        self.call_runtime("__exs_rt_condition", span)?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Stores an inline Int result and readies `next` when both slots hold immediate Ints.
    ///
    /// Falls through without side effects otherwise, so the caller emits its general path next.
    pub(super) fn int_fast_path(
        &mut self,
        operation: immediate::IntOperation,
        left: u32,
        right: u32,
        destination: u32,
        next: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let int_new = self.runtime_index("__exs_rt_int_new", span)?;
        self.get_slot(left, span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.value_local));
        self.get_slot(right, span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.operand_local));
        immediate::both_small_ints_test(self.function.raw(), self.value_local, self.operand_local);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        operation.emit(
            self.function.raw(),
            self.value_local,
            self.operand_local,
            self.int_result_local,
            int_new,
        );
        self.set_slot(destination, span)?;
        self.ready(next, span)?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Emits one compiler-owned passive-data string construction.
    pub(super) fn string(
        &mut self,
        value: &str,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let segment = self.literals.segment(value).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0211",
                span,
                "missing compiler string literal data segment",
            ))
        })?;
        let data_index = segment.data_index;
        let length = i32::try_from(value.len()).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0211",
                span,
                "string literal is too large for Wasm linear memory",
            ))
        })?;
        let global_index = segment.global_index;
        // Literal Strings are immutable, so each literal caches its runtime String in its own
        // global. Only the first evaluation copies the data segment into a new String, which
        // the runtime pins as a permanent GC root.
        self.function
            .instruction(&Instruction::GlobalGet(global_index));
        self.function.instruction(&Instruction::I64Eqz);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
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
        self.call_runtime("__exs_rt_string_new", span)?;
        self.call_runtime("__exs_rt_literal_string_pin", span)?;
        self.function
            .instruction(&Instruction::GlobalSet(global_index));
        self.function.instruction(&Instruction::End);
        self.function
            .instruction(&Instruction::GlobalGet(global_index));
        Ok(())
    }

    /// Emits one compiler-owned passive-data Bytes construction.
    pub(super) fn bytes(
        &mut self,
        value: &str,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let data_index = self
            .literals
            .segment(value)
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0211",
                    span,
                    "missing compiler Bytes literal data segment",
                ))
            })?
            .data_index;
        let length = i32::try_from(value.len()).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0211",
                span,
                "Bytes literal is too large for Wasm linear memory",
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
        self.call_runtime("__exs_rt_bytes_new", span)
    }

    /// Emits a named runtime ABI call, after setting its source position when it may need one.
    pub(super) fn call_runtime(
        &mut self,
        name: &str,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        if requires_source_position(name) {
            let position = self.source_map.id(span).ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0214",
                    span,
                    "missing source-map position for generated runtime call",
                ))
            })?;
            let set_source_position = self.runtime_index("__exs_rt_set_source_position", span)?;
            self.function.source_position(position, set_source_position);
        }
        let function = self.runtime_index(name, span)?;
        self.function.runtime_call(function);
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

/// Returns the memory operand addressing one slot relative to a frame block address.
pub(super) fn slot_memory(slot: u32) -> MemArg {
    MemArg {
        offset: u64::from(ASYNC_FRAME_SLOTS_OFFSET) + u64::from(slot) * 8,
        align: 3,
        memory_index: 0,
    }
}

/// Returns the runtime ABI operation implementing one binary source operator.
pub(super) fn binary_operation(operator: BinaryOperator) -> &'static str {
    match operator {
        BinaryOperator::Add => "__exs_rt_add",
        BinaryOperator::Subtract => "__exs_rt_sub",
        BinaryOperator::Multiply => "__exs_rt_mul",
        BinaryOperator::Divide => "__exs_rt_div",
        BinaryOperator::Equal => "__exs_rt_eq",
        BinaryOperator::NotEqual => "__exs_rt_ne",
        BinaryOperator::LessThan => "__exs_rt_lt",
        BinaryOperator::LessOrEqual => "__exs_rt_le",
        BinaryOperator::GreaterThan => "__exs_rt_gt",
        BinaryOperator::GreaterOrEqual => "__exs_rt_ge",
        BinaryOperator::And | BinaryOperator::Or => unreachable!(),
    }
}
