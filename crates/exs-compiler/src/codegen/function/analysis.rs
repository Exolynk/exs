//! Small source-lowering helpers.

use crate::ast::{BinaryOperator, Expression, FormattedStringPart};
use crate::diagnostic::SourceSpan;

/// Maps a source binary operator to its runtime ABI operation name.
pub(in crate::codegen::function) fn runtime_operation(operator: BinaryOperator) -> &'static str {
    match operator {
        BinaryOperator::Add => "__exs_rt_add",
        BinaryOperator::Subtract => "__exs_rt_sub",
        BinaryOperator::Multiply => "__exs_rt_mul",
        BinaryOperator::Divide => "__exs_rt_div",
        BinaryOperator::Equal => "__exs_rt_eq",
        BinaryOperator::NotEqual => "__exs_rt_ne",
        BinaryOperator::LessThan => "__exs_rt_lt",
        BinaryOperator::LessOrEqual => "__exs_rt_le",
        BinaryOperator::GreaterThan => "__exs_rt_gt",
        BinaryOperator::GreaterOrEqual => "__exs_rt_ge",
        BinaryOperator::And | BinaryOperator::Or => unreachable!(),
    }
}

/// Returns the source span used for a runtime condition check.
pub(in crate::codegen::function) fn condition_span<'a>(
    expression: &Expression<'a>,
) -> SourceSpan<'a> {
    match expression {
        Expression::Integer(_, span)
        | Expression::Float(_, span)
        | Expression::String(_, span)
        | Expression::Bytes(_, span)
        | Expression::Bool(_, span)
        | Expression::None(span) => *span,
        Expression::FormattedString { span, .. } => *span,
        Expression::List { span, .. } => *span,
        Expression::Object { span, .. } => *span,
        Expression::TypedObject { span, .. } => *span,
        Expression::Variable(identifier) => identifier.span,
        Expression::Closure { span, .. } => *span,
        Expression::ParallelStatic { span, .. } | Expression::ParallelDynamic { span, .. } => *span,
        Expression::Unary { span, .. }
        | Expression::IsType { span, .. }
        | Expression::Propagate { span, .. }
        | Expression::Binary { span, .. }
        | Expression::Call { span, .. }
        | Expression::HostCall { span, .. }
        | Expression::HostStream { span, .. }
        | Expression::HostTime { span, .. }
        | Expression::MethodCall { span, .. }
        | Expression::StaticMethodCall { span, .. }
        | Expression::Index { span, .. }
        | Expression::Property { span, .. } => *span,
        Expression::Match { span, .. } => *span,
    }
}

/// Returns whether evaluating `expression` can run statements of the current function.
///
/// Only `match` block arms embed statements in an expression, so only they can rebind a
/// variable while a surrounding expression is evaluated. Closures count too, conservatively.
pub(in crate::codegen::function) fn contains_statements(expression: &Expression<'_>) -> bool {
    let any = |expressions: &[Expression<'_>]| expressions.iter().any(contains_statements);
    match expression {
        Expression::Match { .. } | Expression::Closure { .. } => true,
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::String(_, _)
        | Expression::Bytes(_, _)
        | Expression::Bool(_, _)
        | Expression::None(_)
        | Expression::Variable(_) => false,
        Expression::FormattedString { parts, .. } => parts.iter().any(|part| {
            matches!(part, FormattedStringPart::Expression(expression) if contains_statements(expression))
        }),
        Expression::List { elements, .. } => any(elements),
        Expression::Object { properties, .. } | Expression::TypedObject { properties, .. } => {
            properties
                .iter()
                .any(|property| contains_statements(&property.value))
        }
        Expression::IsType { value, .. }
        | Expression::Propagate { value, .. }
        | Expression::Unary { operand: value, .. } => contains_statements(value),
        Expression::Binary { left, right, .. } => {
            contains_statements(left) || contains_statements(right)
        }
        Expression::Call { arguments, .. }
        | Expression::HostStream { arguments, .. }
        | Expression::HostTime { arguments, .. }
        | Expression::StaticMethodCall { arguments, .. } => any(arguments),
        Expression::HostCall {
            name, arguments, ..
        } => contains_statements(name) || any(arguments),
        Expression::MethodCall {
            receiver,
            arguments,
            ..
        } => contains_statements(receiver) || any(arguments),
        Expression::ParallelStatic { tasks, .. } => any(tasks),
        Expression::ParallelDynamic { functions, .. } => contains_statements(functions),
        Expression::Index {
            receiver, index, ..
        } => contains_statements(receiver) || contains_statements(index),
        Expression::Property { receiver, .. } => contains_statements(receiver),
    }
}
