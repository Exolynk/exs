//! Small source-lowering helpers.

use crate::ast::{BinaryOperator, Expression};
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
