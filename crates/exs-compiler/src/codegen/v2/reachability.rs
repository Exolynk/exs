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
        visit_block(&function.body, &mut |expression| {
            if let Expression::Call { callee, .. } = expression {
                pending.push(callee.name.as_str());
            }
        });
    }
    module
        .functions
        .iter()
        .filter(|function| reached.contains(function.name.name.as_str()))
        .collect()
}

/// Returns every distinct String literal and formatted-string text of `functions`.
///
/// Formatted strings start from the empty String, so it is always included.
pub(super) fn string_literals(functions: &[&FunctionDeclaration<'_>]) -> Vec<String> {
    let mut literals = vec![String::new()];
    let mut add = |text: &str| {
        if !literals.iter().any(|literal| literal == text) {
            literals.push(text.to_owned());
        }
    };
    for function in functions {
        visit_block(&function.body, &mut |expression| match expression {
            Expression::String(text, _) => add(text),
            Expression::FormattedString { parts, .. } => {
                for part in parts {
                    if let FormattedStringPart::Text(text) = part {
                        add(text);
                    }
                }
            }
            _ => {}
        });
    }
    literals
}

/// Calls `visit` for every expression inside `block`, outer expressions first.
fn visit_block<'e, 'a>(block: &'e Block<'a>, visit: &mut dyn FnMut(&'e Expression<'a>)) {
    for statement in &block.statements {
        visit_statement(statement, visit);
    }
}

/// Calls `visit` for every expression inside `statement`.
fn visit_statement<'e, 'a>(
    statement: &'e Statement<'a>,
    visit: &mut dyn FnMut(&'e Expression<'a>),
) {
    match statement {
        Statement::Let { value, .. }
        | Statement::Expression {
            expression: value, ..
        } => {
            visit_expression(value, visit);
        }
        Statement::Assign { target, value, .. } => {
            match target {
                AssignmentTarget::Variable(_) => {}
                AssignmentTarget::Index {
                    receiver, index, ..
                } => {
                    visit_expression(receiver, visit);
                    visit_expression(index, visit);
                }
                AssignmentTarget::Property { receiver, .. } => visit_expression(receiver, visit),
            }
            visit_expression(value, visit);
        }
        Statement::Return { value, .. } => {
            if let Some(value) = value {
                visit_expression(value, visit);
            }
        }
        Statement::Block { block, .. } => visit_block(block, visit),
        Statement::If {
            condition,
            then_block,
            else_branch,
            ..
        } => {
            visit_expression(condition, visit);
            visit_block(then_block, visit);
            match else_branch {
                Some(ElseBranch::Block(block)) => visit_block(block, visit),
                Some(ElseBranch::If(statement)) => visit_statement(statement, visit),
                None => {}
            }
        }
        Statement::While {
            condition, body, ..
        } => {
            visit_expression(condition, visit);
            visit_block(body, visit);
        }
        Statement::For { iterable, body, .. } => {
            visit_expression(iterable, visit);
            visit_block(body, visit);
        }
        Statement::Break { .. } | Statement::Continue { .. } => {}
    }
}

/// Calls `visit` for `expression` and every expression inside it.
fn visit_expression<'e, 'a>(
    expression: &'e Expression<'a>,
    visit: &mut dyn FnMut(&'e Expression<'a>),
) {
    visit(expression);
    let all = |expressions: &'e [Expression<'a>], visit: &mut dyn FnMut(&'e Expression<'a>)| {
        for expression in expressions {
            visit_expression(expression, visit);
        }
    };
    match expression {
        Expression::Call { arguments, .. } => all(arguments, visit),
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
                    visit_expression(expression, visit);
                }
            }
        }
        Expression::IsType { value, .. }
        | Expression::Propagate { value, .. }
        | Expression::Unary { operand: value, .. } => visit_expression(value, visit),
        Expression::Binary { left, right, .. } => {
            visit_expression(left, visit);
            visit_expression(right, visit);
        }
        Expression::List { elements, .. } => all(elements, visit),
        Expression::Object { properties, .. } | Expression::TypedObject { properties, .. } => {
            for property in properties {
                visit_expression(&property.value, visit);
            }
        }
        Expression::Match { value, arms, .. } => {
            visit_expression(value, visit);
            for arm in arms {
                match &arm.body {
                    MatchArmBody::Expression(value) => visit_expression(value, visit),
                    MatchArmBody::Block(block) => visit_block(block, visit),
                }
            }
        }
        Expression::Closure { body, .. } => visit_block(body, visit),
        Expression::ParallelStatic { tasks, .. } => all(tasks, visit),
        Expression::ParallelDynamic { functions, .. } => visit_expression(functions, visit),
        Expression::HostCall {
            name, arguments, ..
        } => {
            visit_expression(name, visit);
            all(arguments, visit);
        }
        Expression::HostStream { arguments, .. }
        | Expression::HostTime { arguments, .. }
        | Expression::StaticMethodCall { arguments, .. } => all(arguments, visit),
        Expression::MethodCall {
            receiver,
            arguments,
            ..
        } => {
            visit_expression(receiver, visit);
            all(arguments, visit);
        }
        Expression::Index {
            receiver, index, ..
        } => {
            visit_expression(receiver, visit);
            visit_expression(index, visit);
        }
        Expression::Property { receiver, .. } => visit_expression(receiver, visit),
    }
}
