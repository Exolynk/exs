//! Wasm state dispatch and primary operation emission for continuation frames.

use std::collections::HashMap;

use exs_abi::{
    ASYNC_FRAME_STATE_OFFSET, HOST_CALL_PENDING, HOST_CALL_READY, STATUS_READY, TYPE_ERROR,
    TYPE_OBJECT,
};
use exs_value::ValueRef;
use wasm_encoder::{BlockType, Instruction, MemArg, ValType};

use crate::ast::UnaryOperator;
use crate::codegen::literals::LiteralPool;
use crate::codegen::source_map::SourceMap;
use crate::codegen::types::TypeContract;
use crate::codegen::{CompileDiagnostic, CompileDiagnostics, SourceSpan};
use crate::codegen::{diagnostics, immediate, registers};

use super::FrameLayout;
use super::body::StepBody;
use super::graph::{FormatPart, HostTimeField, Operation, operation_span};
use super::step_calls::{binary_operation, slot_memory};

pub(super) struct StepCompiler<'source, 'context> {
    /// Runtime ABI export indexes.
    pub(super) runtime: &'context HashMap<String, u32>,
    /// Passive data indexes for compiler string literals.
    pub(super) literals: &'context LiteralPool,
    /// Compiler-assigned source positions.
    pub(super) source_map: &'context SourceMap<'source>,
    /// Durable child-frame layouts for nominal suspendable method targets.
    pub(super) frame_layouts: &'context HashMap<String, FrameLayout>,
    /// Linear-memory address of the runtime register block.
    pub(super) registers_address: u32,
    /// The declared return contract checked before a resumable frame completes.
    pub(super) return_contract: &'context TypeContract,
    /// Wasm function body under construction.
    pub(super) function: StepBody,
    /// Reused Wasm i64 local holding one temporary ValueRef result.
    pub(super) scratch_local: u32,
    /// Second reused Wasm i64 local for ValueRefs held while the scratch local is busy.
    pub(super) value_local: u32,
    /// Reused Wasm i64 local holding the right operand of an inline Int operation.
    pub(super) operand_local: u32,
    /// Reused Wasm i64 local holding the unboxed result of an inline Int operation.
    pub(super) int_result_local: u32,
    /// Reused Wasm i32 local holding the async-frame completion code.
    pub(super) completion_local: u32,
    /// Reused Wasm local for compiler literal-buffer pointers.
    pub(super) literal_buffer_local: u32,
    /// Reused Wasm local holding one variadic List length during boundary validation.
    pub(super) variadic_length_local: u32,
    /// Reused Wasm local holding one variadic List index during boundary validation.
    pub(super) variadic_index_local: u32,
    /// Wasm i32 local holding this frame's linear-memory header and slot block address.
    pub(super) block_local: u32,
    /// Wasm i32 local holding a newly created child frame's block address.
    pub(super) child_block_local: u32,
    /// Number of durable slots addressed by the emitted body.
    pub(super) used_slots: u32,
    /// Wasm local holding the state selected by the next dispatch-loop iteration.
    pub(super) state_local: u32,
    /// Control depth directly inside the state dispatch loop.
    pub(super) loop_depth: u32,
    /// Number of continuation states in this function.
    pub(super) state_count: u32,
    /// The state whose operation body is currently being emitted.
    pub(super) current_state: u32,
}

