//! Wasm state dispatch and primary operation emission for continuation frames.

use std::collections::HashMap;

use exs_abi::{HOST_CALL_PENDING, HOST_CALL_READY, STATUS_READY, TYPE_OBJECT};
use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::ast::UnaryOperator;
use crate::codegen::diagnostics;
use crate::codegen::source_map::SourceMap;
use crate::codegen::types::TypeContract;
use crate::codegen::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

use super::FrameLayout;
use super::graph::{HostTimeField, Operation, operation_span};
use super::step_calls::binary_operation;

pub(super) struct StepCompiler<'source, 'context> {
    /// Runtime ABI export indexes.
    pub(super) runtime: &'context HashMap<String, u32>,
    /// Passive data indexes for compiler string literals.
    pub(super) literals: &'context HashMap<String, u32>,
    /// Compiler-assigned source positions.
    pub(super) source_map: &'context SourceMap<'source>,
    /// Durable child-frame layouts for nominal suspendable method targets.
    pub(super) frame_layouts: &'context HashMap<String, FrameLayout>,
    /// The declared return contract checked before a resumable frame completes.
    pub(super) return_contract: &'context TypeContract,
    /// Wasm function body under construction.
    pub(super) function: Function,
    /// Reused Wasm local for host status and temporary result values.
    pub(super) scratch_local: u32,
    /// Reused Wasm local for compiler literal-buffer pointers.
    pub(super) literal_buffer_local: u32,
    /// Reused Wasm local holding one variadic List length during boundary validation.
    pub(super) variadic_length_local: u32,
    /// Reused Wasm local holding one variadic List index during boundary validation.
    pub(super) variadic_index_local: u32,
    /// Cached continuation state used by the generated balanced operation dispatcher.
    pub(super) state_local: u32,
}

