//! Expression lowering of the v2 backend.

use wasm_encoder::{BlockType, Instruction};

use super::function::{FunctionCompiler, error, unsupported};
use crate::ast::{BinaryOperator, Expression, FormattedStringPart, UnaryOperator};
use crate::codegen::VALUE_TYPE;
use crate::codegen::immediate::{self, IntOperation};
use crate::diagnostic::{CompileDiagnostics, SourceSpan};
use exs_abi::{TYPE_FLOAT, TYPE_STRING};
use exs_value::layout::{KIND_SHIFT, type_kind};
use wasm_encoder::MemArg;

/// Core code of `-` for `__exs_core_arithmetic`.
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
            Expression::Float(value, _) => {
                self.instruction(Instruction::I64Const(value.to_bits().cast_signed()));
                self.instruction(Instruction::Call(self.context.float_new));
            }
            Expression::String(text, span) => self.push_string(text, *span)?,
            Expression::FormattedString { parts, span, .. } => {
                let mut values = vec![self.static_string("", *span)?];
                for part in parts {
                    match part {
                        FormattedStringPart::Text(text) => {
                            self.push_string(text, *span)?;
                            values.push(self.store_new());
                        }
                        FormattedStringPart::Expression(expression) => {
                            self.compile_expression(expression)?;
                            values.push(self.store_new());
                        }
                    }
                }
                self.concat_values(&values, *span)?;
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
        if matches!(operator, BinaryOperator::Add) {
            let mut operands = vec![right];
            let mut leftmost = left;
            while let Expression::Binary {
                operator: BinaryOperator::Add,
                left,
                right,
                ..
            } = leftmost
            {
                operands.push(right);
                leftmost = left;
            }
            operands.push(leftmost);
            operands.reverse();
            let mut values = Vec::with_capacity(operands.len());
            for operand in operands {
                self.compile_expression(operand)?;
                values.push(self.store_new());
            }
            return self.add_values(&values, span);
        }
        self.compile_expression(left)?;
        let left = self.store_new();
        self.compile_expression(right)?;
        let right = self.store_new();
        self.int_or_slow_binary(operator, left, right, span)
    }

    /// Emits `operator` with an inline path for two immediate Ints, then one for two Floats,
    /// and the core library for every other operand pair.
    fn int_or_slow_binary(
        &mut self,
        operator: BinaryOperator,
        left: u32,
        right: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
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
            self.float_or_slow_binary(operator, left, right, span)?;
            self.instruction(Instruction::End);
            Ok(())
        } else {
            self.float_or_slow_binary(operator, left, right, span)
        }
    }

    /// Emits the inline Float path of `operator`, if it has one, with the core as fallback.
    fn float_or_slow_binary(
        &mut self,
        operator: BinaryOperator,
        left: u32,
        right: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        match FloatOperation::from_binary(operator) {
            Some(float) => self.float_fast_path(float, left, right, |compiler| {
                compiler.compile_slow_binary(operator, left, right, span)
            }),
            None => self.compile_slow_binary(operator, left, right, span),
        }
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
            BinaryOperator::Add => self.concat_pair(left, right, span),
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

/// A binary operator with an inline path for two Float operands.
#[derive(Clone, Copy)]
enum FloatOperation {
    /// `+`
    Add,
    /// `-`
    Subtract,
    /// `*`
    Multiply,
    /// `/`
    Divide,
}

impl FloatOperation {
    /// Returns the inline Float operation of one source operator, if it has one.
    const fn from_binary(operator: BinaryOperator) -> Option<Self> {
        match operator {
            BinaryOperator::Add => Some(Self::Add),
            BinaryOperator::Subtract => Some(Self::Subtract),
            BinaryOperator::Multiply => Some(Self::Multiply),
            BinaryOperator::Divide => Some(Self::Divide),
            _ => None,
        }
    }

    /// Returns the Wasm instruction combining two `f64` values.
    const fn instruction(self) -> Instruction<'static> {
        match self {
            Self::Add => Instruction::F64Add,
            Self::Subtract => Instruction::F64Sub,
            Self::Multiply => Instruction::F64Mul,
            Self::Divide => Instruction::F64Div,
        }
    }
}

