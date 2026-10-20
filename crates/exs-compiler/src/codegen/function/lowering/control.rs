use super::*;
use crate::ast::TypeAnnotation;

impl<'a, 'module> FunctionCompiler<'a, 'module> {
    /// Compiles a non-throwing runtime test against one resolved type contract.
    pub(in crate::codegen::function) fn compile_is_type(
        &mut self,
        expression: &Expression<'a>,
        annotation: &TypeAnnotation<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(expression)?;
        let value = self.store_stack_value()?;
        let contract = self.types.resolve(Some(annotation), annotation.span)?;
        let matches = self.type_match_local;
        self.function.instruction(&Instruction::LocalGet(value));
        immediate::type_mask_test(&mut self.function, contract.builtin_mask);
        self.function.instruction(&Instruction::LocalSet(matches));
        for type_id in &contract.nominal_type_ids {
            self.function.instruction(&Instruction::LocalGet(matches));
            self.function.instruction(&Instruction::LocalGet(value));
            self.emit_object_type_test(*type_id, span)?;
            self.function.instruction(&Instruction::I32Or);
            self.function.instruction(&Instruction::LocalSet(matches));
        }
        for type_id in &contract.enum_type_ids {
            self.function.instruction(&Instruction::LocalGet(matches));
            self.function.instruction(&Instruction::LocalGet(value));
            self.compile_string(type_id, span)?;
            self.runtime_call("__exs_rt_enum_is_type", span)?;
            self.function.instruction(&Instruction::I32Or);
            self.function.instruction(&Instruction::LocalSet(matches));
        }
        self.function.instruction(&Instruction::LocalGet(matches));
        immediate::bool_from_condition(&mut self.function);
        self.clear_root_slot(value)
    }

    /// Returns the current function's recoverable or fatal generic type-contract Error.
    pub(in crate::codegen::function) fn return_type_error(
        &mut self,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.emit_none();
        let value = self.store_stack_value()?;
        self.function.instruction(&Instruction::LocalGet(value));
        self.function
            .instruction(&Instruction::I32Const(i32::from(types::permits_error(
                &self.return_type,
            ))));
        self.runtime_call("__exs_rt_type_mismatch", span)?;
        self.clear_root_slot(value)?;
        self.return_stack_value()
    }

