//! Expression lowering and function contract validation.

use exs_value::is_valid_int;
use wasm_encoder::{BlockType, Instruction, ValType};

use crate::ast::{BinaryOperator, Expression, ObjectProperty, UnaryOperator};
use crate::codegen::diagnostics;
use crate::codegen::types::{self, TypeContract};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

use super::FunctionCompiler;
use super::analysis::{condition_span, runtime_operation};

impl<'a, 'module> FunctionCompiler<'a, 'module> {
    /// Compiles one source expression into a ValueRef on the Wasm stack.
    pub(super) fn compile_expression(
        &mut self,
        expression: &Expression<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        match expression {
            Expression::Integer(value, span) => {
                if !is_valid_int(*value) {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0206",
                        *span,
                        "integer literal is outside the ExS 56-bit range",
                    )));
                }
                self.function.instruction(&Instruction::I64Const(*value));
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
                let local = self.lookup(&identifier.name).ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0205",
                        identifier.span,
                        format!("unknown binding `{}`", identifier.name),
                    ))
                })?;
                self.function.instruction(&Instruction::LocalGet(local));
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
                        .filter(|value| is_valid_int(*value))
                        .ok_or_else(|| {
                            diagnostics(CompileDiagnostic::new(
                                "E0206",
                                *operand_span,
                                "integer literal is outside the ExS 56-bit range",
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
                    self.function.instruction(&Instruction::LocalGet(left));
                    self.function.instruction(&Instruction::LocalGet(right));
                    self.runtime_value_call(runtime_operation(*operator), 2, *span)?;
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
                let signature = self.signatures.get(&callee.name).ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0207",
                        callee.span,
                        format!("unknown function `{}`", callee.name),
                    ))
                })?;
                if signature.arity != arguments.len() {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0208",
                        *span,
                        format!(
                            "function `{}` expects {} arguments but received {}",
                            callee.name,
                            signature.arity,
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
                self.set_runtime_call_site(*span)?;
                self.function
                    .instruction(&Instruction::Call(signature.index));
                for local in argument_locals {
                    self.clear_root_slot(local)?;
                }
            }
            Expression::HostCall { span, .. } => {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0300",
                    *span,
                    "host.call requires the Phase 8 continuation lowerer",
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

    /// Compiles the reserved `Error(kind, message, data)` source constructor.
    pub(super) fn compile_error_builtin(
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

    /// Compiles one static nominal type method call.
    pub(super) fn compile_static_method_call(
        &mut self,
        type_name: &crate::ast::Identifier<'a>,
        method: &crate::ast::Identifier<'a>,
        arguments: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if self.types.get(&type_name.name).is_none() {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0216",
                type_name.span,
                format!("unknown type `{}`", type_name.name),
            )));
        }
        let key = format!("{}::{}", type_name.name, method.name);
        let signature = self.signatures.get(&key).ok_or_else(|| {
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
        if signature.arity != arguments.len() {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0208",
                span,
                format!(
                    "static method `{}::{}` expects {} arguments but received {}",
                    type_name.name,
                    method.name,
                    signature.arity,
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
        self.set_runtime_call_site(span)?;
        self.function
            .instruction(&Instruction::Call(signature.index));
        for local in argument_locals {
            self.clear_root_slot(local)?;
        }
        Ok(())
    }

    /// Compiles an instance implementation dispatch with generic runtime-method fallback.
    pub(super) fn compile_method_call(
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
    fn emit_instance_method_dispatch(
        &mut self,
        targets: &[super::method::InstanceMethod],
        index: usize,
        receiver: u32,
        arguments: &[u32],
        method: &crate::ast::Identifier<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let Some(target) = targets.get(index) else {
            return self.compile_runtime_method_call(receiver, arguments, method, span);
        };
        self.function.instruction(&Instruction::LocalGet(receiver));
        self.function
            .instruction(&Instruction::I32Const(target.type_id.cast_signed()));
        self.runtime_call("__exs_rt_object_is_type", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        self.enter_control()?;
        if target.signature.arity == arguments.len() + 1 {
            self.function.instruction(&Instruction::LocalGet(receiver));
            for argument in arguments {
                self.function.instruction(&Instruction::LocalGet(*argument));
            }
            self.set_runtime_call_site(span)?;
            self.function
                .instruction(&Instruction::Call(target.signature.index));
        } else {
            self.function.instruction(&Instruction::LocalGet(receiver));
            self.runtime_value_call("__exs_rt_method_arity_error", 1, span)?;
        }
        self.function.instruction(&Instruction::Else);
        self.emit_instance_method_dispatch(targets, index + 1, receiver, arguments, method, span)?;
        self.function.instruction(&Instruction::End);
        self.exit_control()
    }

    /// Calls the generic runtime method dispatcher using already evaluated argument locals.
    fn compile_runtime_method_call(
        &mut self,
        receiver: u32,
        arguments: &[u32],
        method: &crate::ast::Identifier<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
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

    /// Compiles one static source string through the compiler-owned literal pool.
    pub(super) fn compile_string(
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

    /// Constructs a runtime list while evaluating every element in source order.
    pub(super) fn compile_list(
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
    pub(super) fn compile_object(
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
    pub(super) fn compile_typed_object(
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

    /// Returns the current function's recoverable or fatal generic type-contract Error.
    fn return_type_error(&mut self, span: SourceSpan<'a>) -> Result<(), CompileDiagnostics<'a>> {
        self.runtime_call("__exs_rt_none_new", span)?;
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
    pub(super) fn compile_logical(
        &mut self,
        left: &Expression<'a>,
        right: &Expression<'a>,
        is_or: bool,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(left)?;
        self.compile_condition(span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        self.enter_control()?;
        if is_or {
            self.function.instruction(&Instruction::I32Const(1));
            self.runtime_call("__exs_rt_bool_new", span)?;
        } else {
            self.checked_boolean_expression(right)?;
        }
        self.function.instruction(&Instruction::Else);
        if is_or {
            self.checked_boolean_expression(right)?;
        } else {
            self.function.instruction(&Instruction::I32Const(0));
            self.runtime_call("__exs_rt_bool_new", span)?;
        }
        self.function.instruction(&Instruction::End);
        self.exit_control()
    }

    /// Lowers the postfix propagation operator for Option and Result values.
    pub(super) fn compile_propagate(
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
        self.runtime_value_call("__exs_rt_propagate", 1, span)?;
        self.return_if_error(span)
    }

    /// Validates the ValueRef on the stack as a Boolean and lowers it to Wasm i32 control flow.
    pub(super) fn compile_condition(
        &mut self,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.runtime_value_call("__exs_rt_condition_value", 1, span)?;
        self.return_if_error(span)?;
        self.runtime_value_call("__exs_rt_condition", 1, span)
    }

    /// Returns a language Error from the current function or leaves the non-Error value on stack.
    pub(super) fn return_if_error(
        &mut self,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let outcome = self.store_stack_value()?;
        self.function.instruction(&Instruction::LocalGet(outcome));
        self.runtime_value_call("__exs_rt_is_error", 1, span)?;
        self.runtime_value_call("__exs_rt_condition", 1, span)?;
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

    /// Validates the ValueRef on stack against this function's declared return type.
    pub(super) fn validate_return_type(
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
    pub(super) fn validate_local_type(
        &mut self,
        local: u32,
        contract: &TypeContract,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let matches = self.type_match_local;
        self.function.instruction(&Instruction::LocalGet(local));
        self.function
            .instruction(&Instruction::I32Const(contract.builtin_mask.cast_signed()));
        self.runtime_call("__exs_rt_type_matches", span)?;
        self.function.instruction(&Instruction::LocalSet(matches));
        for type_id in &contract.nominal_type_ids {
            self.function.instruction(&Instruction::LocalGet(matches));
            self.function.instruction(&Instruction::LocalGet(local));
            self.function
                .instruction(&Instruction::I32Const(type_id.cast_signed()));
            self.runtime_call("__exs_rt_object_is_type", span)?;
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
    pub(super) fn checked_boolean_expression(
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