impl<'a> FunctionCompiler<'a, '_> {
    /// Pushes the static String of one literal.
    fn push_string(
        &mut self,
        text: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let raw =
            self.context.strings.get(text).copied().ok_or_else(|| {
                error("E0999", span, "missing v2 static String literal".to_owned())
            })?;
        self.instruction(Instruction::I64Const(raw.cast_signed()));
        Ok(())
    }

    /// Stores the static String of one literal in a new rooted local.
    fn static_string(
        &mut self,
        text: &str,
        span: SourceSpan<'a>,
    ) -> Result<u32, CompileDiagnostics<'a>> {
        self.push_string(text, span)?;
        Ok(self.store_new())
    }

    /// Emits `operation` inline for two Float operands and `otherwise` for anything else.
    fn float_fast_path(
        &mut self,
        operation: FloatOperation,
        left: u32,
        right: u32,
        otherwise: impl FnOnce(&mut Self) -> Result<(), CompileDiagnostics<'a>>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let float_kind = i64::from(type_kind(TYPE_FLOAT));
        for local in [left, right] {
            self.instruction(Instruction::LocalGet(local));
            self.instruction(Instruction::I64Const(i64::from(KIND_SHIFT)));
            self.instruction(Instruction::I64ShrU);
            self.instruction(Instruction::I64Const(float_kind));
            self.instruction(Instruction::I64Eq);
        }
        self.instruction(Instruction::I32And);
        self.instruction(Instruction::If(BlockType::Result(VALUE_TYPE)));
        for local in [left, right] {
            self.instruction(Instruction::LocalGet(local));
            self.instruction(Instruction::I32WrapI64);
            self.instruction(Instruction::F64Load(MemArg {
                offset: FLOAT_VALUE,
                align: 3,
                memory_index: 0,
            }));
        }
        self.instruction(operation.instruction());
        self.instruction(Instruction::I64ReinterpretF64);
        self.instruction(Instruction::Call(self.context.float_new));
        self.instruction(Instruction::Else);
        otherwise(self)?;
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Adds the values of `values` left to right with the language's `+`.
    ///
    /// A String on the left joins the whole chain with one allocation; otherwise every `+`
    /// runs in turn with its inline Int and Float paths.
    fn add_values(
        &mut self,
        values: &[u32],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let [first, rest @ ..] = values else {
            return Err(error("E0999", span, "empty `+` chain".to_owned()));
        };
        if let [second] = rest {
            return self.add_pair(*first, *second, span);
        }
        self.instruction(Instruction::LocalGet(*first));
        immediate::type_mask_test(&mut self.function, TYPE_STRING);
        self.instruction(Instruction::If(BlockType::Result(VALUE_TYPE)));
        self.concat_values(values, span)?;
        self.instruction(Instruction::Else);
        let mut accumulated = *first;
        for value in rest {
            self.add_pair(accumulated, *value, span)?;
            accumulated = self.store_new();
        }
        self.instruction(Instruction::LocalGet(accumulated));
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Emits one `+` of two rooted locals with inline Int and Float paths.
    fn add_pair(
        &mut self,
        left: u32,
        right: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.int_or_slow_binary(BinaryOperator::Add, left, right, span)
    }

    /// Calls the core `+` for two rooted locals through the shared operand pair.
    fn concat_pair(
        &mut self,
        left: u32,
        right: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let pair = match self.pair {
            Some(pair) => pair,
            None => {
                let pair = self.consecutive_locals(2);
                self.pair = Some(pair);
                pair
            }
        };
        self.instruction(Instruction::LocalGet(left));
        self.store(pair);
        self.instruction(Instruction::LocalGet(right));
        self.store(pair + 1);
        self.push_vm();
        self.push_slot_address(pair);
        self.instruction(Instruction::I32Const(2));
        self.core_call("__exs_core_concat", span)
    }

    /// Joins `values` with the language's `+` in one core call over consecutive root slots.
    fn concat_values(
        &mut self,
        values: &[u32],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let count = u32::try_from(values.len())
            .map_err(|_| error("E0212", span, "too many `+` operands".to_owned()))?;
        let block = self.consecutive_locals(count);
        for (offset, value) in (0..count).zip(values) {
            self.instruction(Instruction::LocalGet(*value));
            self.store(block + offset);
        }
        self.push_vm();
        self.push_slot_address(block);
        self.instruction(Instruction::I32Const(count.cast_signed()));
        self.core_call("__exs_core_concat", span)
    }
}

/// Byte offset of the value inside a Float object.
const FLOAT_VALUE: u64 = core::mem::offset_of!(exs_layout::FloatObject, value) as u64;

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