    /// Compiles short-circuiting boolean conjunction or disjunction.
    pub(in crate::codegen::function) fn compile_logical(
        &mut self,
        left: &Expression<'a>,
        right: &Expression<'a>,
        is_or: bool,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(left)?;
        self.compile_condition(span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Result(VALUE_TYPE)));
        self.enter_control()?;
        if is_or {
            immediate::push(&mut self.function, ValueRef::boolean(true));
        } else {
            self.checked_boolean_expression(right)?;
        }
        self.function.instruction(&Instruction::Else);
        if is_or {
            self.checked_boolean_expression(right)?;
        } else {
            immediate::push(&mut self.function, ValueRef::boolean(false));
        }
        self.function.instruction(&Instruction::End);
        self.exit_control()
    }

    /// Lowers the postfix propagation operator for Option and Result values.
    pub(in crate::codegen::function) fn compile_propagate(
        &mut self,
        value: &Expression<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if !types::permits_error(&self.return_type) {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0218",
                span,
                "? requires the current function return type to include Error or Any",
            )));
        }
        self.compile_expression(value)?;
        // Only None needs the runtime, which turns it into a MissingValue Error; every other
        // value passes through unchanged, and Errors return below.
        let operand = self.store_stack_value()?;
        self.function.instruction(&Instruction::LocalGet(operand));
        immediate::push(&mut self.function, ValueRef::NONE);
        self.function.instruction(&Instruction::I64Eq);
        self.function
            .instruction(&Instruction::If(BlockType::Result(VALUE_TYPE)));
        self.enter_control()?;
        self.function.instruction(&Instruction::LocalGet(operand));
        self.runtime_call("__exs_rt_propagate", span)?;
        self.function.instruction(&Instruction::Else);
        self.function.instruction(&Instruction::LocalGet(operand));
        self.function.instruction(&Instruction::End);
        self.exit_control()?;
        self.clear_root_slot(operand)?;
        self.return_if_error(span)
    }

    /// Validates the ValueRef on the stack as a Boolean and lowers it to Wasm i32 control flow.
    pub(in crate::codegen::function) fn compile_condition(
        &mut self,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        // Every Bool is an immediate, so only non-Bool values take the runtime error path.
        let value = self.immediate_scratch_local;
        self.function.instruction(&Instruction::LocalTee(value));
        immediate::is_bool_test(&mut self.function);
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        self.enter_control()?;
        self.function.instruction(&Instruction::LocalGet(value));
        self.function.instruction(&Instruction::I32WrapI64);
        self.function.instruction(&Instruction::Else);
        self.function.instruction(&Instruction::LocalGet(value));
        self.runtime_value_call("__exs_rt_condition_value", 1, span)?;
        self.return_if_error(span)?;
        self.runtime_value_call("__exs_rt_condition", 1, span)?;
        self.function.instruction(&Instruction::End);
        self.exit_control()
    }

    /// Returns a language Error from the current function or leaves the non-Error value on stack.
    pub(in crate::codegen::function) fn return_if_error(
        &mut self,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let outcome = self.store_stack_value()?;
        self.function.instruction(&Instruction::LocalGet(outcome));
        immediate::type_mask_test(&mut self.function, exs_abi::TYPE_ERROR);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.enter_control()?;
        self.function.instruction(&Instruction::LocalGet(outcome));
        let return_type = self.return_type.clone();
        self.validate_local_type(outcome, &return_type, span)?;
        self.function.instruction(&Instruction::LocalGet(outcome));
        self.return_stack_value()?;
        self.function.instruction(&Instruction::End);
        self.exit_control()?;
        self.function.instruction(&Instruction::LocalGet(outcome));
        self.clear_root_slot(outcome)
    }

    /// Returns a fatal language Error from the current function or leaves the value on stack.
    pub(in crate::codegen::function) fn return_if_fatal_error(
        &mut self,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let outcome = self.store_stack_value()?;
        self.function.instruction(&Instruction::LocalGet(outcome));
        self.runtime_call("__exs_rt_is_fatal_error", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.enter_control()?;
        self.function.instruction(&Instruction::LocalGet(outcome));
        self.return_stack_value()?;
        self.function.instruction(&Instruction::End);
        self.exit_control()?;
        self.function.instruction(&Instruction::LocalGet(outcome));
        self.clear_root_slot(outcome)
    }

    /// Validates the ValueRef on stack against this function's declared return type.
    pub(in crate::codegen::function) fn validate_return_type(
        &mut self,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let value = self.return_value_local;
        self.store_stack_value_in(value)?;
        let return_type = self.return_type.clone();
        self.validate_local_type(value, &return_type, span)?;
        self.function.instruction(&Instruction::LocalGet(value));
        self.clear_root_slot(value)
    }

    /// Checks one rooted local against a type mask and returns a mismatch Error or traps.
    pub(in crate::codegen::function) fn validate_local_type(
        &mut self,
        local: u32,
        contract: &TypeContract,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let matches = self.type_match_local;
        self.function.instruction(&Instruction::LocalGet(local));
        immediate::type_mask_test(&mut self.function, contract.builtin_mask);
        self.function.instruction(&Instruction::LocalSet(matches));
        for type_id in &contract.nominal_type_ids {
            self.function.instruction(&Instruction::LocalGet(matches));
            self.function.instruction(&Instruction::LocalGet(local));
            self.emit_object_type_test(*type_id, span)?;
            self.function.instruction(&Instruction::I32Or);
            self.function.instruction(&Instruction::LocalSet(matches));
        }
        for type_id in &contract.enum_type_ids {
            self.function.instruction(&Instruction::LocalGet(matches));
            self.function.instruction(&Instruction::LocalGet(local));
            self.compile_string(type_id, span)?;
            self.runtime_call("__exs_rt_enum_is_type", span)?;
            self.function.instruction(&Instruction::I32Or);
            self.function.instruction(&Instruction::LocalSet(matches));
        }
        self.function.instruction(&Instruction::LocalGet(matches));
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.enter_control()?;
        self.function.instruction(&Instruction::Else);
        self.function.instruction(&Instruction::LocalGet(local));
        self.function
            .instruction(&Instruction::I32Const(i32::from(types::permits_error(
                &self.return_type,
            ))));
        self.runtime_call("__exs_rt_type_mismatch", span)?;
        self.return_stack_value()?;
        self.function.instruction(&Instruction::End);
        self.exit_control()
    }

    /// Compiles an expression and verifies it is a boolean without consuming it.
    pub(in crate::codegen::function) fn checked_boolean_expression(
        &mut self,
        expression: &Expression<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(expression)?;
        let temporary = self.store_stack_value()?;
        self.function.instruction(&Instruction::LocalGet(temporary));
        self.compile_condition(condition_span(expression))?;
        self.function.instruction(&Instruction::Drop);
        self.function.instruction(&Instruction::LocalGet(temporary));
        self.clear_root_slot(temporary)?;
        Ok(())
    }
}
