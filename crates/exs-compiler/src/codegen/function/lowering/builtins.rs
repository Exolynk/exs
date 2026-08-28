use super::*;

impl<'a, 'module> FunctionCompiler<'a, 'module> {
    /// Constructs one enum value after validating each payload field in declaration order.
    pub(in crate::codegen::function) fn compile_enum_variant(
        &mut self,
        variant: &EnumVariant,
        arguments: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.runtime_call("__exs_rt_list_new", span)?;
        let fields = self.store_stack_value()?;
        for (argument, contract) in arguments.iter().zip(&variant.fields) {
            self.compile_expression(argument)?;
            let value = self.store_stack_value()?;
            self.validate_local_type(value, contract, span)?;
            self.function.instruction(&Instruction::LocalGet(fields));
            self.function.instruction(&Instruction::LocalGet(value));
            self.runtime_value_call("__exs_rt_append", 2, span)?;
            self.function.instruction(&Instruction::Drop);
            self.clear_root_slot(value)?;
        }
        self.compile_string(&variant.type_identity, span)?;
        let type_identity = self.store_stack_value()?;
        self.compile_string(&variant.name, span)?;
        let variant_name = self.store_stack_value()?;
        self.function
            .instruction(&Instruction::I32Const(variant.type_id.cast_signed()));
        self.function
            .instruction(&Instruction::LocalGet(type_identity));
        self.function
            .instruction(&Instruction::LocalGet(variant_name));
        self.function.instruction(&Instruction::LocalGet(fields));
        self.runtime_call("__exs_rt_enum_new", span)?;
        self.clear_root_slot(type_identity)?;
        self.clear_root_slot(variant_name)?;
        self.clear_root_slot(fields)
    }

    /// Compiles the reserved `Error(kind, message, data)` source constructor.
    pub(in crate::codegen::function) fn compile_error_builtin(
        &mut self,
        arguments: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if arguments.len() != 3 {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0208",
                span,
                format!(
                    "constructor `Error` expects 3 arguments but received {}",
                    arguments.len()
                ),
            )));
        }
        let mut argument_locals = Vec::new();
        for argument in arguments {
            self.compile_expression(argument)?;
            argument_locals.push(self.store_stack_value()?);
        }
        for local in &argument_locals {
            self.function.instruction(&Instruction::LocalGet(*local));
        }
        self.runtime_value_call("__exs_rt_error_new", 3, span)?;
        for local in argument_locals {
            self.clear_root_slot(local)?;
        }
        Ok(())
    }

    /// Compiles the fatal `assert(condition[, description])` standard intrinsic.
    pub(in crate::codegen::function) fn compile_assert_builtin(
        &mut self,
        arguments: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if !(1..=2).contains(&arguments.len()) {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0208",
                span,
                format!(
                    "assert expects 1 or 2 arguments but received {}",
                    arguments.len()
                ),
            )));
        }
        self.compile_expression(&arguments[0])?;
        let condition = self.store_stack_value()?;
        if let Some(description) = arguments.get(1) {
            self.compile_expression(description)?;
        } else {
            self.compile_string(crate::codegen::standard::ASSERT_DEFAULT_DESCRIPTION, span)?;
        }
        let description = self.store_stack_value()?;
        self.function.instruction(&Instruction::LocalGet(condition));
        self.function
            .instruction(&Instruction::LocalGet(description));
        self.runtime_value_call("__exs_rt_assert", 2, span)?;
        self.return_if_fatal_error(span)?;
        self.clear_root_slot(condition)?;
        self.clear_root_slot(description)
    }

    /// Compiles the fatal `assert_eq(actual, expected[, description])` standard intrinsic.
    pub(in crate::codegen::function) fn compile_assert_eq_builtin(
        &mut self,
        arguments: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if !(2..=3).contains(&arguments.len()) {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0208",
                span,
                format!(
                    "assert_eq expects 2 or 3 arguments but received {}",
                    arguments.len()
                ),
            )));
        }
        let mut locals = Vec::with_capacity(3);
        for argument in &arguments[..2] {
            self.compile_expression(argument)?;
            locals.push(self.store_stack_value()?);
        }
        if let Some(description) = arguments.get(2) {
            self.compile_expression(description)?;
        } else {
            self.compile_string(
                crate::codegen::standard::ASSERT_EQ_DEFAULT_DESCRIPTION,
                span,
            )?;
        }
        locals.push(self.store_stack_value()?);
        let operator = TraitOperator::from_binary(BinaryOperator::Equal).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "missing standard equality operator",
            ))
        })?;
        let targets = self.methods.operator(operator).to_vec();
        self.emit_operator_dispatch(operator, &targets, 0, locals[0], locals[1], span)?;
        let comparison = self.store_stack_value()?;
        self.function
            .instruction(&Instruction::LocalGet(comparison));
        for local in &locals {
            self.function.instruction(&Instruction::LocalGet(*local));
        }
        self.runtime_value_call("__exs_rt_assert_eq", 4, span)?;
        self.return_if_fatal_error(span)?;
        self.clear_root_slot(comparison)?;
        for local in locals {
            self.clear_root_slot(local)?;
        }
        Ok(())
    }
}
