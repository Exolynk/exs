use super::*;

impl<'a, 'module> FunctionCompiler<'a, 'module> {
    /// Compiles one source expression into a ValueRef on the Wasm stack.
    pub(in crate::codegen::function) fn compile_expression(
        &mut self,
        expression: &Expression<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        match expression {
            Expression::Integer(value, span) => {
                let value = i64::try_from(*value).map_err(|_| {
                    diagnostics(CompileDiagnostic::new(
                        "E0206",
                        *span,
                        "integer literal is outside the ExS signed 64-bit range",
                    ))
                })?;
                self.function.instruction(&Instruction::I64Const(value));
                self.runtime_call("__exs_rt_int_new", *span)?;
            }
            Expression::Float(value, span) => {
                self.function
                    .instruction(&Instruction::F64Const((*value).into()));
                self.runtime_call("__exs_rt_float_new", *span)?;
            }
            Expression::String(value, span) => {
                self.compile_string(value, *span)?;
            }
            Expression::Bytes(value, span) => {
                self.compile_bytes(value, *span)?;
            }
            Expression::FormattedString { parts, span, .. } => {
                self.compile_formatted_string(parts, *span)?;
            }
            Expression::Bool(value, span) => {
                self.function
                    .instruction(&Instruction::I32Const(i32::from(*value)));
                self.runtime_call("__exs_rt_bool_new", *span)?;
            }
            Expression::None(span) => {
                self.runtime_call("__exs_rt_none_new", *span)?;
            }
            Expression::IsError { value, span } => {
                self.compile_expression(value)?;
                self.runtime_value_call("__exs_rt_is_error", 1, *span)?;
            }
            Expression::Propagate { value, span } => self.compile_propagate(value, *span)?,
            Expression::Variable(identifier) => {
                if let Some(variant) = self.types.enum_variant(&identifier.name) {
                    if !variant.fields.is_empty() {
                        return Err(diagnostics(CompileDiagnostic::new(
                            "E0208",
                            identifier.span,
                            format!(
                                "enum variant `{}` expects {} arguments",
                                variant.name,
                                variant.fields.len()
                            ),
                        )));
                    }
                    return self.compile_enum_variant(variant, &[], identifier.span);
                }
                let local = self.lookup(&identifier.name).ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0205",
                        identifier.span,
                        format!("unknown binding `{}`", identifier.name),
                    ))
                })?;
                self.function.instruction(&Instruction::LocalGet(local));
            }
            Expression::Closure { span, .. } => {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0225",
                    *span,
                    "closure lowering is not implemented",
                )));
            }
            Expression::ParallelStatic { span, .. } | Expression::ParallelDynamic { span, .. } => {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0300",
                    *span,
                    "par requires the Phase 11 continuation lowerer",
                )));
            }
            Expression::List { elements, span } => {
                self.compile_list(elements, *span)?;
            }
            Expression::Object { properties, span } => {
                self.compile_object(properties, *span)?;
            }
            Expression::TypedObject {
                type_name,
                properties,
                span,
            } => self.compile_typed_object(type_name, properties, *span)?,
            Expression::Match { span, .. } => {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0999",
                    *span,
                    "match expressions require continuation lowering",
                )));
            }
            Expression::Unary {
                operator,
                operand,
                span,
            } => {
                if matches!(operator, UnaryOperator::Negate)
                    && let Expression::Integer(value, operand_span) = operand.as_ref()
                {
                    let negative = value
                        .checked_neg()
                        .and_then(|value| i64::try_from(value).ok())
                        .ok_or_else(|| {
                            diagnostics(CompileDiagnostic::new(
                                "E0206",
                                *operand_span,
                                "integer literal is outside the ExS signed 64-bit range",
                            ))
                        })?;
                    self.function.instruction(&Instruction::I64Const(negative));
                    self.runtime_call("__exs_rt_int_new", *operand_span)?;
                    return Ok(());
                }
                self.compile_expression(operand)?;
                self.runtime_value_call(
                    match operator {
                        UnaryOperator::Negate => "__exs_rt_neg",
                        UnaryOperator::Not => "__exs_rt_not",
                    },
                    1,
                    *span,
                )?;
            }
            Expression::Binary {
                operator,
                left,
                right,
                span,
            } => match operator {
                BinaryOperator::And => self.compile_logical(left, right, false, *span)?,
                BinaryOperator::Or => self.compile_logical(left, right, true, *span)?,
                _ => {
                    self.compile_expression(left)?;
                    let left = self.store_stack_value()?;
                    self.compile_expression(right)?;
                    let right = self.store_stack_value()?;
                    if let Some(operator_trait) = TraitOperator::from_binary(*operator) {
                        let targets = self.methods.operator(operator_trait).to_vec();
                        self.emit_operator_dispatch(
                            operator_trait,
                            &targets,
                            0,
                            left,
                            right,
                            *span,
                        )?;
                    } else {
                        self.function.instruction(&Instruction::LocalGet(left));
                        self.function.instruction(&Instruction::LocalGet(right));
                        self.runtime_value_call(runtime_operation(*operator), 2, *span)?;
                    }
                    self.clear_root_slot(left)?;
                    self.clear_root_slot(right)?;
                }
            },
            Expression::Call {
                callee,
                arguments,
                span,
            } => {
                if callee.name == "Error" {
                    self.compile_error_builtin(arguments, *span)?;
                    return Ok(());
                }
                if matches!(callee.name.as_str(), "assert" | "std::test::assert") {
                    self.compile_assert_builtin(arguments, *span)?;
                    return Ok(());
                }
                if matches!(callee.name.as_str(), "assert_eq" | "std::test::assert_eq") {
                    self.compile_assert_eq_builtin(arguments, *span)?;
                    return Ok(());
                }
                if let Some(variant) = self.types.enum_variant(&callee.name) {
                    if variant.fields.len() != arguments.len() {
                        return Err(diagnostics(CompileDiagnostic::new(
                            "E0208",
                            *span,
                            format!(
                                "enum variant `{}` expects {} arguments but received {}",
                                variant.name,
                                variant.fields.len(),
                                arguments.len()
                            ),
                        )));
                    }
                    return self.compile_enum_variant(variant, arguments, *span);
                }
                let signature = self.signatures.get(&callee.name).cloned().ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0207",
                        callee.span,
                        format!("unknown function `{}`", callee.name),
                    ))
                })?;
                if !signature.accepts_arity(arguments.len()) {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0208",
                        *span,
                        format!(
                            "function `{}` expects {} arguments but received {}",
                            callee.name,
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
                let packed_tail = self.emit_call_arguments(&argument_locals, &signature, *span)?;
                self.set_runtime_call_site(*span)?;
                self.function
                    .instruction(&Instruction::Call(signature.index));
                self.return_if_fatal_error(*span)?;
                if let Some(list) = packed_tail {
                    self.clear_root_slot(list)?;
                }
                for local in argument_locals {
                    self.clear_root_slot(local)?;
                }
            }
            Expression::HostCall { span, .. }
            | Expression::HostStream { span, .. }
            | Expression::HostTime { span, .. } => {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0300",
                    *span,
                    "Host operations require the Phase 8 continuation lowerer",
                )));
            }
            Expression::MethodCall {
                receiver,
                method,
                arguments,
                span,
            } => self.compile_method_call(receiver, method, arguments, *span)?,
            Expression::StaticMethodCall {
                type_name,
                method,
                arguments,
                span,
            } => self.compile_static_method_call(type_name, method, arguments, *span)?,
            Expression::Index {
                receiver,
                index,
                span,
            } => {
                self.compile_expression(receiver)?;
                let receiver = self.store_stack_value()?;
                self.compile_expression(index)?;
                let index = self.store_stack_value()?;
                self.function.instruction(&Instruction::LocalGet(receiver));
                self.function.instruction(&Instruction::LocalGet(index));
                self.runtime_value_call("__exs_rt_index_get", 2, *span)?;
                self.clear_root_slot(receiver)?;
                self.clear_root_slot(index)?;
            }
            Expression::Property {
                receiver,
                property,
                span,
            } => {
                self.compile_expression(receiver)?;
                let receiver = self.store_stack_value()?;
                self.compile_string(&property.name, property.span)?;
                let property = self.store_stack_value()?;
                self.function.instruction(&Instruction::LocalGet(receiver));
                self.function.instruction(&Instruction::LocalGet(property));
                self.runtime_value_call("__exs_rt_index_get", 2, *span)?;
                self.clear_root_slot(receiver)?;
                self.clear_root_slot(property)?;
            }
        }
        Ok(())
    }
}
