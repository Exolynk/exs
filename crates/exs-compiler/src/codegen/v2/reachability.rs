//! The functions a v2 module must contain.

use std::collections::{HashMap, HashSet};

use crate::ast::{
    AssignmentTarget, Block, ElseBranch, Expression, FormattedStringPart, FunctionDeclaration,
    FunctionVisibility, MatchArmBody, Module, Statement,
};

/// Returns the public functions and every function they can call directly, in source order.
pub(super) fn reachable_functions<'m, 'a>(
    module: &'m Module<'a>,
) -> Vec<&'m FunctionDeclaration<'a>> {
    let by_name: HashMap<&str, &FunctionDeclaration<'a>> = module
        .functions
        .iter()
        .map(|function| (function.name.name.as_str(), function))
        .collect();
    let mut reached = HashSet::new();
    let mut pending: Vec<&str> = module
        .functions
        .iter()
        .filter(|function| function.visibility == FunctionVisibility::Public)
        .map(|function| function.name.name.as_str())
        .collect();
    while let Some(name) = pending.pop() {
        let Some(function) = by_name.get(name) else {
            continue;
        };
        if !reached.insert(name) {
            continue;
        }
        let mut callees = Vec::new();
        block_calls(&function.body, &mut callees);
        pending.extend(callees);
    }
    module
        .functions
        .iter()
        .filter(|function| reached.contains(function.name.name.as_str()))
        .collect()
}

/// Appends the callee names of every direct call inside `block`.
fn block_calls<'e>(block: &'e Block<'_>, callees: &mut Vec<&'e str>) {
    for statement in &block.statements {
        statement_calls(statement, callees);
    }
}

/// Appends the callee names of every direct call inside `statement`.
fn statement_calls<'e>(statement: &'e Statement<'_>, callees: &mut Vec<&'e str>) {
    match statement {
        Statement::Let { value, .. }
        | Statement::Expression {
            expression: value, ..
        } => {
            expression_calls(value, callees);
        }
        Statement::Assign { target, value, .. } => {
            match target {
                AssignmentTarget::Variable(_) => {}
                AssignmentTarget::Index {
                    receiver, index, ..
                } => {
                    expression_calls(receiver, callees);
                    expression_calls(index, callees);
                }
                AssignmentTarget::Property { receiver, .. } => expression_calls(receiver, callees),
            }
            expression_calls(value, callees);
        }
        Statement::Return { value, .. } => {
            if let Some(value) = value {
                expression_calls(value, callees);
            }
        }
        Statement::Block { block, .. } => block_calls(block, callees),
        Statement::If {
            condition,
            then_block,
            else_branch,
            ..
        } => {
            expression_calls(condition, callees);
            block_calls(then_block, callees);
            match else_branch {
                Some(ElseBranch::Block(block)) => block_calls(block, callees),
                Some(ElseBranch::If(statement)) => statement_calls(statement, callees),
                None => {}
            }
        }
        Statement::While {
            condition, body, ..
        } => {
            expression_calls(condition, callees);
            block_calls(body, callees);
        }
        Statement::For { iterable, body, .. } => {
            expression_calls(iterable, callees);
            block_calls(body, callees);
        }
        Statement::Break { .. } | Statement::Continue { .. } => {}
    }
}

/// Appends the callee names of every direct call inside `expression`.
fn expression_calls<'e>(expression: &'e Expression<'_>, callees: &mut Vec<&'e str>) {
    let all = |expressions: &'e [Expression<'_>], callees: &mut Vec<&'e str>| {
        for expression in expressions {
            expression_calls(expression, callees);
        }
    };
    match expression {
        Expression::Call {
            callee, arguments, ..
        } => {
            callees.push(callee.name.as_str());
            all(arguments, callees);
        }
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::String(_, _)
        | Expression::Bytes(_, _)
        | Expression::Bool(_, _)
        | Expression::None(_)
        | Expression::Variable(_) => {}
        Expression::FormattedString { parts, .. } => {
            for part in parts {
                if let FormattedStringPart::Expression(expression) = part {
                    expression_calls(expression, callees);
                }
            }
        }
        Expression::IsType { value, .. }
        | Expression::Propagate { value, .. }
        | Expression::Unary { operand: value, .. } => expression_calls(value, callees),
        Expression::Binary { left, right, .. } => {
            expression_calls(left, callees);
            expression_calls(right, callees);
        }
        Expression::List { elements, .. } => all(elements, callees),
        Expression::Object { properties, .. } | Expression::TypedObject { properties, .. } => {
            for property in properties {
                expression_calls(&property.value, callees);
            }
        }
        Expression::Match { value, arms, .. } => {
            expression_calls(value, callees);
            for arm in arms {
                match &arm.body {
                    MatchArmBody::Expression(value) => expression_calls(value, callees),
                    MatchArmBody::Block(block) => block_calls(block, callees),
                }
            }
        }
        Expression::Closure { body, .. } => block_calls(body, callees),
        Expression::ParallelStatic { tasks, .. } => all(tasks, callees),
        Expression::ParallelDynamic { functions, .. } => expression_calls(functions, callees),
        Expression::HostCall {
            name, arguments, ..
        } => {
            expression_calls(name, callees);
            all(arguments, callees);
        }
        Expression::HostStream { arguments, .. }
        | Expression::HostTime { arguments, .. }
        | Expression::StaticMethodCall { arguments, .. } => all(arguments, callees),
        Expression::MethodCall {
            receiver,
            arguments,
            ..
        } => {
            expression_calls(receiver, callees);
            all(arguments, callees);
        }
        Expression::Index {
            receiver, index, ..
        } => {
            expression_calls(receiver, callees);
            expression_calls(index, callees);
        }
        Expression::Property { receiver, .. } => expression_calls(receiver, callees),
    }
}