impl<'source, 'context> StepCompiler<'source, 'context> {
    /// Emits a balanced continuation-state dispatcher for all graph operations.
    pub(super) fn emit_dispatch(
        &mut self,
        operations: &[Operation<'source, '_>],
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.function.instruction(&Instruction::LocalGet(0));
        self.call_runtime("__exs_rt_async_frame_state", span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.state_local));
        self.emit_dispatch_range(operations, 0)
    }

    /// Emits one balanced range of continuation-state dispatch comparisons.
    fn emit_dispatch_range(
        &mut self,
        operations: &[Operation<'source, '_>],
        start: usize,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let Some((operation, remaining)) = operations.split_first() else {
            self.function.instruction(&Instruction::Unreachable);
            return Ok(());
        };
        let state = u32::try_from(start).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                operation_span(operation),
                "too many continuation states for one function",
            ))
        })?;
        if remaining.is_empty() {
            self.function
                .instruction(&Instruction::LocalGet(self.state_local));
            self.function
                .instruction(&Instruction::I32Const(state.cast_signed()));
            self.function.instruction(&Instruction::I32Eq);
            self.function
                .instruction(&Instruction::If(BlockType::Empty));
            self.emit_state(state, operation)?;
            self.function.instruction(&Instruction::End);
            self.function.instruction(&Instruction::Unreachable);
            return Ok(());
        }
        let middle = operations.len() / 2;
        let pivot = start.checked_add(middle).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                operation_span(operation),
                "too many continuation states for one function",
            ))
        })?;
        let pivot = u32::try_from(pivot).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                operation_span(operation),
                "too many continuation states for one function",
            ))
        })?;
        self.function
            .instruction(&Instruction::LocalGet(self.state_local));
        self.function
            .instruction(&Instruction::I32Const(pivot.cast_signed()));
        self.function.instruction(&Instruction::I32LtU);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.emit_dispatch_range(&operations[..middle], start)?;
        self.function.instruction(&Instruction::Else);
        self.emit_dispatch_range(&operations[middle..], start + middle)?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Emits one operation body after the dispatcher selected its exact state.
    fn emit_state(
        &mut self,
        state: u32,
        operation: &Operation<'source, '_>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.emit_operation(state, operation)?;
        Ok(())
    }

    /// Emits one operation body, which always returns a dispatcher status.
    pub(super) fn emit_operation(
        &mut self,
        state: u32,
        operation: &Operation<'source, '_>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let next = state.checked_add(1).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                operation_span(operation),
                "too many continuation states",
            ))
        })?;
        match operation {
            Operation::Literal {
                expression,
                destination,
            } => {
                self.literal(expression)?;
                self.set_slot(*destination, operation_span(operation))?;
                self.ready(next, operation_span(operation))?;
            }
            Operation::String {
                value,
                destination,
                span,
            } => {
                self.string(value, *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::BytesStatic {
                value,
                from_utf8,
                destination,
                span,
            } => {
                self.get_slot(*value, *span)?;
                self.call_runtime(
                    if *from_utf8 {
                        "__exs_rt_bytes_from_utf8"
                    } else {
                        "__exs_rt_bytes_from_list"
                    },
                    *span,
                )?;
                self.set_slot(*destination, *span)?;
                self.complete_if_error(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Integer {
                value,
                destination,
                span,
            } => {
                self.integer(*value, *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::None { destination, span } => {
                self.call_runtime("__exs_rt_none_new", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Boolean {
                value,
                destination,
                span,
            } => {
                self.function
                    .instruction(&Instruction::I32Const(i32::from(*value)));
                self.call_runtime("__exs_rt_bool_new", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Copy {
                source,
                destination,
                span,
            } => {
                self.get_slot(*source, *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::CellNew {
                value,
                destination,
                span,
            } => {
                self.get_slot(*value, *span)?;
                self.call_runtime("__exs_rt_cell_new", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::CellGet {
                cell,
                destination,
                span,
            } => {
                self.get_slot(*cell, *span)?;
                self.call_runtime("__exs_rt_cell_get", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::CellSet { cell, value, span } => {
                self.get_slot(*cell, *span)?;
                self.get_slot(*value, *span)?;
                self.call_runtime("__exs_rt_cell_set", *span)?;
                self.function.instruction(&Instruction::Drop);
                self.ready(next, *span)?;
            }
            Operation::Closure {
                layout,
                arity,
                variadic,
                captures,
                destination,
                span,
            } => {
                self.call_runtime("__exs_rt_list_new", *span)?;
                self.set_slot(*destination, *span)?;
                for capture in captures {
                    self.get_slot(*destination, *span)?;
                    self.get_slot(*capture, *span)?;
                    self.call_runtime("__exs_rt_append", *span)?;
                    self.function.instruction(&Instruction::Drop);
                }
                self.function
                    .instruction(&Instruction::I32Const(layout.function_id.cast_signed()));
                self.function
                    .instruction(&Instruction::I32Const(layout.slot_count.cast_signed()));
                let arity = i32::try_from(*arity).map_err(|_| {
                    diagnostics(CompileDiagnostic::new(
                        "E0212",
                        *span,
                        "too many closure function parameters",
                    ))
                })?;
                self.function.instruction(&Instruction::I32Const(arity));
                self.function
                    .instruction(&Instruction::I32Const(i32::from(*variadic)));
                self.get_slot(*destination, *span)?;
                self.call_runtime("__exs_rt_closure_new", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Unary {
                operator,
                operand,
                destination,
                span,
            } => {
                self.get_slot(*operand, *span)?;
                self.call_runtime(
                    match operator {
                        UnaryOperator::Negate => "__exs_rt_neg",
                        UnaryOperator::Not => "__exs_rt_not",
                    },
                    *span,
                )?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Binary {
                operator,
                left,
                right,
                destination,
                span,
            } => {
                self.get_slot(*left, *span)?;
                self.get_slot(*right, *span)?;
                self.call_runtime(binary_operation(*operator), *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Operator {
                operator,
                left,
                right,
                targets,
                destination,
                span,
            } => {
                self.operator_call(*operator, next, *left, *right, targets, *destination, *span)?
            }
            Operation::OrderingTest {
                value,
                test,
                destination,
                span,
            } => {
                self.get_slot(*value, *span)?;
                self.function.instruction(&Instruction::I32Const(*test));
                self.call_runtime("__exs_rt_ordering_test", *span)?;
                self.set_slot(*destination, *span)?;
                self.complete_if_error(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::List {
                elements,
                destination,
                span,
            } => {
                self.call_runtime("__exs_rt_list_new", *span)?;
                self.set_slot(*destination, *span)?;
                for element in elements {
                    self.get_slot(*destination, *span)?;
                    self.get_slot(*element, *span)?;
                    self.call_runtime("__exs_rt_append", *span)?;
                    self.function.instruction(&Instruction::Drop);
                }
                self.ready(next, *span)?;
            }
            Operation::Object {
                properties,
                destination,
                span,
            } => {
                self.call_runtime("__exs_rt_object_new", *span)?;
                self.set_slot(*destination, *span)?;
                for (key, key_span, value) in properties {
                    self.get_slot(*destination, *span)?;
                    self.string(key, *key_span)?;
                    self.get_slot(*value, *span)?;
                    self.call_runtime("__exs_rt_index_set", *span)?;
                    self.function.instruction(&Instruction::Drop);
                }
                self.ready(next, *span)?;
            }
            Operation::Error {
                kind,
                message,
                data,
                destination,
                span,
            } => {
                self.get_slot(*kind, *span)?;
                self.get_slot(*message, *span)?;
                self.get_slot(*data, *span)?;
                self.call_runtime("__exs_rt_error_new", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Assert {
                condition,
                actual,
                expected,
                description,
                destination,
                span,
            } => {
                self.get_slot(*condition, *span)?;
                if let (Some(actual), Some(expected)) = (actual, expected) {
                    self.get_slot(*actual, *span)?;
                    self.get_slot(*expected, *span)?;
                }
                self.get_slot(*description, *span)?;
                self.call_runtime(
                    if actual.is_some() {
                        "__exs_rt_assert_eq"
                    } else {
                        "__exs_rt_assert"
                    },
                    *span,
                )?;
                self.set_slot(*destination, *span)?;
                self.complete_if_fatal_error(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::TypedObject {
                type_id,
                destination,
                span,
            } => {
                self.function
                    .instruction(&Instruction::I32Const(type_id.cast_signed()));
                self.call_runtime("__exs_rt_object_typed_new", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Enum {
                type_id,
                type_identity,
                variant,
                fields,
                type_identity_slot,
                variant_slot,
                destination,
                span,
            } => {
                self.string(type_identity, *span)?;
                self.set_slot(*type_identity_slot, *span)?;
                self.string(variant, *span)?;
                self.set_slot(*variant_slot, *span)?;
                self.call_runtime("__exs_rt_list_new", *span)?;
                self.set_slot(*destination, *span)?;
                for field in fields {
                    self.get_slot(*destination, *span)?;
                    self.get_slot(*field, *span)?;
                    self.call_runtime("__exs_rt_append", *span)?;
                    self.function.instruction(&Instruction::Drop);
                }
                self.function
                    .instruction(&Instruction::I32Const(type_id.cast_signed()));
                self.get_slot(*type_identity_slot, *span)?;
                self.get_slot(*variant_slot, *span)?;
                self.get_slot(*destination, *span)?;
                self.call_runtime("__exs_rt_enum_new", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::EnumMatches {
                value,
                type_identity,
                variant,
                type_identity_slot,
                variant_slot,
                destination,
                span,
            } => {
                self.string(type_identity, *span)?;
                self.set_slot(*type_identity_slot, *span)?;
                self.string(variant, *span)?;
                self.set_slot(*variant_slot, *span)?;
                self.get_slot(*value, *span)?;
                self.get_slot(*type_identity_slot, *span)?;
                self.get_slot(*variant_slot, *span)?;
                self.call_runtime("__exs_rt_enum_matches", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::EnumField {
                value,
                index,
                destination,
                span,
            } => {
                self.get_slot(*value, *span)?;
                self.function
                    .instruction(&Instruction::I32Const(index.cast_signed()));
                self.call_runtime("__exs_rt_enum_field", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::MatchError {
                value,
                destination,
                span,
            } => {
                self.get_slot(*value, *span)?;
                self.call_runtime("__exs_rt_match_error", *span)?;
                self.set_slot(*destination, *span)?;
                self.complete_if_error(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::IsError {
                value,
                destination,
                span,
            } => {
                self.get_slot(*value, *span)?;
                self.call_runtime("__exs_rt_is_error", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Propagate {
                value,
                destination,
                span,
            } => {
                self.get_slot(*value, *span)?;
                self.call_runtime("__exs_rt_propagate", *span)?;
                self.set_slot(*destination, *span)?;
                self.get_slot(*destination, *span)?;
                self.call_runtime("__exs_rt_is_error", *span)?;
                self.call_runtime("__exs_rt_condition", *span)?;
                self.function
                    .instruction(&Instruction::If(BlockType::Empty));
                self.complete(*destination, *span)?;
                self.function.instruction(&Instruction::End);
                self.ready(next, *span)?;
            }
            Operation::Index {
                receiver,
                index,
                destination,
                span,
            } => {
                self.get_slot(*receiver, *span)?;
                self.get_slot(*index, *span)?;
                self.call_runtime("__exs_rt_index_get", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Property {
                receiver,
                property,
                property_span,
                destination,
                span,
            } => {
                self.get_slot(*receiver, *span)?;
                self.string(property, *property_span)?;
                self.call_runtime("__exs_rt_index_get", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::IndexSet {
                receiver,
                index,
                value,
                span,
            } => {
                self.get_slot(*receiver, *span)?;
                self.get_slot(*index, *span)?;
                self.get_slot(*value, *span)?;
                self.call_runtime("__exs_rt_index_set", *span)?;
                self.function.instruction(&Instruction::Drop);
                self.ready(next, *span)?;
            }
            Operation::PropertySet {
                receiver,
                property,
                property_span,
                value,
                span,
            } => {
                self.get_slot(*receiver, *span)?;
                self.string(property, *property_span)?;
                self.get_slot(*value, *span)?;
                self.call_runtime("__exs_rt_index_set", *span)?;
                self.function.instruction(&Instruction::Drop);
                self.ready(next, *span)?;
            }
            Operation::HostCall {
                name,
                arguments,
                argument_list,
                destination,
                span,
            } => self.host_call(state, *name, arguments, *argument_list, *destination, *span)?,
            Operation::HostResume { destination, span } => {
                self.call_runtime("__exs_rt_host_call_take_ready", *span)?;
                self.set_slot(*destination, *span)?;
                self.complete_if_fatal_error(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::HostStream {
                handle,
                type_id,
                handle_contract,
                destination,
                span,
            } => {
                self.get_slot(*handle, *span)?;
                self.call_runtime("__exs_rt_is_error", *span)?;
                self.call_runtime("__exs_rt_condition", *span)?;
                self.function
                    .instruction(&Instruction::If(BlockType::Empty));
                self.get_slot(*handle, *span)?;
                self.set_slot(*destination, *span)?;
                self.function.instruction(&Instruction::Else);
                self.validate_slot_matches(*handle, handle_contract, *span)?;
                self.function.instruction(&Instruction::LocalGet(2));
                self.function.instruction(&Instruction::I32Eqz);
                self.function
                    .instruction(&Instruction::If(BlockType::Empty));
                self.get_slot(*handle, *span)?;
                self.function.instruction(&Instruction::I32Const(1));
                self.call_runtime("__exs_rt_type_mismatch", *span)?;
                self.set_slot(*destination, *span)?;
                self.function.instruction(&Instruction::Else);
                self.function
                    .instruction(&Instruction::I32Const(type_id.cast_signed()));
                self.call_runtime("__exs_rt_object_typed_new", *span)?;
                self.set_slot(*destination, *span)?;
                self.get_slot(*destination, *span)?;
                self.string("handle", *span)?;
                self.get_slot(*handle, *span)?;
                self.call_runtime("__exs_rt_index_set", *span)?;
                self.function.instruction(&Instruction::Drop);
                self.function.instruction(&Instruction::End);
                self.function.instruction(&Instruction::End);
                self.ready(next, *span)?;
            }
            Operation::HostTime {
                value,
                type_id,
                fields,
                destination,
                span,
            } => self.host_time(next, *value, *type_id, fields, *destination, *span)?,
            Operation::DirectCall {
                signature,
                arguments,
                destination,
                span,
            } => {
                for argument in arguments {
                    self.get_slot(*argument, *span)?;
                }
                self.set_call_site(*span)?;
                self.function
                    .instruction(&Instruction::Call(signature.index));
                self.set_slot(*destination, *span)?;
                self.complete_if_fatal_error(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::ChildCall {
                layout,
                arguments,
                destination,
                span,
            } => self.child_call(next, *layout, arguments, *destination, *span)?,
            Operation::ClosureCall {
                closure,
                arguments,
                destination,
                span,
            } => self.closure_call(next, *closure, arguments, *destination, *span)?,
            Operation::ParallelStart {
                tasks,
                destination,
                span,
            } => {
                self.parallel_start(next, tasks, *destination, *span)?;
            }
            Operation::ParallelDynamicStart {
                functions,
                destination,
                span,
            } => {
                self.parallel_dynamic_start(next, *functions, *destination, *span)?;
            }
            Operation::ParallelTake {
                group,
                destination,
                span,
            } => {
                self.get_slot(*group, *span)?;
                self.call_runtime("__exs_rt_parallel_take_results", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::InstanceCall {
                receiver,
                method,
                method_span,
                arguments,
                targets,
                destination,
                span,
            } => self.instance_call(
                next,
                *receiver,
                method,
                *method_span,
                arguments,
                targets,
                *destination,
                *span,
            )?,
            Operation::ValidateParameters {
                contracts,
                offset,
                span,
            } => {
                for (slot, contract) in contracts.iter().enumerate() {
                    let slot = u32::try_from(slot).map_err(|_| {
                        diagnostics(CompileDiagnostic::new(
                            "E0212",
                            *span,
                            "too many continuation parameter slots",
                        ))
                    })?;
                    let slot = offset.checked_add(slot).ok_or_else(|| {
                        diagnostics(CompileDiagnostic::new(
                            "E0212",
                            *span,
                            "too many continuation parameter slots",
                        ))
                    })?;
                    self.validate_slot_or_complete(slot, contract, *span)?;
                }
                self.ready(next, *span)?;
            }
            Operation::ValidateVariadicParameter {
                slot,
                contract,
                span,
            } => {
                self.validate_variadic_slot_or_complete(*slot, contract, *span)?;
                self.ready(next, *span)?;
            }
            Operation::ValidateSlot {
                slot,
                contract,
                span,
            } => {
                self.validate_slot_or_complete(*slot, contract, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Branch {
                condition,
                checked,
                when_true,
                when_false,
                span,
            } => self.branch_on_value(*condition, *checked, *when_true, *when_false, *span)?,
            Operation::Goto {
                target,
                checkpoint,
                span,
            } => {
                if *checkpoint {
                    self.call_runtime("__exs_rt_scheduler_checkpoint", *span)?;
                }
                self.ready(*target, *span)?;
            }
            Operation::IterSnapshot {
                iterable,
                destination,
                span,
            } => {
                self.get_slot(*iterable, *span)?;
                self.call_runtime("__exs_rt_iter_snapshot", *span)?;
                self.set_slot(*destination, *span)?;
                self.complete_if_error(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::IteratorBranch {
                step,
                item,
                checked,
                type_identity_slot,
                done_variant_slot,
                item_variant_slot,
                type_identity,
                done_variant,
                item_variant,
                when_item,
                when_done,
                span,
            } => {
                self.complete_if_error(*step, *span)?;
                self.string(type_identity, *span)?;
                self.set_slot(*type_identity_slot, *span)?;
                self.string(done_variant, *span)?;
                self.set_slot(*done_variant_slot, *span)?;
                self.get_slot(*step, *span)?;
                self.get_slot(*type_identity_slot, *span)?;
                self.get_slot(*done_variant_slot, *span)?;
                self.call_runtime("__exs_rt_enum_matches", *span)?;
                self.set_slot(*checked, *span)?;
                self.complete_if_error(*checked, *span)?;
                self.get_slot(*checked, *span)?;
                self.call_runtime("__exs_rt_condition", *span)?;
                self.function
                    .instruction(&Instruction::If(BlockType::Empty));
                self.ready(*when_done, *span)?;
                self.function.instruction(&Instruction::Else);
                self.string(item_variant, *span)?;
                self.set_slot(*item_variant_slot, *span)?;
                self.get_slot(*step, *span)?;
                self.get_slot(*type_identity_slot, *span)?;
                self.get_slot(*item_variant_slot, *span)?;
                self.call_runtime("__exs_rt_enum_matches", *span)?;
                self.set_slot(*checked, *span)?;
                self.get_slot(*checked, *span)?;
                self.call_runtime("__exs_rt_condition", *span)?;
                self.function
                    .instruction(&Instruction::If(BlockType::Empty));
                self.get_slot(*step, *span)?;
                self.function.instruction(&Instruction::I32Const(0));
                self.call_runtime("__exs_rt_enum_field", *span)?;
                self.set_slot(*item, *span)?;
                self.complete_if_error(*item, *span)?;
                self.ready(*when_item, *span)?;
                self.function.instruction(&Instruction::Else);
                self.get_slot(*step, *span)?;
                self.call_runtime("__exs_rt_match_error", *span)?;
                self.set_slot(*checked, *span)?;
                self.complete_if_error(*checked, *span)?;
                self.function.instruction(&Instruction::End);
                self.function.instruction(&Instruction::End);
            }
            Operation::Return { value, span } => self.complete(*value, *span)?,
        }
        Ok(())
    }

    /// Emits a host-call boundary and its synchronous fast path.
    pub(super) fn host_call(
        &mut self,
        state: u32,
        name: u32,
        arguments: &[u32],
        argument_list: u32,
        destination: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let resume = state.checked_add(1).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                span,
                "too many continuation states",
            ))
        })?;
        let after_resume = resume.checked_add(1).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                span,
                "too many continuation states",
            ))
        })?;
        self.call_runtime("__exs_rt_list_new", span)?;
        self.set_slot(argument_list, span)?;
        for argument in arguments {
            self.get_slot(argument_list, span)?;
            self.get_slot(*argument, span)?;
            self.call_runtime("__exs_rt_append", span)?;
            self.function.instruction(&Instruction::Drop);
        }
        self.get_slot(name, span)?;
        self.get_slot(argument_list, span)?;
        self.call_runtime("__exs_rt_host_call_start", span)?;
        self.function.instruction(&Instruction::LocalSet(2));
        self.function.instruction(&Instruction::LocalGet(2));
        self.function
            .instruction(&Instruction::I32Const(HOST_CALL_READY));
        self.function.instruction(&Instruction::I32Eq);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.call_runtime("__exs_rt_host_call_take_ready", span)?;
        self.set_slot(destination, span)?;
        self.complete_if_fatal_error(destination, span)?;
        self.ready(after_resume, span)?;
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::LocalGet(2));
        self.function
            .instruction(&Instruction::I32Const(HOST_CALL_PENDING));
        self.function.instruction(&Instruction::I32Eq);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.ready_pending(resume, span)?;
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::Unreachable);
        Ok(())
    }

    /// Starts one suspendable child frame and transfers dispatch to it.
    pub(super) fn child_call(
        &mut self,
        next: u32,
        layout: FrameLayout,
        arguments: &[u32],
        destination: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let slot_count = i32::try_from(layout.slot_count).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                span,
                "too many child continuation frame slots",
            ))
        })?;
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::I32Const(next.cast_signed()));
        self.call_runtime("__exs_rt_async_frame_set_state", span)?;
        self.set_call_site(span)?;
        self.function
            .instruction(&Instruction::I32Const(layout.function_id.cast_signed()));
        self.function
            .instruction(&Instruction::I32Const(slot_count));
        self.call_runtime("__exs_rt_async_frame_new", span)?;
        self.function.instruction(&Instruction::LocalSet(2));
        self.function
            .instruction(&Instruction::I32Const(layout.function_id.cast_signed()));
        self.call_runtime("__exs_rt_frame_push", span)?;
        for (slot, argument) in arguments.iter().enumerate() {
            let slot = i32::try_from(slot).map_err(|_| {
                diagnostics(CompileDiagnostic::new(
                    "E0212",
                    span,
                    "too many child function arguments",
                ))
            })?;
            self.function.instruction(&Instruction::LocalGet(2));
            self.function.instruction(&Instruction::I32Const(slot));
            self.get_slot(*argument, span)?;
            self.call_runtime("__exs_rt_async_frame_set_slot", span)?;
        }
        self.function.instruction(&Instruction::LocalGet(2));
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::I32Const(destination.cast_signed()));
        self.call_runtime("__exs_rt_async_frame_set_caller", span)?;
        self.call_runtime("__exs_rt_scheduler_checkpoint", span)?;
        self.function
            .instruction(&Instruction::I32Const(STATUS_READY));
        self.function.instruction(&Instruction::Return);
        Ok(())
    }
    /// Starts one dynamically selected closure frame and transfers dispatch to it.
    fn closure_call(
        &mut self,
        next: u32,
        closure: u32,
        arguments: &[u32],
        destination: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::I32Const(next.cast_signed()));
        self.call_runtime("__exs_rt_async_frame_set_state", span)?;
        self.set_call_site(span)?;

        self.get_slot(closure, span)?;
        self.call_runtime("__exs_rt_is_closure", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.get_slot(closure, span)?;
        self.call_runtime("__exs_rt_closure_arity", span)?;
        self.function.instruction(&Instruction::LocalSet(5));
        self.get_slot(closure, span)?;
        self.call_runtime("__exs_rt_closure_is_variadic", span)?;
        self.function.instruction(&Instruction::LocalSet(6));
        self.function.instruction(&Instruction::LocalGet(6));
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        self.function.instruction(&Instruction::I32Const(
            i32::try_from(arguments.len()).map_err(|_| {
                diagnostics(CompileDiagnostic::new(
                    "E0212",
                    span,
                    "too many closure function arguments",
                ))
            })?,
        ));
        self.function.instruction(&Instruction::LocalGet(5));
        self.function.instruction(&Instruction::I32LtU);
        self.function.instruction(&Instruction::Else);
        let argument_count = i32::try_from(arguments.len()).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                span,
                "too many closure function arguments",
            ))
        })?;
        self.function.instruction(&Instruction::LocalGet(5));
        self.function
            .instruction(&Instruction::I32Const(argument_count));
        self.function.instruction(&Instruction::I32Ne);
        self.function.instruction(&Instruction::End);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.call_runtime("__exs_rt_closure_arity_error", span)?;
        self.set_slot(destination, span)?;
        self.ready(next, span)?;
        self.function.instruction(&Instruction::End);

        self.function.instruction(&Instruction::LocalGet(6));
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.call_runtime("__exs_rt_list_new", span)?;
        self.set_slot(destination, span)?;
        self.function.instruction(&Instruction::End);

        self.get_slot(closure, span)?;
        self.call_runtime("__exs_rt_closure_function", span)?;
        self.function.instruction(&Instruction::LocalSet(3));
        self.get_slot(closure, span)?;
        self.call_runtime("__exs_rt_closure_slot_count", span)?;
        self.function.instruction(&Instruction::LocalSet(4));
        self.function.instruction(&Instruction::LocalGet(3));
        self.function.instruction(&Instruction::LocalGet(4));
        self.call_runtime("__exs_rt_async_frame_new", span)?;
        self.function.instruction(&Instruction::LocalSet(2));
        self.function.instruction(&Instruction::LocalGet(3));
        self.call_runtime("__exs_rt_frame_push", span)?;

        self.get_slot(closure, span)?;
        self.call_runtime("__exs_rt_closure_capture_count", span)?;
        self.function.instruction(&Instruction::LocalSet(4));
        self.function.instruction(&Instruction::I32Const(0));
        self.function.instruction(&Instruction::LocalSet(5));
        self.function
            .instruction(&Instruction::Block(BlockType::Empty));
        self.function
            .instruction(&Instruction::Loop(BlockType::Empty));
        self.function.instruction(&Instruction::LocalGet(5));
        self.function.instruction(&Instruction::LocalGet(4));
        self.function.instruction(&Instruction::I32GeU);
        self.function.instruction(&Instruction::BrIf(1));
        self.function.instruction(&Instruction::LocalGet(2));
        self.function.instruction(&Instruction::LocalGet(5));
        self.get_slot(closure, span)?;
        self.function.instruction(&Instruction::LocalGet(5));
        self.call_runtime("__exs_rt_closure_capture", span)?;
        self.call_runtime("__exs_rt_async_frame_set_slot", span)?;
        self.function.instruction(&Instruction::LocalGet(5));
        self.function.instruction(&Instruction::I32Const(1));
        self.function.instruction(&Instruction::I32Add);
        self.function.instruction(&Instruction::LocalSet(5));
        self.function.instruction(&Instruction::Br(0));
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::End);
        for (index, argument) in arguments.iter().enumerate() {
            let index = i32::try_from(index).map_err(|_| {
                diagnostics(CompileDiagnostic::new(
                    "E0212",
                    span,
                    "too many closure function arguments",
                ))
            })?;
            self.function.instruction(&Instruction::I32Const(index));
            self.function.instruction(&Instruction::LocalGet(5));
            self.function.instruction(&Instruction::I32LtU);
            self.function
                .instruction(&Instruction::If(BlockType::Empty));
            self.function.instruction(&Instruction::LocalGet(2));
            self.function.instruction(&Instruction::LocalGet(4));
            self.function.instruction(&Instruction::I32Const(index));
            self.function.instruction(&Instruction::I32Add);
            self.get_slot(*argument, span)?;
            self.call_runtime("__exs_rt_async_frame_set_slot", span)?;
            self.function.instruction(&Instruction::Else);
            self.get_slot(destination, span)?;
            self.get_slot(*argument, span)?;
            self.call_runtime("__exs_rt_append", span)?;
            self.function.instruction(&Instruction::Drop);
            self.function.instruction(&Instruction::End);
        }
        self.function.instruction(&Instruction::LocalGet(6));
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.function.instruction(&Instruction::LocalGet(2));
        self.function.instruction(&Instruction::LocalGet(4));
        self.function.instruction(&Instruction::LocalGet(5));
        self.function.instruction(&Instruction::I32Add);
        self.get_slot(destination, span)?;
        self.call_runtime("__exs_rt_async_frame_set_slot", span)?;
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::LocalGet(2));
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::I32Const(destination.cast_signed()));
        self.call_runtime("__exs_rt_async_frame_set_caller", span)?;
        self.call_runtime("__exs_rt_scheduler_checkpoint", span)?;
        self.function
            .instruction(&Instruction::I32Const(STATUS_READY));
        self.function.instruction(&Instruction::Return);
        self.function.instruction(&Instruction::Else);
        self.get_slot(closure, span)?;
        self.call_runtime("__exs_rt_not_callable_error", span)?;
        self.set_slot(destination, span)?;
        self.ready(next, span)?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }
    /// Starts every static parallel closure task and yields execution to the scheduler.
    fn parallel_start(
        &mut self,
        next: u32,
        tasks: &[u32],
        destination: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let count = i32::try_from(tasks.len()).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                span,
                "too many parallel tasks",
            ))
        })?;
        self.function.instruction(&Instruction::I32Const(count));
        self.call_runtime("__exs_rt_parallel_new", span)?;
        self.set_slot(destination, span)?;
        for (index, task) in tasks.iter().enumerate() {
            let index = i32::try_from(index).map_err(|_| {
                diagnostics(CompileDiagnostic::new(
                    "E0212",
                    span,
                    "too many parallel tasks",
                ))
            })?;
            self.get_slot(*task, span)?;
            self.call_runtime("__exs_rt_closure_function", span)?;
            self.function.instruction(&Instruction::LocalSet(3));
            self.get_slot(*task, span)?;
            self.call_runtime("__exs_rt_closure_slot_count", span)?;
            self.function.instruction(&Instruction::LocalSet(4));
            self.get_slot(destination, span)?;
            self.function.instruction(&Instruction::I32Const(index));
            self.function.instruction(&Instruction::LocalGet(3));
            self.function.instruction(&Instruction::LocalGet(4));
            self.call_runtime("__exs_rt_async_frame_new_parallel", span)?;
            self.function.instruction(&Instruction::LocalSet(2));
            self.get_slot(*task, span)?;
            self.call_runtime("__exs_rt_closure_capture_count", span)?;
            self.function.instruction(&Instruction::LocalSet(4));
            self.function.instruction(&Instruction::I32Const(0));
            self.function.instruction(&Instruction::LocalSet(5));
            self.function
                .instruction(&Instruction::Block(BlockType::Empty));
            self.function
                .instruction(&Instruction::Loop(BlockType::Empty));
            self.function.instruction(&Instruction::LocalGet(5));
            self.function.instruction(&Instruction::LocalGet(4));
            self.function.instruction(&Instruction::I32GeU);
            self.function.instruction(&Instruction::BrIf(1));
            self.function.instruction(&Instruction::LocalGet(2));
            self.function.instruction(&Instruction::LocalGet(5));
            self.get_slot(*task, span)?;
            self.function.instruction(&Instruction::LocalGet(5));
            self.call_runtime("__exs_rt_closure_capture", span)?;
            self.call_runtime("__exs_rt_async_frame_set_slot", span)?;
            self.function.instruction(&Instruction::LocalGet(5));
            self.function.instruction(&Instruction::I32Const(1));
            self.function.instruction(&Instruction::I32Add);
            self.function.instruction(&Instruction::LocalSet(5));
            self.function.instruction(&Instruction::Br(0));
            self.function.instruction(&Instruction::End);
            self.function.instruction(&Instruction::End);
            self.get_slot(*task, span)?;
            self.call_runtime("__exs_rt_closure_is_variadic", span)?;
            self.function
                .instruction(&Instruction::If(BlockType::Empty));
            self.get_slot(*task, span)?;
            self.call_runtime("__exs_rt_closure_arity", span)?;
            self.function.instruction(&Instruction::LocalSet(5));
            self.call_runtime("__exs_rt_list_new", span)?;
            self.function.instruction(&Instruction::LocalSet(6));
            self.function.instruction(&Instruction::LocalGet(2));
            self.function.instruction(&Instruction::LocalGet(4));
            self.function.instruction(&Instruction::LocalGet(5));
            self.function.instruction(&Instruction::I32Add);
            self.function.instruction(&Instruction::LocalGet(6));
            self.call_runtime("__exs_rt_async_frame_set_slot", span)?;
            self.function.instruction(&Instruction::End);
        }
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::I32Const(next.cast_signed()));
        self.call_runtime("__exs_rt_async_frame_set_state", span)?;
        self.get_slot(destination, span)?;
        self.call_runtime("__exs_rt_parallel_wait", span)?;
        self.function.instruction(&Instruction::Drop);
        self.function
            .instruction(&Instruction::I32Const(STATUS_READY));
        self.function.instruction(&Instruction::Return);
        Ok(())
    }

    /// Starts every zero-argument closure held in one runtime List and yields to the scheduler.
    fn parallel_dynamic_start(
        &mut self,
        next: u32,
        functions: u32,
        destination: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.get_slot(functions, span)?;
        self.call_runtime("__exs_rt_is_list", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.get_slot(functions, span)?;
        self.call_runtime("__exs_rt_parallel_list_count", span)?;
        self.function.instruction(&Instruction::LocalSet(7));
        self.function.instruction(&Instruction::I32Const(0));
        self.function.instruction(&Instruction::LocalSet(1));
        self.function
            .instruction(&Instruction::Block(BlockType::Empty));
        self.function
            .instruction(&Instruction::Loop(BlockType::Empty));
        self.function.instruction(&Instruction::LocalGet(1));
        self.function.instruction(&Instruction::LocalGet(7));
        self.function.instruction(&Instruction::I32GeU);
        self.function.instruction(&Instruction::BrIf(1));
        self.get_slot(functions, span)?;
        self.function.instruction(&Instruction::LocalGet(1));
        self.call_runtime("__exs_rt_parallel_list_get", span)?;
        self.function.instruction(&Instruction::LocalSet(6));
        self.function.instruction(&Instruction::LocalGet(6));
        self.call_runtime("__exs_rt_is_closure", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.function.instruction(&Instruction::Else);
        self.function.instruction(&Instruction::LocalGet(6));
        self.call_runtime("__exs_rt_not_callable_error", span)?;
        self.set_slot(destination, span)?;
        self.complete(destination, span)?;
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::LocalGet(6));
        self.call_runtime("__exs_rt_closure_arity", span)?;
        self.function.instruction(&Instruction::LocalSet(5));
        self.function.instruction(&Instruction::LocalGet(5));
        self.function.instruction(&Instruction::I32Const(0));
        self.function.instruction(&Instruction::I32Ne);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.function.instruction(&Instruction::LocalGet(6));
        self.call_runtime("__exs_rt_closure_is_variadic", span)?;
        self.function.instruction(&Instruction::I32Eqz);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.call_runtime("__exs_rt_closure_arity_error", span)?;
        self.set_slot(destination, span)?;
        self.complete(destination, span)?;
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::LocalGet(1));
        self.function.instruction(&Instruction::I32Const(1));
        self.function.instruction(&Instruction::I32Add);
        self.function.instruction(&Instruction::LocalSet(1));
        self.function.instruction(&Instruction::Br(0));
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::LocalGet(7));
        self.call_runtime("__exs_rt_parallel_new", span)?;
        self.set_slot(destination, span)?;
        self.function.instruction(&Instruction::I32Const(0));
        self.function.instruction(&Instruction::LocalSet(1));
        self.function
            .instruction(&Instruction::Block(BlockType::Empty));
        self.function
            .instruction(&Instruction::Loop(BlockType::Empty));
        self.function.instruction(&Instruction::LocalGet(1));
        self.function.instruction(&Instruction::LocalGet(7));
        self.function.instruction(&Instruction::I32GeU);
        self.function.instruction(&Instruction::BrIf(1));
        self.get_slot(functions, span)?;
        self.function.instruction(&Instruction::LocalGet(1));
        self.call_runtime("__exs_rt_parallel_list_get", span)?;
        self.function.instruction(&Instruction::LocalSet(6));
        self.function.instruction(&Instruction::LocalGet(6));
        self.call_runtime("__exs_rt_closure_function", span)?;
        self.function.instruction(&Instruction::LocalSet(3));
        self.function.instruction(&Instruction::LocalGet(6));
        self.call_runtime("__exs_rt_closure_slot_count", span)?;
        self.function.instruction(&Instruction::LocalSet(4));
        self.get_slot(destination, span)?;
        self.function.instruction(&Instruction::LocalGet(1));
        self.function.instruction(&Instruction::LocalGet(3));
        self.function.instruction(&Instruction::LocalGet(4));
        self.call_runtime("__exs_rt_async_frame_new_parallel", span)?;
        self.function.instruction(&Instruction::LocalSet(2));
        self.function.instruction(&Instruction::LocalGet(6));
        self.call_runtime("__exs_rt_closure_capture_count", span)?;
        self.function.instruction(&Instruction::LocalSet(4));
        self.function.instruction(&Instruction::I32Const(0));
        self.function.instruction(&Instruction::LocalSet(5));
        self.function
            .instruction(&Instruction::Block(BlockType::Empty));
        self.function
            .instruction(&Instruction::Loop(BlockType::Empty));
        self.function.instruction(&Instruction::LocalGet(5));
        self.function.instruction(&Instruction::LocalGet(4));
        self.function.instruction(&Instruction::I32GeU);
        self.function.instruction(&Instruction::BrIf(1));
        self.function.instruction(&Instruction::LocalGet(2));
        self.function.instruction(&Instruction::LocalGet(5));
        self.function.instruction(&Instruction::LocalGet(6));
        self.function.instruction(&Instruction::LocalGet(5));
        self.call_runtime("__exs_rt_closure_capture", span)?;
        self.call_runtime("__exs_rt_async_frame_set_slot", span)?;
        self.function.instruction(&Instruction::LocalGet(5));
        self.function.instruction(&Instruction::I32Const(1));
        self.function.instruction(&Instruction::I32Add);
        self.function.instruction(&Instruction::LocalSet(5));
        self.function.instruction(&Instruction::Br(0));
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::LocalGet(6));
        self.call_runtime("__exs_rt_closure_is_variadic", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.function.instruction(&Instruction::LocalGet(6));
        self.call_runtime("__exs_rt_closure_arity", span)?;
        self.function.instruction(&Instruction::LocalSet(5));
        self.call_runtime("__exs_rt_list_new", span)?;
        self.function.instruction(&Instruction::LocalSet(3));
        self.function.instruction(&Instruction::LocalGet(2));
        self.function.instruction(&Instruction::LocalGet(4));
        self.function.instruction(&Instruction::LocalGet(5));
        self.function.instruction(&Instruction::I32Add);
        self.function.instruction(&Instruction::LocalGet(3));
        self.call_runtime("__exs_rt_async_frame_set_slot", span)?;
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::LocalGet(1));
        self.function.instruction(&Instruction::I32Const(1));
        self.function.instruction(&Instruction::I32Add);
        self.function.instruction(&Instruction::LocalSet(1));
        self.function.instruction(&Instruction::Br(0));
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::I32Const(next.cast_signed()));
        self.call_runtime("__exs_rt_async_frame_set_state", span)?;
        self.get_slot(destination, span)?;
        self.call_runtime("__exs_rt_parallel_wait", span)?;
        self.function.instruction(&Instruction::Return);
        self.function.instruction(&Instruction::Else);
        self.get_slot(functions, span)?;
        self.call_runtime("__exs_rt_parallel_list_error", span)?;
        self.set_slot(destination, span)?;
        self.complete(destination, span)?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Converts a raw runner time object into its compiler-owned nominal prelude representation.
    fn host_time(
        &mut self,
        next: u32,
        value: u32,
        type_id: u32,
        fields: &[HostTimeField],
        destination: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.get_slot(value, span)?;
        self.call_runtime("__exs_rt_is_error", span)?;
        self.call_runtime("__exs_rt_condition", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.get_slot(value, span)?;
        self.set_slot(destination, span)?;
        self.ready(next, span)?;
        self.function.instruction(&Instruction::Else);
        self.get_slot(value, span)?;
        self.function
            .instruction(&Instruction::I32Const(TYPE_OBJECT.cast_signed()));
        self.call_runtime("__exs_rt_type_matches", span)?;
        self.function.instruction(&Instruction::I32Eqz);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.get_slot(value, span)?;
        self.function.instruction(&Instruction::I32Const(1));
        self.call_runtime("__exs_rt_type_mismatch", span)?;
        self.set_slot(destination, span)?;
        self.ready(next, span)?;
        self.function.instruction(&Instruction::Else);
        self.function
            .instruction(&Instruction::I32Const(type_id.cast_signed()));
        self.call_runtime("__exs_rt_object_typed_new", span)?;
        self.set_slot(destination, span)?;
        self.host_time_fields(next, value, fields, destination, span, 0)?;
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Copies and validates runner time fields until one mismatch or the nominal object is complete.
    fn host_time_fields(
        &mut self,
        next: u32,
        value: u32,
        fields: &[HostTimeField],
        destination: u32,
        span: SourceSpan<'source>,
        index: usize,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let Some(field) = fields.get(index) else {
            return self.ready(next, span);
        };
        self.get_slot(value, span)?;
        self.string(&field.name, span)?;
        self.call_runtime("__exs_rt_index_get", span)?;
        self.set_slot(field.slot, span)?;
        self.validate_slot_matches(field.slot, &field.contract, span)?;
        self.function.instruction(&Instruction::LocalGet(2));
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.get_slot(destination, span)?;
        self.string(&field.name, span)?;
        self.get_slot(field.slot, span)?;
        self.call_runtime("__exs_rt_index_set", span)?;
        self.function.instruction(&Instruction::Drop);
        self.host_time_fields(next, value, fields, destination, span, index + 1)?;
        self.function.instruction(&Instruction::Else);
        self.get_slot(field.slot, span)?;
        self.function.instruction(&Instruction::I32Const(1));
        self.call_runtime("__exs_rt_type_mismatch", span)?;
        self.set_slot(destination, span)?;
        self.ready(next, span)?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }
}
