use super::*;

impl<'a, 'module> FunctionCompiler<'a, 'module> {
    /// Compiles one static source string through the compiler-owned literal pool.
    pub(in crate::codegen::function) fn compile_string(
        &mut self,
        value: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
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
        self.enter_control()?;
        self.function.instruction(&Instruction::I32Const(length));
        self.runtime_call("__exs_rt_literal_buffer_alloc", span)?;
        let buffer_pointer = self.literal_pointer_local;
        self.function
            .instruction(&Instruction::LocalTee(buffer_pointer));
        self.function.instruction(&Instruction::I32Const(0));
        self.function.instruction(&Instruction::I32Const(length));
        self.function
            .instruction(&Instruction::MemoryInit { mem: 0, data_index });
        self.function
            .instruction(&Instruction::LocalGet(buffer_pointer));
        self.function.instruction(&Instruction::I32Const(length));
        self.runtime_call("__exs_rt_string_new", span)?;
        self.runtime_call("__exs_rt_literal_string_pin", span)?;
        self.function
            .instruction(&Instruction::GlobalSet(global_index));
        self.function.instruction(&Instruction::End);
        self.function
            .instruction(&Instruction::GlobalGet(global_index));
        self.exit_control()
    }

    /// Compiles one static source Bytes literal through the compiler-owned literal pool.
    pub(in crate::codegen::function) fn compile_bytes(
        &mut self,
        value: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
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
        self.runtime_call("__exs_rt_literal_buffer_alloc", span)?;
        let buffer_pointer = self.literal_pointer_local;
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

    /// Builds one formatted string in a single runtime builder, evaluating parts in order.
    ///
    /// Values whose type has a `ToString` implementation call it; every other value is
    /// rendered directly by the runtime, matching the default `to_string` fallback.
    pub(in crate::codegen::function) fn compile_formatted_string(
        &mut self,
        parts: &[FormattedStringPart<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let capacity = parts.iter().fold(0_usize, |total, part| {
            total.saturating_add(match part {
                FormattedStringPart::Text(value) => value.len(),
                FormattedStringPart::Expression(_) => {
                    crate::codegen::standard::FORMATTED_VALUE_CAPACITY
                }
            })
        });
        self.function.instruction(&Instruction::I32Const(
            i32::try_from(capacity).unwrap_or(i32::MAX),
        ));
        self.runtime_call("__exs_rt_format_new", span)?;
        let builder = self.store_stack_value()?;
        let targets = self
            .methods
            .trait_instance(
                crate::codegen::standard::TO_STRING_TRAIT,
                crate::codegen::standard::TO_STRING_METHOD,
            )
            .map(ToOwned::to_owned)
            .unwrap_or_default();
        for part in parts {
            match part {
                FormattedStringPart::Text(value) => {
                    self.compile_string(value, span)?;
                    let text = self.store_stack_value()?;
                    self.emit_format_push(builder, text, "__exs_rt_format_push", span)?;
                    self.clear_root_slot(text)?;
                }
                FormattedStringPart::Expression(expression) => {
                    self.compile_expression(expression)?;
                    let value = self.store_stack_value()?;
                    if targets.is_empty() {
                        self.emit_format_push(
                            builder,
                            value,
                            "__exs_rt_format_push_default",
                            span,
                        )?;
                    } else {
                        for (index, target) in targets.iter().enumerate() {
                            self.function.instruction(&Instruction::LocalGet(value));
                            self.emit_object_type_test(target.type_id, span)?;
                            if index > 0 {
                                self.function.instruction(&Instruction::I32Or);
                            }
                        }
                        self.function
                            .instruction(&Instruction::If(BlockType::Empty));
                        self.enter_control()?;
                        let method = crate::ast::Identifier {
                            name: crate::codegen::standard::TO_STRING_METHOD.to_owned(),
                            span,
                        };
                        self.emit_instance_method_dispatch(&targets, 0, value, &[], &method, span)?;
                        let rendered = self.store_stack_value()?;
                        self.emit_format_push(builder, rendered, "__exs_rt_format_push", span)?;
                        self.clear_root_slot(rendered)?;
                        self.function.instruction(&Instruction::Else);
                        self.emit_format_push(
                            builder,
                            value,
                            "__exs_rt_format_push_default",
                            span,
                        )?;
                        self.function.instruction(&Instruction::End);
                        self.exit_control()?;
                    }
                    self.clear_root_slot(value)?;
                }
            }
        }
        self.function.instruction(&Instruction::LocalGet(builder));
        self.runtime_call("__exs_rt_format_finish", span)?;
        self.clear_root_slot(builder)
    }

    /// Appends one rooted part to a rooted formatted-string builder through `export`.
    fn emit_format_push(
        &mut self,
        builder: u32,
        part: u32,
        export: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.function.instruction(&Instruction::LocalGet(builder));
        self.function.instruction(&Instruction::LocalGet(part));
        self.runtime_call(export, span)?;
        self.store_stack_value_in(builder)
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
            self.runtime_call("__exs_rt_append", span)?;
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
        if crate::codegen::standard::has_unique_keys(
            properties.iter().map(|property| property.key.as_str()),
        ) {
            return self.compile_unique_object(properties, span);
        }
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
            self.runtime_call("__exs_rt_property_set", property.span)?;
            self.function.instruction(&Instruction::Drop);
            self.clear_root_slot(key)?;
            self.clear_root_slot(value)?;
        }
        self.function
            .instruction(&Instruction::LocalGet(object_local));
        Ok(())
    }

    /// Builds an Object literal with distinct keys by appending to a pre-sized Object.
    fn compile_unique_object(
        &mut self,
        properties: &[ObjectProperty<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let capacity = i32::try_from(properties.len()).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                span,
                "too many properties for one Object literal",
            ))
        })?;
        self.function.instruction(&Instruction::I32Const(capacity));
        self.runtime_call("__exs_rt_object_new_with_capacity", span)?;
        let object = self.store_stack_value()?;
        for property in properties {
            self.compile_expression(&property.value)?;
            let value = self.store_stack_value()?;
            self.compile_string(&property.key, property.key_span)?;
            let key = self.store_stack_value()?;
            self.function.instruction(&Instruction::LocalGet(object));
            self.function.instruction(&Instruction::LocalGet(key));
            self.function.instruction(&Instruction::LocalGet(value));
            self.runtime_call("__exs_rt_object_push_field", property.span)?;
            self.clear_root_slot(key)?;
            self.clear_root_slot(value)?;
        }
        self.function.instruction(&Instruction::LocalGet(object));
        self.clear_root_slot(object)
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
        let capacity = i32::try_from(nominal.fields.len()).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                span,
                "too many fields for one nominal type",
            ))
        })?;
        self.function
            .instruction(&Instruction::I32Const(nominal.id.cast_signed()));
        self.function.instruction(&Instruction::I32Const(capacity));
        self.runtime_call("__exs_rt_object_typed_new_with_capacity", span)?;
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
            self.runtime_call("__exs_rt_object_push_field", property.span)?;
            self.clear_root_slot(key)?;
            self.clear_root_slot(value)?;
        }
        for field in &nominal.fields {
            if properties.iter().any(|property| property.key == field.name) {
                continue;
            }
            self.emit_none();
            let value = self.store_stack_value()?;
            self.validate_local_type(value, &field.contract, span)?;
            self.compile_string(&field.name, span)?;
            let key = self.store_stack_value()?;
            self.function.instruction(&Instruction::LocalGet(object));
            self.function.instruction(&Instruction::LocalGet(key));
            self.function.instruction(&Instruction::LocalGet(value));
            self.runtime_call("__exs_rt_object_push_field", span)?;
            self.clear_root_slot(key)?;
            self.clear_root_slot(value)?;
        }
        self.function.instruction(&Instruction::LocalGet(object));
        Ok(())
    }
}
