use super::*;

impl<'a, 'module> FunctionCompiler<'a, 'module> {
    /// Compiles one static nominal type method call.
    pub(in crate::codegen::function) fn compile_static_method_call(
        &mut self,
        type_name: &crate::ast::Identifier<'a>,
        method: &crate::ast::Identifier<'a>,
        arguments: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let static_export = match (type_name.name.as_str(), method.name.as_str()) {
            ("Bytes", "from_list") => Some(("__exs_rt_bytes_from_list", 1)),
            ("Bytes", "from_utf8") => Some(("__exs_rt_bytes_from_utf8", 1)),
            ("Bytes", "from_hex") => Some(("__exs_rt_bytes_from_hex", 1)),
            ("Bytes", "from_base64") => Some(("__exs_rt_bytes_from_base64", 1)),
            ("Int", "parse") => Some(("__exs_rt_integer_parse", 1)),
            ("Float", "parse") => Some(("__exs_rt_float_parse", 1)),
            ("Object", "from_entries") => Some(("__exs_rt_object_from_entries", 1)),
            ("Math", "pi") => Some(("__exs_rt_math_pi", 0)),
            ("Math", "tau") => Some(("__exs_rt_math_tau", 0)),
            ("Math", "e") => Some(("__exs_rt_math_e", 0)),
            ("Math", "sin") => Some(("__exs_rt_math_sin", 1)),
            ("Math", "cos") => Some(("__exs_rt_math_cos", 1)),
            ("Math", "tan") => Some(("__exs_rt_math_tan", 1)),
            ("Math", "asin") => Some(("__exs_rt_math_asin", 1)),
            ("Math", "acos") => Some(("__exs_rt_math_acos", 1)),
            ("Math", "atan") => Some(("__exs_rt_math_atan", 1)),
            ("Math", "exp") => Some(("__exs_rt_math_exp", 1)),
            ("Math", "ln") => Some(("__exs_rt_math_ln", 1)),
            ("Math", "log2") => Some(("__exs_rt_math_log2", 1)),
            ("Math", "log10") => Some(("__exs_rt_math_log10", 1)),
            ("Math", "atan2") => Some(("__exs_rt_math_atan2", 2)),
            ("Math", "hypot") => Some(("__exs_rt_math_hypot", 2)),
            _ => None,
        };
        if let Some((static_export, arity)) = static_export {
            if arguments.len() != arity as usize {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0208",
                    span,
                    format!(
                        "static method `{}::{}` expects {} arguments but received {}",
                        type_name.name,
                        method.name,
                        arity,
                        arguments.len()
                    ),
                )));
            }
            for argument in arguments {
                self.compile_expression(argument)?;
            }
            self.runtime_value_call(static_export, arity, span)?;
            return Ok(());
        }
        let key = format!("{}::{}", type_name.name, method.name);
        if let Some(variant) = self.types.enum_variant(&key) {
            if variant.fields.len() != arguments.len() {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0208",
                    span,
                    format!(
                        "enum variant `{}` expects {} arguments but received {}",
                        variant.name,
                        variant.fields.len(),
                        arguments.len()
                    ),
                )));
            }
            return self.compile_enum_variant(variant, arguments, span);
        }
        if self.types.get(&type_name.name).is_none() {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0216",
                type_name.span,
                format!("unknown type `{}`", type_name.name),
            )));
        }
        let signature = self.signatures.get(&key).cloned().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0225",
                method.span,
                format!(
                    "type `{}` has no static method `{}`",
                    type_name.name, method.name
                ),
            ))
        })?;
        if !self.methods.is_static(&key) {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0225",
                method.span,
                format!(
                    "type `{}` method `{}` requires a receiver",
                    type_name.name, method.name
                ),
            )));
        }
        if !signature.accepts_arity(arguments.len()) {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0208",
                span,
                format!(
                    "static method `{}::{}` expects {} arguments but received {}",
                    type_name.name,
                    method.name,
                    signature.expected_arity_description(),
                    arguments.len()
                ),
            )));
        }
        let mut argument_locals = Vec::new();
        for argument in arguments {
            self.compile_expression(argument)?;
            argument_locals.push(self.store_stack_value()?);
        }
        let packed_tail = self.emit_call_arguments(&argument_locals, &signature, span)?;
        self.set_runtime_call_site(span)?;
        self.function
            .instruction(&Instruction::Call(signature.index));
        self.return_if_fatal_error(span)?;
        if let Some(list) = packed_tail {
            self.clear_root_slot(list)?;
        }
        for local in argument_locals {
            self.clear_root_slot(local)?;
        }
        Ok(())
    }

    /// Compiles an instance implementation dispatch with generic runtime-method fallback.
    pub(in crate::codegen::function) fn compile_method_call(
        &mut self,
        receiver: &Expression<'a>,
        method: &crate::ast::Identifier<'a>,
        arguments: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(receiver)?;
        let receiver = self.store_stack_value()?;
        let mut argument_locals = Vec::new();
        for argument in arguments {
            self.compile_expression(argument)?;
            argument_locals.push(self.store_stack_value()?);
        }
        let targets = self.methods.instance(&method.name).map(ToOwned::to_owned);
        self.emit_instance_method_dispatch(
            targets.as_deref().unwrap_or_default(),
            0,
            receiver,
            &argument_locals,
            method,
            span,
        )?;
        self.clear_root_slot(receiver)?;
        for argument in argument_locals {
            self.clear_root_slot(argument)?;
        }
        Ok(())
    }

    /// Emits nested Wasm branches that dispatch one static method name by nominal Object tag.
    pub(in crate::codegen::function) fn emit_instance_method_dispatch(
        &mut self,
        targets: &[super::method::InstanceMethod],
        index: usize,
        receiver: u32,
        arguments: &[u32],
        method: &crate::ast::Identifier<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let Some(target) = targets.get(index) else {
            if let Some(helper) = crate::prelude::iterator_helper(&method.name) {
                return self.compile_standard_callback_method_call(
                    receiver, arguments, method, helper, span,
                );
            }
            return self.compile_runtime_method_call(receiver, arguments, method, span);
        };
        self.function.instruction(&Instruction::LocalGet(receiver));
        self.function
            .instruction(&Instruction::I32Const(target.type_id.cast_signed()));
        self.runtime_call("__exs_rt_object_is_type", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        self.enter_control()?;
        if target.signature.accepts_arity(arguments.len() + 1) {
            self.function.instruction(&Instruction::LocalGet(receiver));
            let fixed_arity = target.signature.arity.saturating_sub(1);
            for argument in arguments.iter().take(fixed_arity) {
                self.function.instruction(&Instruction::LocalGet(*argument));
            }
            let packed_tail =
                self.emit_variadic_tail(arguments, fixed_arity, target.signature.variadic, span)?;
            self.set_runtime_call_site(span)?;
            self.function
                .instruction(&Instruction::Call(target.signature.index));
            self.return_if_fatal_error(span)?;
            if let Some(list) = packed_tail {
                self.clear_root_slot(list)?;
            }
        } else {
            self.function.instruction(&Instruction::LocalGet(receiver));
            self.runtime_value_call("__exs_rt_method_arity_error", 1, span)?;
        }
        self.function.instruction(&Instruction::Else);
        self.emit_instance_method_dispatch(targets, index + 1, receiver, arguments, method, span)?;
        self.function.instruction(&Instruction::End);
        self.exit_control()
    }

    /// Calls one private eager iterable helper after nominal method dispatch misses.
    fn compile_standard_callback_method_call(
        &mut self,
        receiver: u32,
        arguments: &[u32],
        method: &crate::ast::Identifier<'a>,
        helper: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let signature = self.signatures.get(helper).cloned().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0999",
                method.span,
                "missing standard iterable helper",
            ))
        })?;
        if !signature.accepts_arity(arguments.len() + 1) {
            self.function.instruction(&Instruction::LocalGet(receiver));
            return self.runtime_value_call("__exs_rt_method_arity_error", 1, span);
        }
        self.function.instruction(&Instruction::LocalGet(receiver));
        for argument in arguments {
            self.function.instruction(&Instruction::LocalGet(*argument));
        }
        self.set_runtime_call_site(span)?;
        self.function
            .instruction(&Instruction::Call(signature.index));
        self.return_if_fatal_error(span)
    }

    /// Emits nominal trait dispatch before preserving one operator's runtime built-in fallback.
    pub(in crate::codegen::function) fn emit_operator_dispatch(
        &mut self,
        operator: TraitOperator,
        targets: &[super::method::InstanceMethod],
        index: usize,
        left: u32,
        right: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let Some(target) = targets.get(index) else {
            self.function.instruction(&Instruction::LocalGet(left));
            self.function.instruction(&Instruction::LocalGet(right));
            self.runtime_value_call(operator.runtime_export(), 2, span)?;
            return self.emit_operator_result(operator, span);
        };
        self.function.instruction(&Instruction::LocalGet(left));
        self.function
            .instruction(&Instruction::I32Const(target.type_id.cast_signed()));
        self.runtime_call("__exs_rt_object_is_type", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        self.enter_control()?;
        self.function.instruction(&Instruction::LocalGet(left));
        self.function.instruction(&Instruction::LocalGet(right));
        self.set_runtime_call_site(span)?;
        self.function
            .instruction(&Instruction::Call(target.signature.index));
        self.return_if_fatal_error(span)?;
        self.emit_operator_result(operator, span)?;
        self.function.instruction(&Instruction::Else);
        self.emit_operator_dispatch(operator, targets, index + 1, left, right, span)?;
        self.function.instruction(&Instruction::End);
        self.exit_control()
    }

    /// Converts one `Compare` result into the source operator's final Bool result.
    pub(in crate::codegen::function) fn emit_operator_result(
        &mut self,
        operator: TraitOperator,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let Some(test) = operator.comparison_test() else {
            return Ok(());
        };
        self.return_if_error(span)?;
        self.function.instruction(&Instruction::I32Const(test));
        self.runtime_value_call("__exs_rt_ordering_test", 2, span)
    }

    /// Calls the generic runtime method dispatcher using already evaluated argument locals.
    pub(in crate::codegen::function) fn compile_runtime_method_call(
        &mut self,
        receiver: u32,
        arguments: &[u32],
        method: &crate::ast::Identifier<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if let Some(export) = builtin_runtime_method_export(&method.name, arguments.len()) {
            self.function.instruction(&Instruction::LocalGet(receiver));
            for argument in arguments {
                self.function.instruction(&Instruction::LocalGet(*argument));
            }
            return self.runtime_value_call(export, arguments.len() as u32 + 1, span);
        }
        self.runtime_call("__exs_rt_list_new", span)?;
        let list = self.store_stack_value()?;
        for argument in arguments {
            self.function.instruction(&Instruction::LocalGet(list));
            self.function.instruction(&Instruction::LocalGet(*argument));
            self.runtime_value_call("__exs_rt_append", 2, span)?;
            self.function.instruction(&Instruction::Drop);
        }
        self.compile_string(&method.name, method.span)?;
        let method = self.store_stack_value()?;
        self.function.instruction(&Instruction::LocalGet(receiver));
        self.function.instruction(&Instruction::LocalGet(method));
        self.function.instruction(&Instruction::LocalGet(list));
        self.runtime_value_call("__exs_rt_call_method", 3, span)?;
        self.clear_root_slot(method)?;
        self.clear_root_slot(list)
    }
}

/// Returns the direct runtime export for one fixed-arity built-in member fallback.
fn builtin_runtime_method_export(method: &str, arity: usize) -> Option<&'static str> {
    match (method, arity) {
        ("length", 0) => Some("__exs_rt_length"),
        ("is_empty", 0) => Some("__exs_rt_is_empty"),
        ("slice", 2) => Some("__exs_rt_slice"),
        ("contains", 1) => Some("__exs_rt_contains"),
        ("starts_with", 1) => Some("__exs_rt_starts_with"),
        ("ends_with", 1) => Some("__exs_rt_ends_with"),
        ("trim", 0) => Some("__exs_rt_trim"),
        ("trim_start", 0) => Some("__exs_rt_trim_start"),
        ("trim_end", 0) => Some("__exs_rt_trim_end"),
        ("replace", 2) => Some("__exs_rt_replace"),
        ("split", 1) => Some("__exs_rt_split"),
        _ => None,
    }
}
