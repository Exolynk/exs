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
                self.compile_formatted(parts, *span)?
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
                // Like v1, a negated Int literal is one constant, which also admits i64::MIN.
                if matches!(operator, UnaryOperator::Negate)
                    && let Expression::Integer(value, operand_span) = operand.as_ref()
                {
                    let negative = value.checked_neg().ok_or_else(|| {
                        error(
                            "E0206",
                            *operand_span,
                            "integer literal is outside the ExS signed 64-bit range".to_owned(),
                        )
                    })?;
                    return self.compile_integer(negative, *operand_span);
                }
                self.compile_expression(operand)?;
                match operator {
                    UnaryOperator::Negate => {
                        let value = self.store_new();
                        self.set_position(*span)?;
                        self.push_vm();
                        self.instruction(Instruction::LocalGet(value));
                        self.core_call("__exs_core_negate", *span)?;
                    }
                    UnaryOperator::Not => self.compile_not(*span)?,
                }
            }
            Expression::Propagate { value, span } => self.compile_propagate(value, *span)?,
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
            } if callee.name == "Error" => self.compile_error_constructor(arguments, *span)?,
            Expression::Call {
                callee,
                arguments,
                span,
            } => self.compile_call(&callee.name, arguments, *span)?,
            Expression::List { elements, span } => self.compile_list(elements, *span)?,
            Expression::Object { properties, span } => self.compile_object(properties, *span)?,
            Expression::TypedObject {
                type_name,
                properties,
                span,
            } => self.compile_typed_object(type_name, properties, *span)?,
            Expression::Index {
                receiver,
                index,
                span,
            } => self.compile_index_get(receiver, index, *span)?,
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
            } => self.compile_static_call(type_name, method, arguments, *span)?,
            Expression::Property {
                receiver,
                property,
                span,
            } => self.compile_property_get(receiver, &property.name, *span)?,
            other => {
                return Err(unsupported(
                    expression_span(other),
                    unsupported_expression(other),
                ));
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
            return self.compile_logical(operator, left, right, span);
        }
        if matches!(operator, BinaryOperator::Add) {
            // Each `+` keeps the span of its own Binary node, the origin v1 reports for it.
            let mut operands = vec![right];
            let mut spans = vec![span];
            let mut leftmost = left;
            while let Expression::Binary {
                operator: BinaryOperator::Add,
                left,
                right,
                span,
            } = leftmost
            {
                operands.push(right);
                spans.push(*span);
                leftmost = left;
            }
            operands.push(leftmost);
            operands.reverse();
            spans.reverse();
            let mut values = Vec::with_capacity(operands.len());
            for operand in operands {
                self.compile_expression(operand)?;
                values.push(self.store_new());
            }
            return self.add_values(&values, &spans, span);
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
        self.guard_operator(operator, left);
        let arithmetic = |code: i32| {
            move |compiler: &mut Self| -> Result<(), CompileDiagnostics<'a>> {
                compiler.set_position(span)?;
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
                self.set_position(span)?;
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
    ///
    /// Like v1, the left condition reports the whole expression's span and the right one its
    /// own span.
    fn compile_logical(
        &mut self,
        operator: BinaryOperator,
        left: &Expression<'a>,
        right: &Expression<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let is_or = matches!(operator, BinaryOperator::Or);
        self.compile_expression(left)?;
        self.condition(span)?;
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
        self.condition(expression_span(expression))?;
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
        self.set_call_site(span)?;
        self.instruction(Instruction::Call(index));
        let result = self.store_new();
        self.return_if_fatal(result);
        self.instruction(Instruction::LocalGet(result));
        Ok(())
    }

    /// Compiles a formatted string like v1: text parts and expression parts in order, every
    /// value rendered with its default text, which never fails.
    fn compile_formatted(
        &mut self,
        parts: &[FormattedStringPart<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let count = u32::try_from(parts.len())
            .map_err(|_| error("E0212", span, "too many formatted-string parts".to_owned()))?;
        let block = self.consecutive_locals(count.max(1));
        for (offset, part) in (0..count).zip(parts) {
            match part {
                FormattedStringPart::Text(text) => self.push_string(text, span)?,
                FormattedStringPart::Expression(expression) => {
                    self.compile_expression(expression)?;
                }
            }
            self.store(block + offset);
            if matches!(part, FormattedStringPart::Expression(_)) {
                self.guard_to_string(block + offset);
            }
        }
        self.push_vm();
        self.push_slot_address(block);
        self.instruction(Instruction::I32Const(count.cast_signed()));
        self.core_call("__exs_core_format", span)
    }

    /// Compiles `!` on the `ValueRef` on the stack: a Bool flips inline, any other value is a
    /// TypeError value.
    fn compile_not(&mut self, span: SourceSpan<'a>) -> Result<(), CompileDiagnostics<'a>> {
        let value = self.scratch;
        self.instruction(Instruction::LocalTee(value));
        immediate::is_bool_test(&mut self.function);
        self.instruction(Instruction::If(BlockType::Result(VALUE_TYPE)));
        // Bool immediates differ only in their lowest payload bit.
        self.instruction(Instruction::LocalGet(value));
        self.instruction(Instruction::I64Const(1));
        self.instruction(Instruction::I64Xor);
        self.instruction(Instruction::Else);
        self.set_position(span)?;
        self.push_vm();
        self.instruction(Instruction::LocalGet(value));
        self.core_call("__exs_core_not_error", span)?;
        self.instruction(Instruction::End);
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
    /// Returns the raw `ValueRef` of the static String of one literal.
    pub(super) fn static_string_value(
        &self,
        text: &str,
        span: SourceSpan<'a>,
    ) -> Result<u64, CompileDiagnostics<'a>> {
        self.context
            .strings
            .get(text)
            .copied()
            .ok_or_else(|| error("E0999", span, "missing v2 static String literal".to_owned()))
    }

    /// Pushes the static String of one literal.
    fn push_string(
        &mut self,
        text: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let raw = self.static_string_value(text, span)?;
        self.instruction(Instruction::I64Const(raw.cast_signed()));
        Ok(())
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
    ///
    /// `spans` holds the span of each `+`, the origin of an Error that step creates.
    fn add_values(
        &mut self,
        values: &[u32],
        spans: &[SourceSpan<'a>],
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
        self.concat_values(values, Some(spans), span)?;
        self.instruction(Instruction::Else);
        let mut accumulated = *first;
        for (value, step) in rest.iter().zip(spans) {
            self.add_pair(accumulated, *value, *step)?;
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
        self.set_position(span)?;
        self.push_vm();
        self.push_slot_address(pair);
        self.instruction(Instruction::I32Const(2));
        self.instruction(Instruction::I32Const(0));
        self.core_call("__exs_core_concat", span)
    }

    /// Joins `values` with the language's `+` in one core call over consecutive root slots.
    ///
    /// With `spans`, every step takes its Error origin from a static position array; without,
    /// Errors use the position set before the call.
    fn concat_values(
        &mut self,
        values: &[u32],
        spans: Option<&[SourceSpan<'a>]>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let positions = match spans {
            Some(spans) => self.positions_address(spans)?,
            None => 0,
        };
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
        self.instruction(Instruction::I32Const(positions.cast_signed()));
        self.core_call("__exs_core_concat", span)
    }

    /// Appends the source positions of `spans` to the static position arrays and returns the
    /// address of the first one.
    fn positions_address(&self, spans: &[SourceSpan<'a>]) -> Result<u32, CompileDiagnostics<'a>> {
        let mut ids = Vec::with_capacity(spans.len());
        for span in spans {
            ids.push(self.position(*span)?);
        }
        let mut positions = self.context.positions.borrow_mut();
        let address = self.context.positions_base + positions.len() as u32 * 4;
        positions.extend(ids);
        Ok(address)
    }
}

/// Byte offset of the value inside a Float object.
const FLOAT_VALUE: u64 = core::mem::offset_of!(exs_layout::FloatObject, value) as u64;

/// Names the expressions of one kind the v2 backend does not compile yet.
fn unsupported_expression(expression: &Expression<'_>) -> &'static str {
    match expression {
        Expression::Bytes(..) => "Bytes literals",
        Expression::IsType { .. } => "`is` type tests",
        Expression::Match { .. } => "`match` expressions",
        Expression::Closure { .. } => "closures",
        Expression::ParallelStatic { .. } | Expression::ParallelDynamic { .. } => {
            "parallel expressions"
        }
        Expression::HostCall { .. }
        | Expression::HostStream { .. }
        | Expression::HostTime { .. } => "Host operations",
        Expression::MethodCall { .. } => "method calls",
        Expression::StaticMethodCall { .. } => "static method calls",
        _ => "these expressions",
    }
}

/// Returns the source span of one expression.
pub(super) fn expression_span<'a>(expression: &Expression<'a>) -> SourceSpan<'a> {
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
