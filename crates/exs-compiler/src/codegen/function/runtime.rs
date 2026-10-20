//! Runtime ABI calls, rooted local values, and structured-control bookkeeping.

use exs_value::ValueRef;
use wasm_encoder::{Instruction, MemArg};

use crate::codegen::diagnostics;
use crate::codegen::function::FunctionSignature;
use crate::codegen::immediate;
use crate::codegen::source_map::requires_source_position;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

use super::{FunctionCompiler, SCALAR_LOCAL_COUNT};
use crate::codegen::root_slot_memory_arg;

impl<'a, 'module> FunctionCompiler<'a, 'module> {
    /// Emits fixed call arguments followed by one packed List for a variadic tail.
    pub(super) fn emit_call_arguments(
        &mut self,
        arguments: &[u32],
        signature: &FunctionSignature,
        span: SourceSpan<'a>,
    ) -> Result<Option<u32>, CompileDiagnostics<'a>> {
        for argument in arguments.iter().take(signature.arity) {
            self.function.instruction(&Instruction::LocalGet(*argument));
        }
        self.emit_variadic_tail(arguments, signature.arity, signature.variadic, span)
    }

    /// Packs a source argument suffix into one rooted List when a call target is variadic.
    pub(super) fn emit_variadic_tail(
        &mut self,
        arguments: &[u32],
        fixed_arity: usize,
        variadic: bool,
        span: SourceSpan<'a>,
    ) -> Result<Option<u32>, CompileDiagnostics<'a>> {
        if !variadic {
            return Ok(None);
        }
        self.runtime_call("__exs_rt_list_new", span)?;
        let list = self.store_stack_value()?;
        for argument in arguments.iter().skip(fixed_arity) {
            self.function.instruction(&Instruction::LocalGet(list));
            self.function.instruction(&Instruction::LocalGet(*argument));
            self.runtime_call("__exs_rt_append", span)?;
            self.function.instruction(&Instruction::Drop);
        }
        self.function.instruction(&Instruction::LocalGet(list));
        Ok(Some(list))
    }

    /// Emits one named runtime ABI call after resolving its template function index.
    pub(super) fn runtime_call(
        &mut self,
        name: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if requires_source_position(name) {
            self.set_runtime_source_position(span)?;
        }
        self.runtime_call_unpositioned(name, span)
    }

    /// Emits the runtime source position used by a subsequent fallible ABI operation.
    pub(super) fn set_runtime_source_position(
        &mut self,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let position = self.source_map.id(span).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0214",
                span,
                "missing source-map position for generated runtime call",
            ))
        })?;
        self.function
            .instruction(&Instruction::I32Const(position.cast_signed()));
        self.runtime_call_unpositioned("__exs_rt_set_source_position", span)
    }

    /// Emits the source call site consumed by the next generated function entry.
    pub(super) fn set_runtime_call_site(
        &mut self,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let position = self.source_map.id(span).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0214",
                span,
                "missing source-map position for generated function call",
            ))
        })?;
        crate::codegen::registers::store_call_site(
            &mut self.function,
            self.registers_address,
            position,
        );
        Ok(())
    }

    /// Pushes the immediate None value.
    pub(super) fn emit_none(&mut self) {
        immediate::push(&mut self.function, ValueRef::NONE);
    }

    /// Replaces the ValueRef on the stack with the i32 result of one nominal Object type test.
    pub(super) fn emit_object_type_test(
        &mut self,
        type_id: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if immediate::object_type_test(&mut self.function, type_id) {
            return Ok(());
        }
        self.function
            .instruction(&Instruction::I32Const(type_id.cast_signed()));
        self.runtime_call("__exs_rt_object_is_type", span)
    }

    /// Returns the runtime exports used by inline function entry and exit.
    fn frame_exports(
        &self,
    ) -> Result<crate::codegen::registers::FrameExports, CompileDiagnostics<'a>> {
        let span = self.declaration.span;
        Ok(crate::codegen::registers::FrameExports {
            root_push: self.runtime_function("__exs_rt_root_push", span)?,
            root_pop: self.runtime_function("__exs_rt_root_pop", span)?,
            trace_grow: self.runtime_function("__exs_rt_trace_grow", span)?,
        })
    }

    /// Returns the function index of one runtime ABI export.
    pub(super) fn runtime_function(
        &self,
        name: &str,
        span: SourceSpan<'a>,
    ) -> Result<u32, CompileDiagnostics<'a>> {
        self.runtime.get(name).copied().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0209",
                span,
                format!("runtime template does not export `{name}`"),
            ))
        })
    }

    /// Emits one runtime ABI call without updating the active source position.
    pub(super) fn runtime_call_unpositioned(
        &mut self,
        name: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let index = self.runtime_function(name, span)?;
        self.function.instruction(&Instruction::Call(index));
        Ok(())
    }

    /// Calls a runtime operation after spilling every ValueRef argument into rooted locals.
    pub(super) fn runtime_value_call(
        &mut self,
        name: &str,
        argument_count: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let mut arguments = Vec::new();
        for _ in 0..argument_count {
            arguments.push(self.store_stack_value()?);
        }
        arguments.reverse();
        for argument in &arguments {
            self.function.instruction(&Instruction::LocalGet(*argument));
        }
        self.runtime_call(name, span)?;
        for argument in arguments {
            self.clear_root_slot(argument)?;
        }
        Ok(())
    }

    /// Increases the tracked structured-control nesting depth.
    pub(super) fn enter_control(&mut self) -> Result<(), CompileDiagnostics<'a>> {
        self.control_depth = self.control_depth.checked_add(1).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                self.declaration.span,
                "too many nested control-flow blocks in one function",
            ))
        })?;
        Ok(())
    }

    /// Decreases the tracked structured-control nesting depth.
    pub(super) fn exit_control(&mut self) -> Result<(), CompileDiagnostics<'a>> {
        self.control_depth = self.control_depth.checked_sub(1).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0999",
                self.declaration.span,
                "unbalanced compiler control-flow bookkeeping",
            ))
        })?;
        Ok(())
    }

    /// Branches to one active structured-control depth.
    pub(super) fn branch_to(
        &mut self,
        target_depth: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let depth = self
            .control_depth
            .checked_sub(target_depth)
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0999",
                    span,
                    "invalid compiler loop branch target",
                ))
            })?;
        self.function.instruction(&Instruction::Br(depth));
        Ok(())
    }

    /// Conditionally branches to one active structured-control depth.
    pub(super) fn branch_if_to(
        &mut self,
        target_depth: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let depth = self
            .control_depth
            .checked_sub(target_depth)
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0999",
                    span,
                    "invalid compiler loop branch target",
                ))
            })?;
        self.function.instruction(&Instruction::BrIf(depth));
        Ok(())
    }

    /// Clears roots introduced in lexical scopes at or below one scope-stack position.
    pub(super) fn clear_roots_from_scope(
        &mut self,
        scope_start: usize,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let locals = self
            .scopes
            .iter()
            .skip(scope_start)
            .flat_map(|scope| scope.values().copied())
            .collect::<Vec<_>>();
        for local in locals {
            self.clear_root_slot(local)?;
        }
        Ok(())
    }

    /// Returns the inline entry pushing this function's root frame of `slot_count` slots and
    /// its language stack frame; the runtime only runs when a frame needs a new chunk.
    pub(super) fn function_entry(
        &self,
        slot_count: u32,
    ) -> Result<Vec<Instruction<'static>>, CompileDiagnostics<'a>> {
        if i32::try_from(slot_count).is_err() {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0212",
                self.declaration.span,
                "too many root slots for the Wasm i32 ABI",
            )));
        }
        Ok(crate::codegen::registers::function_enter(
            self.registers_address,
            self.frame_exports()?,
            self.function_id,
            slot_count,
            self.root_frame_local,
            self.trace_entry_local,
        ))
    }

    /// Roots every parameter and validates it against its declared contract.
    pub(super) fn store_parameters(&mut self) -> Result<(), CompileDiagnostics<'a>> {
        let signature = self.signatures.get(&self.signature_key).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0999",
                self.declaration.name.span,
                "missing function signature during parameter validation",
            ))
        })?;
        for (parameter, types) in signature
            .parameter_types
            .iter()
            .take(signature.arity)
            .enumerate()
        {
            let parameter = u32::try_from(parameter).map_err(|_| {
                diagnostics(CompileDiagnostic::new(
                    "E0212",
                    self.declaration.span,
                    "too many parameters for one function",
                ))
            })?;
            self.set_root_slot(parameter)?;
            self.validate_local_type(
                parameter,
                types,
                self.declaration.parameters[parameter as usize].name.span,
            )?;
        }
        if signature.variadic {
            let parameter = u32::try_from(signature.arity).map_err(|_| {
                diagnostics(CompileDiagnostic::new(
                    "E0212",
                    self.declaration.span,
                    "too many parameters for one function",
                ))
            })?;
            let contract = signature
                .parameter_types
                .get(signature.arity)
                .ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0999",
                        self.declaration.span,
                        "missing variadic parameter contract during lowering",
                    ))
                })?;
            self.set_root_slot(parameter)?;
            self.validate_variadic_parameter(
                parameter,
                contract,
                self.declaration.parameters[parameter as usize].name.span,
            )?;
        }
        Ok(())
    }

    /// Validates every value carried by one packed variadic parameter List.
    fn validate_variadic_parameter(
        &mut self,
        parameter: u32,
        contract: &crate::codegen::types::TypeContract,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let length = self.variadic_length_local;
        let index = self.variadic_index_local;
        self.function.instruction(&Instruction::LocalGet(parameter));
        self.runtime_call("__exs_rt_list_length", span)?;
        self.function.instruction(&Instruction::LocalSet(length));
        self.function.instruction(&Instruction::I32Const(0));
        self.function.instruction(&Instruction::LocalSet(index));
        self.function
            .instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
        self.function
            .instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
        self.function.instruction(&Instruction::LocalGet(index));
        self.function.instruction(&Instruction::LocalGet(length));
        self.function.instruction(&Instruction::I32GeU);
        self.function.instruction(&Instruction::BrIf(1));
        self.function.instruction(&Instruction::LocalGet(parameter));
        self.function.instruction(&Instruction::LocalGet(index));
        self.runtime_call("__exs_rt_list_get", span)?;
        let value = self.store_stack_value()?;
        self.validate_local_type(value, contract, span)?;
        self.clear_root_slot(value)?;
        self.function.instruction(&Instruction::LocalGet(index));
        self.function.instruction(&Instruction::I32Const(1));
        self.function.instruction(&Instruction::I32Add);
        self.function.instruction(&Instruction::LocalSet(index));
        self.function.instruction(&Instruction::Br(0));
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Stores the stack's ValueRef in a fresh compiler local and roots it.
    pub(super) fn store_stack_value(&mut self) -> Result<u32, CompileDiagnostics<'a>> {
        let local = self.allocate_local();
        self.store_stack_value_in(local)?;
        Ok(local)
    }

    /// Stores the stack's ValueRef in one compiler local and updates its root slot.
    pub(super) fn store_stack_value_in(
        &mut self,
        local: u32,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.function.instruction(&Instruction::LocalSet(local));
        self.set_root_slot(local)
    }

    /// Registers one compiler local in the active root frame with an inline shadow-stack store.
    pub(super) fn set_root_slot(&mut self, local: u32) -> Result<(), CompileDiagnostics<'a>> {
        let memory_arg = self.root_slot_memory_arg(local)?;
        self.function
            .instruction(&Instruction::LocalGet(self.root_frame_local));
        self.function.instruction(&Instruction::LocalGet(local));
        self.function
            .instruction(&Instruction::I64Store(memory_arg));
        Ok(())
    }

    /// Removes one compiler local from the active root frame by zeroing its shadow-stack slot.
    pub(super) fn clear_root_slot(&mut self, local: u32) -> Result<(), CompileDiagnostics<'a>> {
        let memory_arg = self.root_slot_memory_arg(local)?;
        self.function
            .instruction(&Instruction::LocalGet(self.root_frame_local));
        self.function.instruction(&Instruction::I64Const(0));
        self.function
            .instruction(&Instruction::I64Store(memory_arg));
        Ok(())
    }

    /// Returns the shadow-stack address operand of one compiler local's root slot.
    ///
    /// The frame local holds the slot base address, so every slot must lie inside the frame
    /// allocated by `__exs_rt_root_push`; slot indexes are therefore validated at compile time.
    fn root_slot_memory_arg(&self, local: u32) -> Result<MemArg, CompileDiagnostics<'a>> {
        // Parameters use the first slots; ValueRef temporaries follow, skipping the i32 scalars.
        let slot = if local < self.root_frame_local {
            local
        } else if local >= self.first_value_local {
            local - SCALAR_LOCAL_COUNT
        } else {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0212",
                self.declaration.span,
                "scalar local has no root slot",
            )));
        };
        Ok(root_slot_memory_arg(slot))
    }

    /// Returns the ValueRef on the stack after safely removing this function's root frame.
    pub(super) fn return_stack_value(&mut self) -> Result<(), CompileDiagnostics<'a>> {
        let result = self.store_stack_value()?;
        let exports = self.frame_exports()?;
        for instruction in crate::codegen::registers::function_leave(
            self.registers_address,
            exports,
            self.root_frame_local,
        ) {
            self.function.instruction(&instruction);
        }
        self.function.instruction(&Instruction::LocalGet(result));
        self.function.instruction(&Instruction::Return);
        Ok(())
    }

    /// Looks up one lexical binding's Wasm local index.
    pub(super) fn lookup(&self, name: &str) -> Option<u32> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    /// Reserves the next preallocated ValueRef local slot.
    pub(super) fn allocate_local(&mut self) -> u32 {
        let local = self.next_local;
        self.next_local += 1;
        local
    }
}
