use super::*;

impl<'a, 'module> FunctionCompiler<'a, 'module> {
    /// Compiles one static source string through the compiler-owned literal pool.
    pub(in crate::codegen::function) fn compile_string(
        &mut self,
        value: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
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
        self.runtime_call("__exs_rt_literal_buffer_alloc", span)?;
        let buffer_pointer = self.allocate_local();
        self.function
            .instruction(&Instruction::LocalTee(buffer_pointer));
        self.function.instruction(&Instruction::I32Const(0));
        self.function.instruction(&Instruction::I32Const(length));
        self.function
            .instruction(&Instruction::MemoryInit { mem: 0, data_index });
        self.function
            .instruction(&Instruction::LocalGet(buffer_pointer));
        self.function.instruction(&Instruction::I32Const(length));
        self.runtime_call("__exs_rt_string_new", span)
    }

    /// Compiles one static source Bytes literal through the compiler-owned literal pool.
    pub(in crate::codegen::function) fn compile_bytes(
        &mut self,
        value: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let data_index = self.literals.get(value).copied().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0211",
                span,
                "missing compiler Bytes literal data segment",
            ))
        })?;
        let length = i32::try_from(value.len()).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0211",
                span,
                "Bytes literal is too large for Wasm linear memory",
            ))
        })?;
        self.function.instruction(&Instruction::I32Const(length));
        self.runtime_call("__exs_rt_literal_buffer_alloc", span)?;
        let buffer_pointer = self.allocate_local();
        self.function
            .instruction(&Instruction::LocalTee(buffer_pointer));
        self.function.instruction(&Instruction::I32Const(0));
        self.function.instruction(&Instruction::I32Const(length));
        self.function
            .instruction(&Instruction::MemoryInit { mem: 0, data_index });
        self.function
            .instruction(&Instruction::LocalGet(buffer_pointer));
        self.function.instruction(&Instruction::I32Const(length));
        self.runtime_call("__exs_rt_bytes_new", span)
    }

    /// Concatenates formatted-string fragments with the standard String-add conversion rules.
    pub(in crate::codegen::function) fn compile_formatted_string(
        &mut self,
        parts: &[FormattedStringPart<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_string("", span)?;
        let result = self.store_stack_value()?;
        for part in parts {
            match part {
                FormattedStringPart::Text(value) => self.compile_string(value, span)?,
                FormattedStringPart::Expression(expression) => {
                    self.compile_to_string(expression, span)?
                }
            }
            let value = self.store_stack_value()?;
            self.function.instruction(&Instruction::LocalGet(result));
            self.function.instruction(&Instruction::LocalGet(value));
            self.runtime_value_call("__exs_rt_add", 2, span)?;
            self.store_stack_value_in(result)?;
            self.clear_root_slot(value)?;
        }
        self.function.instruction(&Instruction::LocalGet(result));
        self.clear_root_slot(result)
    }

    /// Evaluates one formatted value and invokes its `ToString` implementation.
    pub(in crate::codegen::function) fn compile_to_string(
        &mut self,
        expression: &Expression<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(expression)?;
        let receiver = self.store_stack_value()?;
        let method = crate::ast::Identifier {
            name: crate::codegen::standard::TO_STRING_METHOD.to_owned(),
            span,
        };
        let targets = self
            .methods
            .trait_instance(crate::codegen::standard::TO_STRING_TRAIT, &method.name)
            .map(ToOwned::to_owned);
        self.emit_instance_method_dispatch(
            targets.as_deref().unwrap_or_default(),
            0,
            receiver,
            &[],
            &method,
            span,
        )?;
        self.clear_root_slot(receiver)
    }

    /// Constructs a runtime list while evaluating every element in source order.
    pub(in crate::codegen::function) fn compile_list(
        &mut self,
        elements: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.runtime_call("__exs_rt_list_new", span)?;
        let list_local = self.store_stack_value()?;
        for element in elements {
            self.compile_expression(element)?;
            let element = self.store_stack_value()?;
            self.function
                .instruction(&Instruction::LocalGet(list_local));
            self.function.instruction(&Instruction::LocalGet(element));
            self.runtime_value_call("__exs_rt_append", 2, span)?;
            self.function.instruction(&Instruction::Drop);
            self.clear_root_slot(element)?;
        }
        self.function
            .instruction(&Instruction::LocalGet(list_local));
        Ok(())
    }

    /// Constructs a runtime object while evaluating property values in source order.
    pub(in crate::codegen::function) fn compile_object(
        &mut self,
        properties: &[ObjectProperty<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.runtime_call("__exs_rt_object_new", span)?;
        let object_local = self.store_stack_value()?;
        for property in properties {
            self.compile_string(&property.key, property.key_span)?;
            let key = self.store_stack_value()?;
            self.compile_expression(&property.value)?;
            let value = self.store_stack_value()?;
            self.function
                .instruction(&Instruction::LocalGet(object_local));
            self.function.instruction(&Instruction::LocalGet(key));
            self.function.instruction(&Instruction::LocalGet(value));
            self.runtime_value_call("__exs_rt_index_set", 3, property.span)?;
            self.function.instruction(&Instruction::Drop);
            self.clear_root_slot(key)?;
            self.clear_root_slot(value)?;
        }
        self.function
            .instruction(&Instruction::LocalGet(object_local));
        Ok(())
    }

    /// Constructs one nominal Object and validates every declared field contract.
    pub(in crate::codegen::function) fn compile_typed_object(
        &mut self,
        type_name: &crate::ast::Identifier<'a>,
        properties: &[ObjectProperty<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let nominal = self.types.get(&type_name.name).cloned().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0216",
                type_name.span,
                format!("unknown type `{}`", type_name.name),
            ))
        })?;
        if nominal.kind != NominalKind::Object {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0216",
                type_name.span,
                format!("enum `{}` requires a variant constructor", type_name.name),
            )));
        }
        for property in properties {
            if !nominal
                .fields
                .iter()
                .any(|field| field.name == property.key)
            {
                self.return_type_error(property.key_span)?;
                return Ok(());
            }
            if properties
                .iter()
                .filter(|other| other.key == property.key)
                .count()
                > 1
            {
                self.return_type_error(property.key_span)?;
                return Ok(());
            }
        }
        self.function
            .instruction(&Instruction::I32Const(nominal.id.cast_signed()));
        self.runtime_call("__exs_rt_object_typed_new", span)?;
        let object = self.store_stack_value()?;
        for property in properties {
            let field = nominal
                .fields
                .iter()
                .find(|field| field.name == property.key)
                .ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0999",
                        property.key_span,
                        "missing resolved nominal field",
                    ))
                })?;
            self.compile_expression(&property.value)?;
            let value = self.store_stack_value()?;
            self.validate_local_type(value, &field.contract, property.span)?;
            self.compile_string(&property.key, property.key_span)?;
            let key = self.store_stack_value()?;
            self.function.instruction(&Instruction::LocalGet(object));
            self.function.instruction(&Instruction::LocalGet(key));
            self.function.instruction(&Instruction::LocalGet(value));
            self.runtime_value_call("__exs_rt_index_set", 3, property.span)?;
            self.function.instruction(&Instruction::Drop);
            self.clear_root_slot(key)?;
            self.clear_root_slot(value)?;
        }
        for field in &nominal.fields {
            if properties.iter().any(|property| property.key == field.name) {
                continue;
            }
            self.runtime_call("__exs_rt_none_new", span)?;
            let value = self.store_stack_value()?;
            self.validate_local_type(value, &field.contract, span)?;
            self.compile_string(&field.name, span)?;
            let key = self.store_stack_value()?;
            self.function.instruction(&Instruction::LocalGet(object));
            self.function.instruction(&Instruction::LocalGet(key));
            self.function.instruction(&Instruction::LocalGet(value));
            self.runtime_value_call("__exs_rt_index_set", 3, span)?;
            self.function.instruction(&Instruction::Drop);
            self.clear_root_slot(key)?;
            self.clear_root_slot(value)?;
        }
        self.function.instruction(&Instruction::LocalGet(object));
        Ok(())
    }
}
