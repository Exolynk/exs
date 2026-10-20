//! Expression lowering of the v2 backend.

use wasm_encoder::{BlockType, Instruction};

use super::function::{FunctionCompiler, error, unsupported};
use crate::ast::{BinaryOperator, Expression, UnaryOperator};
use crate::codegen::VALUE_TYPE;
use crate::codegen::immediate::{self, IntOperation};
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Core arithmetic operator codes of `__exs_core_arithmetic`.
const ADD: i32 = 0;
/// Core code of `-`.
const SUBTRACT: i32 = 1;
/// Core code of `*`.
const MULTIPLY: i32 = 2;

impl<'a> FunctionCompiler<'a, '_> {
    /// Compiles one expression, leaving its `ValueRef` on the stack.
    pub(super) fn compile_expression(
        &mut self,
        expression: &Expression<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        match expression {
            Expression::Integer(value, span) => self.compile_integer(*value, *span)?,
            Expression::Float(value, span) => {
                self.push_vm();
                self.instruction(Instruction::I64Const(value.to_bits().cast_signed()));
                self.core_call("__exs_core_float", *span)?;
            }
            Expression::Bool(value, _) => {
                immediate::push(&mut self.function, exs_value::ValueRef::boolean(*value));
            }
            Expression::None(_) => immediate::push(&mut self.function, exs_value::ValueRef::NONE),
            Expression::Variable(identifier) => {
                if self.context.types.enum_variant(&identifier.name).is_some() {
                    return Err(unsupported(identifier.span, "enum values"));
                }
                let local = self.lookup(&identifier.name).ok_or_else(|| {
                    error(
                        "E0205",
                        identifier.span,
                        format!("unknown binding `{}`", identifier.name),
                    )
                })?;
                self.instruction(Instruction::LocalGet(local));
            }
            Expression::Unary {
                operator,
                operand,
                span,
            } => {
                self.compile_expression(operand)?;
                match operator {
                    UnaryOperator::Negate => {
                        let value = self.store_new();
                        self.push_vm();
                        self.instruction(Instruction::LocalGet(value));
                        self.core_call("__exs_core_negate", *span)?;
                    }
                    UnaryOperator::Not => {
                        self.condition();
                        self.instruction(Instruction::I32Eqz);
                        immediate::bool_from_condition(&mut self.function);
                    }
                }
            }
            Expression::Binary {
                operator,
                left,
                right,
                span,
            } => self.compile_binary(*operator, left, right, *span)?,
            Expression::Call {
                callee,
                arguments,
                span,
            } => self.compile_call(&callee.name, arguments, *span)?,
            other => {
                return Err(unsupported(expression_span(other), "these expressions"));
            }
        }
        Ok(())
    }

    /// Pushes one Int literal, boxing it outside the `i32` range.
    fn compile_integer(
        &mut self,
        value: i128,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let value = i64::try_from(value).map_err(|_| {
            error(
                "E0003",
                span,
                "integer literal is outside the ExS signed 64-bit range".to_owned(),
            )
        })?;
        if let Some(immediate) = exs_value::ValueRef::small_int(value) {
            immediate::push(&mut self.function, immediate);
            return Ok(());
        }
        self.push_vm();
        self.instruction(Instruction::I64Const(value));
        self.core_call("__exs_core_int", span)
    }

    /// Compiles one binary operator.
    fn compile_binary(
        &mut self,
        operator: BinaryOperator,
        left: &Expression<'a>,
        right: &Expression<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if matches!(operator, BinaryOperator::And | BinaryOperator::Or) {
            return self.compile_logical(operator, left, right);
        }
        self.compile_expression(left)?;
        let left = self.store_new();
        self.compile_expression(right)?;
        let right = self.store_new();
        if let Some(operation) = IntOperation::from_binary(operator) {
            immediate::both_small_ints_test(&mut self.function, left, right);
            self.instruction(Instruction::If(BlockType::Result(VALUE_TYPE)));
            let scratch = self.scratch;
            operation.emit(
                &mut self.function,
                left,
                right,
                scratch,
                self.context.int_new,
            );
            self.instruction(Instruction::Else);
            self.compile_slow_binary(operator, left, right, span)?;
            self.instruction(Instruction::End);
        } else {
            self.compile_slow_binary(operator, left, right, span)?;
        }
        Ok(())
    }

    /// Calls the core library for one binary operator on two rooted operand locals.
    fn compile_slow_binary(
        &mut self,
        operator: BinaryOperator,
        left: u32,
        right: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let arithmetic = |code: i32| {
            move |compiler: &mut Self| -> Result<(), CompileDiagnostics<'a>> {
                compiler.push_vm();
                compiler.instruction(Instruction::I32Const(code));
                compiler.instruction(Instruction::LocalGet(left));
                compiler.instruction(Instruction::LocalGet(right));
                compiler.core_call("__exs_core_arithmetic", span)
            }
        };
        let comparison = |code: i32| {
            move |compiler: &mut Self| -> Result<(), CompileDiagnostics<'a>> {
                compiler.instruction(Instruction::I32Const(code));
                compiler.instruction(Instruction::LocalGet(left));
                compiler.instruction(Instruction::LocalGet(right));
                compiler.core_call("__exs_core_compare", span)?;
                immediate::bool_from_condition(&mut compiler.function);
                Ok(())
            }
        };
        match operator {
            BinaryOperator::Add => arithmetic(ADD)(self),
            BinaryOperator::Subtract => arithmetic(SUBTRACT)(self),
            BinaryOperator::Multiply => arithmetic(MULTIPLY)(self),
            BinaryOperator::Divide => {
                self.push_vm();
                self.instruction(Instruction::LocalGet(left));
                self.instruction(Instruction::LocalGet(right));
                self.core_call("__exs_core_divide", span)
            }
            BinaryOperator::LessThan => comparison(0)(self),
            BinaryOperator::LessOrEqual => comparison(1)(self),
            BinaryOperator::GreaterThan => comparison(2)(self),
            BinaryOperator::GreaterOrEqual => comparison(3)(self),
            BinaryOperator::Equal | BinaryOperator::NotEqual => {
                self.instruction(Instruction::LocalGet(left));
                self.instruction(Instruction::LocalGet(right));
                self.core_call("__exs_core_equal", span)?;
                if matches!(operator, BinaryOperator::NotEqual) {
                    self.instruction(Instruction::I32Eqz);
                }
                immediate::bool_from_condition(&mut self.function);
                Ok(())
            }
            BinaryOperator::And | BinaryOperator::Or => Err(error(
                "E0999",
                span,
                "logical operator reached arithmetic lowering".to_owned(),
            )),
        }
    }

    /// Compiles short-circuiting `&&` or `||`; both operands must be Bools.
    fn compile_logical(
        &mut self,
        operator: BinaryOperator,
        left: &Expression<'a>,
        right: &Expression<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let is_or = matches!(operator, BinaryOperator::Or);
        self.compile_expression(left)?;
        self.condition();
        self.instruction(Instruction::If(BlockType::Result(VALUE_TYPE)));
        if is_or {
            immediate::push(&mut self.function, exs_value::ValueRef::TRUE);
        } else {
            self.checked_boolean(right)?;
        }
        self.instruction(Instruction::Else);
        if is_or {
            self.checked_boolean(right)?;
        } else {
            immediate::push(&mut self.function, exs_value::ValueRef::FALSE);
        }
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Compiles one expression that must produce a Bool and leaves it on the stack.
    fn checked_boolean(
        &mut self,
        expression: &Expression<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(expression)?;
        let value = self.store_new();
        self.instruction(Instruction::LocalGet(value));
        self.condition();
        self.instruction(Instruction::Drop);
        self.instruction(Instruction::LocalGet(value));
        Ok(())
    }

    /// Compiles one direct call of a compiled source function.
    fn compile_call(
        &mut self,
        callee: &str,
        arguments: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if self.lookup(callee).is_some() {
            return Err(unsupported(span, "calls of local bindings"));
        }
        let Some((index, arity)) = self
            .context
            .functions
            .get(callee)
            .map(|info| (info.index, info.arity))
        else {
            return Err(unsupported(span, "calls of built-in and prelude functions"));
        };
        if arity as usize != arguments.len() {
            return Err(error(
                "E0208",
                span,
                format!(
                    "function `{callee}` expects {arity} arguments but received {}",
                    arguments.len()
                ),
            ));
        }
        let mut locals = Vec::with_capacity(arguments.len());
        for argument in arguments {
            self.compile_expression(argument)?;
            locals.push(self.store_new());
        }
        for local in locals {
            self.instruction(Instruction::LocalGet(local));
        }
        self.instruction(Instruction::Call(index));
        Ok(())
    }
}

/// Returns the source span of one expression.
fn expression_span<'a>(expression: &Expression<'a>) -> SourceSpan<'a> {
    match expression {
        Expression::Integer(_, span)
        | Expression::Float(_, span)
        | Expression::String(_, span)
        | Expression::Bytes(_, span)
        | Expression::Bool(_, span)
        | Expression::None(span) => *span,
        Expression::Variable(identifier) => identifier.span,
        Expression::FormattedString { span, .. }
        | Expression::IsType { span, .. }
        | Expression::Propagate { span, .. }
        | Expression::List { span, .. }
        | Expression::Object { span, .. }
        | Expression::TypedObject { span, .. }
        | Expression::Match { span, .. }
        | Expression::Closure { span, .. }
        | Expression::ParallelStatic { span, .. }
        | Expression::ParallelDynamic { span, .. }
        | Expression::Unary { span, .. }
        | Expression::Binary { span, .. }
        | Expression::Call { span, .. }
        | Expression::HostCall { span, .. }
        | Expression::HostStream { span, .. }
        | Expression::HostTime { span, .. }
        | Expression::MethodCall { span, .. }
        | Expression::StaticMethodCall { span, .. }
        | Expression::Index { span, .. }
        | Expression::Property { span, .. } => *span,
    }
}
