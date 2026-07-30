//! Runtime ABI calls, rooted local values, and structured-control bookkeeping.

use wasm_encoder::Instruction;

use crate::codegen::diagnostics;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

use super::FunctionCompiler;

impl<'a, 'module> FunctionCompiler<'a, 'module> {
    /// Emits one named runtime ABI call after resolving its template function index.
    pub(super) fn runtime_call(
        &mut self,
        name: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if name != "__exs_rt_set_source_position" {
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
        self.function
            .instruction(&Instruction::I32Const(position.cast_signed()));
        self.runtime_call_unpositioned("__exs_rt_set_call_site", span)
    }

    /// Emits one runtime ABI call without updating the active source position.
    pub(super) fn runtime_call_unpositioned(
        &mut self,
        name: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let index = self.runtime.get(name).copied().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0209",
                span,
                format!("runtime template does not export `{name}`"),
            ))
        })?;
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

    /// Creates this invocation's root frame and registers its parameters.
    pub(super) fn initialize_root_frame(
        &mut self,
        root_slot_count: u32,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let slot_count = i32::try_from(root_slot_count).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                self.declaration.span,
                "too many root slots for the Wasm i32 ABI",
            ))
        })?;
        self.function
            .instruction(&Instruction::I32Const(slot_count));
        self.runtime_call("__exs_rt_root_push", self.declaration.span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.root_frame_local));
        self.function
            .instruction(&Instruction::I32Const(self.function_id.cast_signed()));
        self.runtime_call("__exs_rt_frame_push", self.declaration.span)?;
        let signature = self
            .signatures
            .get(&self.declaration.name.name)
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0999",
                    self.declaration.name.span,
                    "missing function signature during parameter validation",
                ))
            })?;
        for (parameter, types) in signature.parameter_types.iter().copied().enumerate() {
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

    /// Registers one compiler local in the active root frame.
    pub(super) fn set_root_slot(&mut self, local: u32) -> Result<(), CompileDiagnostics<'a>> {
        let slot = i32::try_from(local).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                self.declaration.span,
                "root slot exceeds the Wasm i32 ABI",
            ))
        })?;
        self.function
            .instruction(&Instruction::LocalGet(self.root_frame_local));
        self.function.instruction(&Instruction::I32Const(slot));
        self.function.instruction(&Instruction::LocalGet(local));
        self.runtime_call("__exs_rt_root_set", self.declaration.span)
    }

    /// Removes one compiler local from the active root frame.
    pub(super) fn clear_root_slot(&mut self, local: u32) -> Result<(), CompileDiagnostics<'a>> {
        let slot = i32::try_from(local).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                self.declaration.span,
                "root slot exceeds the Wasm i32 ABI",
            ))
        })?;
        self.function
            .instruction(&Instruction::LocalGet(self.root_frame_local));
        self.function.instruction(&Instruction::I32Const(slot));
        self.runtime_call("__exs_rt_root_clear", self.declaration.span)
    }

    /// Returns the ValueRef on the stack after safely removing this function's root frame.
    pub(super) fn return_stack_value(&mut self) -> Result<(), CompileDiagnostics<'a>> {
        let result = self.store_stack_value()?;
        self.function
            .instruction(&Instruction::LocalGet(self.root_frame_local));
        self.runtime_call("__exs_rt_root_pop", self.declaration.span)?;
        self.runtime_call("__exs_rt_frame_pop", self.declaration.span)?;
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
