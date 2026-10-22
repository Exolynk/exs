//! Small source-lowering helpers.

use crate::ast::{Expression, FormattedStringPart};

/// Returns whether evaluating `expression` can run statements of the current function.
///
/// Only `match` block arms embed statements in an expression, so only they can rebind a
/// variable while a surrounding expression is evaluated. Closures count too, conservatively.
pub(in crate::codegen) fn contains_statements(expression: &Expression<'_>) -> bool {
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