impl<'source, 'context> StepCompiler<'source, 'context> {
    /// Emits the state dispatch loop holding every graph operation.
    ///
    /// The body is `loop { block_{n-1} .. block_0 { br_table } state_0 end state_1 .. }`, so a
    /// transition to a later state branches straight to its block end and an earlier state
    /// re-enters the loop. The step function only returns to the driver at suspension points.
    pub(super) fn emit_dispatch(
        &mut self,
        operations: &[Operation<'source, '_>],
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let state_count = u32::try_from(operations.len()).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                span,
                "too many continuation states for one function",
            ))
        })?;
        if state_count == 0 {
            self.function.instruction(&Instruction::Unreachable);
            return Ok(());
        }
        self.state_count = state_count;
        // The dispatcher usually runs the cached active frame, whose block address the
        // registers already hold.
        self.function.instruction(&Instruction::I32Const(0));
        self.function
            .instruction(&registers::load_active_frame(self.registers_address));
        self.function.instruction(&Instruction::LocalGet(0));
        self.function.instruction(&Instruction::I32Eq);
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        self.function.instruction(&Instruction::I32Const(0));
        self.function
            .instruction(&registers::load_active_block(self.registers_address));
        self.function.instruction(&Instruction::Else);
        self.function.instruction(&Instruction::LocalGet(0));
        self.call_runtime("__exs_rt_async_frame_block", span)?;
        self.function.instruction(&Instruction::End);
        self.function
            .instruction(&Instruction::LocalTee(self.block_local));
        self.function.instruction(&Instruction::I32Load(MemArg {
            offset: u64::from(ASYNC_FRAME_STATE_OFFSET),
            align: 2,
            memory_index: 0,
        }));
        self.function
            .instruction(&Instruction::LocalSet(self.state_local));
        self.function
            .instruction(&Instruction::Loop(BlockType::Empty));
        self.loop_depth = self.function.depth();
        for _ in 0..state_count {
            self.function
                .instruction(&Instruction::Block(BlockType::Empty));
        }
        // The innermost block catches invalid states; label `index + 1` selects state `index`.
        self.function
            .instruction(&Instruction::Block(BlockType::Empty));
        self.function
            .instruction(&Instruction::LocalGet(self.state_local));
        let targets = (1..=state_count).collect::<Vec<_>>();
        self.function
            .instruction(&Instruction::BrTable(targets.into(), 0));
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::Unreachable);
        for (state, operation) in (0..state_count).zip(operations) {
            self.function.instruction(&Instruction::End);
            self.current_state = state;
            self.emit_operation(state, operation)?;
            self.function.instruction(&Instruction::Unreachable);
        }
        self.function.instruction(&Instruction::End);
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
            Operation::RuntimeStatic {
                values,
                export,
                destination,
                span,
            } => {
                for value in values {
                    self.get_slot(*value, *span)?;
                }
                self.call_runtime(export, *span)?;
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
                immediate::push(self.function.raw(), ValueRef::NONE);
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Boolean {
                value,
                destination,
                span,
            } => {
                immediate::push(self.function.raw(), ValueRef::boolean(*value));
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
            Operation::WireDecode {
                source,
                destination,
                schema,
                span,
            } => {
                self.get_slot(*source, *span)?;
                self.string(schema, *span)?;
                self.call_runtime("__exs_rt_decode_wire", *span)?;
                self.set_slot(*destination, *span)?;
                self.complete_if_error(*destination, *span)?;
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
                if let Some(operation) = immediate::IntOperation::from_binary(*operator) {
                    // The fast path leaves both operands in its locals when it falls through.
                    self.int_fast_path(operation, *left, *right, *destination, next, *span)?;
                    self.function
                        .instruction(&Instruction::LocalGet(self.value_local));
                    self.function
                        .instruction(&Instruction::LocalGet(self.operand_local));
                } else {
                    self.get_slot(*left, *span)?;
                    self.get_slot(*right, *span)?;
                }
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
                comparison,
                span,
            } => self.operator_call(
                *operator,
                next,
                *left,
                *right,
                targets,
                *destination,
                *comparison,
                *span,
            )?,
            Operation::FormatNew {
                capacity,
                destination,
                span,
            } => {
                self.function.instruction(&Instruction::I32Const(*capacity));
                self.call_runtime("__exs_rt_format_new", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::FormatPush {
                builder,
                part,
                span,
            } => {
                self.get_slot(*builder, *span)?;
                match part {
                    FormatPart::Text(text) => self.string(text, *span)?,
                    FormatPart::Slot(slot) => self.get_slot(*slot, *span)?,
                }
                self.call_runtime("__exs_rt_format_push", *span)?;
                self.set_slot(*builder, *span)?;
                self.ready(next, *span)?;
            }
            Operation::FormatValue {
                builder,
                value,
                type_ids,
                skip,
                span,
            } => {
                if !type_ids.is_empty() {
                    for (index, type_id) in type_ids.iter().enumerate() {
                        self.get_slot(*value, *span)?;
                        self.emit_object_type_test(*type_id, *span)?;
                        if index > 0 {
                            self.function.instruction(&Instruction::I32Or);
                        }
                    }
                    self.function
                        .instruction(&Instruction::If(BlockType::Empty));
                    self.ready(next, *span)?;
                    self.function.instruction(&Instruction::End);
                }
                self.get_slot(*builder, *span)?;
                self.get_slot(*value, *span)?;
                self.call_runtime("__exs_rt_format_push_default", *span)?;
                self.set_slot(*builder, *span)?;
                self.ready(*skip, *span)?;
            }
            Operation::FormatFinish {
                builder,
                destination,
                span,
            } => {
                self.get_slot(*builder, *span)?;
                self.call_runtime("__exs_rt_format_finish", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
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
                    self.call_runtime("__exs_rt_property_set", *span)?;
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
                destination,
                span,
            } => {
                let field_count = i32::try_from(fields.len()).map_err(|_| {
                    diagnostics(CompileDiagnostic::new(
                        "E0212",
                        *span,
                        "too many enum payload fields",
                    ))
                })?;
                self.function
                    .instruction(&Instruction::I32Const(type_id.cast_signed()));
                self.string(type_identity, *span)?;
                self.string(variant, *span)?;
                self.function
                    .instruction(&Instruction::I32Const(field_count));
                self.call_runtime("__exs_rt_enum_new_literal", *span)?;
                self.set_slot(*destination, *span)?;
                for field in fields {
                    self.get_slot(*destination, *span)?;
                    self.get_slot(*field, *span)?;
                    self.call_runtime("__exs_rt_enum_push_field", *span)?;
                }
                self.ready(next, *span)?;
            }
            Operation::EnumMatches {
                value,
                type_identity,
                variant,
                destination,
                span,
            } => {
                self.get_slot(*value, *span)?;
                self.string(type_identity, *span)?;
                self.string(variant, *span)?;
                self.call_runtime("__exs_rt_enum_matches_literal", *span)?;
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
            Operation::IsType {
                value,
                contract,
                destination,
                span,
            } => {
                self.validate_slot_matches(*value, contract, *span)?;
                self.function.instruction(&Instruction::LocalGet(2));
                immediate::bool_from_condition(self.function.raw());
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
                immediate::type_mask_test(self.function.raw(), TYPE_ERROR);
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
                self.call_runtime("__exs_rt_property_get", *span)?;
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
                self.call_runtime("__exs_rt_property_set", *span)?;
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
                immediate::type_mask_test(self.function.raw(), TYPE_ERROR);
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
                fallback,
                destination,
                span,
            } => self.instance_call(
                next,
                *receiver,
                method,
                *method_span,
                arguments,
                targets,
                fallback.as_ref(),
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
                    self.checkpoint(*target, *span)?;
                }
                self.ready(*target, *span)?;
            }
            Operation::IterSnapshot {
                iterable,
                destination,
                unshared_range,
                span,
            } => {
                self.get_slot(*iterable, *span)?;
                if let Some(range_type_id) = unshared_range {
                    self.function
                        .instruction(&Instruction::I32Const(*range_type_id));
                    self.call_runtime("__exs_rt_iter_range_snapshot", *span)?;
                } else {
                    self.call_runtime("__exs_rt_iter_snapshot", *span)?;
                }
                self.set_slot(*destination, *span)?;
                self.complete_if_error(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::NativeIterNext {
                snapshot,
                item,
                range_type_id,
                when_item,
                when_done,
                span,
            } => {
                self.get_slot(*snapshot, *span)?;
                self.function
                    .instruction(&Instruction::I32Const(*range_type_id));
                self.call_runtime("__exs_rt_iter_native_next", *span)?;
                self.function
                    .instruction(&Instruction::LocalTee(self.value_local));
                self.function.instruction(&Instruction::I64Const(-1));
                self.function.instruction(&Instruction::I64Eq);
                self.function
                    .instruction(&Instruction::If(BlockType::Empty));
                self.ready(next, *span)?;
                self.function.instruction(&Instruction::End);
                self.function
                    .instruction(&Instruction::LocalGet(self.value_local));
                self.function.instruction(&Instruction::I64Eqz);
                self.function
                    .instruction(&Instruction::If(BlockType::Empty));
                self.ready(*when_done, *span)?;
                self.function.instruction(&Instruction::End);
                self.function
                    .instruction(&Instruction::LocalGet(self.value_local));
                self.set_slot(*item, *span)?;
                self.ready(*when_item, *span)?;
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
                self.bool_condition(*span)?;
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
                self.bool_condition(*span)?;
                self.function
                    .instruction(&Instruction::If(BlockType::Empty));
                self.get_slot(*step, *span)?;
                self.function.instruction(&Instruction::I32Const(0));
                self.call_runtime("__exs_rt_enum_field", *span)?;
                self.set_slot(*item, *span)?;
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
        self.store_state(next);
        self.set_call_site(span)?;
        // One runtime call allocates, traces and links the child frame, then runs the scheduler
        // checkpoint; only the arguments are stored afterwards.
        self.function
            .instruction(&Instruction::I32Const(layout.function_id.cast_signed()));
        self.function
            .instruction(&Instruction::I32Const(slot_count));
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::I32Const(destination.cast_signed()));
        self.call_runtime("__exs_rt_async_call", span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.child_block_local));
        for (slot, argument) in arguments.iter().enumerate() {
            let slot = u32::try_from(slot).map_err(|_| {
                diagnostics(CompileDiagnostic::new(
                    "E0212",
                    span,
                    "too many child function arguments",
                ))
            })?;
            self.function
                .instruction(&Instruction::LocalGet(self.child_block_local));
            self.get_slot(*argument, span)?;
            self.function
                .instruction(&Instruction::I64Store(slot_memory(slot)));
        }
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
        self.store_state(next);
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
        self.get_slot(closure, span)?;
        self.call_runtime("__exs_rt_closure_arity", span)?;
        self.function.instruction(&Instruction::LocalSet(5));
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
        self.function.instruction(&Instruction::Drop);
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
            self.function
                .instruction(&Instruction::LocalSet(self.scratch_local));
            self.function.instruction(&Instruction::LocalGet(2));
            self.function.instruction(&Instruction::LocalGet(4));
            self.function.instruction(&Instruction::LocalGet(5));
            self.function.instruction(&Instruction::I32Add);
            self.function
                .instruction(&Instruction::LocalGet(self.scratch_local));
            self.call_runtime("__exs_rt_async_frame_set_slot", span)?;
            self.function.instruction(&Instruction::End);
        }
        self.store_state(next);
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
        self.function
            .instruction(&Instruction::LocalSet(self.value_local));
        self.function
            .instruction(&Instruction::LocalGet(self.value_local));
        self.call_runtime("__exs_rt_is_closure", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.function.instruction(&Instruction::Else);
        self.function
            .instruction(&Instruction::LocalGet(self.value_local));
        self.call_runtime("__exs_rt_not_callable_error", span)?;
        self.set_slot(destination, span)?;
        self.complete(destination, span)?;
        self.function.instruction(&Instruction::End);
        self.function
            .instruction(&Instruction::LocalGet(self.value_local));
        self.call_runtime("__exs_rt_closure_arity", span)?;
        self.function.instruction(&Instruction::LocalSet(5));
        self.function.instruction(&Instruction::LocalGet(5));
        self.function.instruction(&Instruction::I32Const(0));
        self.function.instruction(&Instruction::I32Ne);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.function
            .instruction(&Instruction::LocalGet(self.value_local));
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
        self.function
            .instruction(&Instruction::LocalSet(self.value_local));
        self.function
            .instruction(&Instruction::LocalGet(self.value_local));
        self.call_runtime("__exs_rt_closure_function", span)?;
        self.function.instruction(&Instruction::LocalSet(3));
        self.function
            .instruction(&Instruction::LocalGet(self.value_local));
        self.call_runtime("__exs_rt_closure_slot_count", span)?;
        self.function.instruction(&Instruction::LocalSet(4));
        self.get_slot(destination, span)?;
        self.function.instruction(&Instruction::LocalGet(1));
        self.function.instruction(&Instruction::LocalGet(3));
        self.function.instruction(&Instruction::LocalGet(4));
        self.call_runtime("__exs_rt_async_frame_new_parallel", span)?;
        self.function.instruction(&Instruction::LocalSet(2));
        self.function
            .instruction(&Instruction::LocalGet(self.value_local));
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
        self.function
            .instruction(&Instruction::LocalGet(self.value_local));
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
        self.function
            .instruction(&Instruction::LocalGet(self.value_local));
        self.call_runtime("__exs_rt_closure_is_variadic", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.function
            .instruction(&Instruction::LocalGet(self.value_local));
        self.call_runtime("__exs_rt_closure_arity", span)?;
        self.function.instruction(&Instruction::LocalSet(5));
        self.call_runtime("__exs_rt_list_new", span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.scratch_local));
        self.function.instruction(&Instruction::LocalGet(2));
        self.function.instruction(&Instruction::LocalGet(4));
        self.function.instruction(&Instruction::LocalGet(5));
        self.function.instruction(&Instruction::I32Add);
        self.function
            .instruction(&Instruction::LocalGet(self.scratch_local));
        self.call_runtime("__exs_rt_async_frame_set_slot", span)?;
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::LocalGet(1));
        self.function.instruction(&Instruction::I32Const(1));
        self.function.instruction(&Instruction::I32Add);
        self.function.instruction(&Instruction::LocalSet(1));
        self.function.instruction(&Instruction::Br(0));
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::End);
        self.store_state(next);
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
        immediate::type_mask_test(self.function.raw(), TYPE_ERROR);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.get_slot(value, span)?;
        self.set_slot(destination, span)?;
        self.ready(next, span)?;
        self.function.instruction(&Instruction::Else);
        self.get_slot(value, span)?;
        immediate::type_mask_test(self.function.raw(), TYPE_OBJECT);
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
