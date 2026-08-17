//! Local-slot analysis and small source-lowering helpers.

use crate::ast::{AssignmentTarget, BinaryOperator, Block, ElseBranch, Expression, Statement};
use crate::diagnostic::SourceSpan;

/// Extra compiler locals reserved for root-frame return and operand-spill bookkeeping.
pub(super) const ROOT_FRAME_RESERVED_LOCALS: u32 = 8;

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

/// Counts local declarations in one block and nested blocks.
pub(super) fn count_lets(block: &Block<'_>) -> u32 {
    block
        .statements
        .iter()
        .map(|statement| match statement {
            Statement::Let { .. } => 1,
            Statement::Block { block, .. } => count_lets(block),
            Statement::If {
                then_block,
                else_branch,
                ..
            } => {
                count_lets(then_block)
                    + else_branch.as_ref().map_or(0, |branch| match branch {
                        ElseBranch::Block(block) => count_lets(block),
                        ElseBranch::If(statement) => count_lets_statement(statement),
                    })
            }
            Statement::While { body, .. } => count_lets(body),
            Statement::For { body, .. } => 1 + count_lets(body),
            _ => 0,
        })
        .sum()
}

/// Counts local declarations in one statement and nested blocks.
fn count_lets_statement(statement: &Statement<'_>) -> u32 {
    match statement {
        Statement::Let { .. } => 1,
        Statement::Block { block, .. } => count_lets(block),
        Statement::If {
            then_block,
            else_branch,
            ..
        } => {
            count_lets(then_block)
                + else_branch.as_ref().map_or(0, |branch| match branch {
                    ElseBranch::Block(block) => count_lets(block),
                    ElseBranch::If(statement) => count_lets_statement(statement),
                })
        }
        Statement::While { body, .. } => count_lets(body),
        Statement::For { body, .. } => 1 + count_lets(body),
        _ => 0,
    }
}

/// Counts expression scratch-local requirements in one block.
pub(super) fn count_expressions_block(block: &Block<'_>) -> u32 {
    block
        .statements
        .iter()
        .map(count_expressions_statement)
        .sum()
}

/// Counts expression scratch-local requirements in one statement.
pub(super) fn count_expressions_statement(statement: &Statement<'_>) -> u32 {
    match statement {
        Statement::Let { value, .. }
        | Statement::Expression {
            expression: value, ..
        } => count_expressions(value),
        Statement::Assign { target, value, .. } => {
            count_assignment_target_expressions(target) + count_expressions(value)
        }
        Statement::Return { value, .. } => value.as_ref().map_or(0, count_expressions),
        Statement::Block { block, .. } => count_expressions_block(block),
        Statement::If {
            condition,
            then_block,
            else_branch,
            ..
        } => {
            count_expressions(condition)
                + count_expressions_block(then_block)
                + else_branch.as_ref().map_or(0, |branch| match branch {
                    ElseBranch::Block(block) => count_expressions_block(block),
                    ElseBranch::If(statement) => count_expressions_statement(statement),
                })
        }
        Statement::While {
            condition, body, ..
        } => count_expressions(condition) + count_expressions_block(body),
        Statement::For { iterable, body, .. } => {
            6 + count_expressions(iterable) + count_expressions_block(body)
        }
        Statement::Break { .. } | Statement::Continue { .. } => 0,
    }
}

/// Counts scratch-local requirements needed to evaluate one assignment target.
pub(super) fn count_assignment_target_expressions(target: &AssignmentTarget<'_>) -> u32 {
    match target {
        AssignmentTarget::Variable(_) => 0,
        AssignmentTarget::Index {
            receiver, index, ..
        } => count_expressions(receiver) + count_expressions(index),
        AssignmentTarget::Property { receiver, .. } => 1 + count_expressions(receiver),
    }
}

/// Counts expression scratch-local requirements recursively.
pub(super) fn count_expressions(expression: &Expression<'_>) -> u32 {
    match expression {
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::String(_, _)
        | Expression::Bool(_, _)
        | Expression::None(_)
        | Expression::Variable(_) => 1,
        Expression::FormattedString { parts, .. } => {
            2 + parts
                .iter()
                .map(|part| match part {
                    crate::ast::FormattedStringPart::Text(_) => 1,
                    crate::ast::FormattedStringPart::Expression(expression) => {
                        count_expressions(expression)
                    }
                })
                .sum::<u32>()
        }
        Expression::IsError { value, .. } | Expression::Propagate { value, .. } => {
            1 + count_expressions(value)
        }
        Expression::Unary { operand, .. } => 1 + count_expressions(operand),
        Expression::Binary { left, right, .. } => {
            1 + count_expressions(left) + count_expressions(right)
        }
        Expression::Call { arguments, .. } => {
            1 + arguments.iter().map(count_expressions).sum::<u32>()
        }
        Expression::HostCall {
            name, arguments, ..
        } => 1 + count_expressions(name) + arguments.iter().map(count_expressions).sum::<u32>(),
        Expression::List { elements, .. } => {
            1 + elements.iter().map(count_expressions).sum::<u32>()
        }
        Expression::Object { properties, .. } => {
            1 + properties
                .iter()
                .map(|property| 1 + count_expressions(&property.value))
                .sum::<u32>()
        }
        Expression::Match { value, arms, .. } => {
            1 + count_expressions(value)
                + arms
                    .iter()
                    .map(|arm| match &arm.body {
                        crate::ast::MatchArmBody::Expression(value) => count_expressions(value),
                        crate::ast::MatchArmBody::Block(block) => count_expressions_block(block),
                    })
                    .sum::<u32>()
        }
        Expression::TypedObject { properties, .. } => {
            1 + properties
                .iter()
                .map(|property| 1 + count_expressions(&property.value))
                .sum::<u32>()
        }
        Expression::MethodCall {
            receiver,
            arguments,
            ..
        } => 5 + count_expressions(receiver) + arguments.iter().map(count_expressions).sum::<u32>(),
        Expression::StaticMethodCall { arguments, .. } => {
            1 + arguments.iter().map(count_expressions).sum::<u32>()
        }
        Expression::Index {
            receiver, index, ..
        } => 1 + count_expressions(receiver) + count_expressions(index),
        Expression::Property { receiver, .. } => 1 + count_expressions(receiver),
        Expression::Closure { .. } => 1,
        Expression::ParallelStatic { tasks, .. } => {
            1 + tasks.iter().map(count_expressions).sum::<u32>()
        }
        Expression::ParallelDynamic { functions, .. } => 1 + count_expressions(functions),
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
        | Expression::IsError { span, .. }
        | Expression::Propagate { span, .. }
        | Expression::Binary { span, .. }
        | Expression::Call { span, .. }
        | Expression::HostCall { span, .. }
        | Expression::MethodCall { span, .. }
        | Expression::StaticMethodCall { span, .. }
        | Expression::Index { span, .. }
        | Expression::Property { span, .. } => *span,
        Expression::Match { span, .. } => *span,
    }
}
