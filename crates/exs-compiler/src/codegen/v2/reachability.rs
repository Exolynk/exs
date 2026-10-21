//! The functions a v2 module must contain.

use std::collections::{HashMap, HashSet};

use super::closures::closure_span_key;
use super::dispatch::{Dispatch, Target};
use super::intrinsics::{DATETIME_KEYS, is_datetime_parser};
use super::units::Unit;
use crate::ast::{
    AssignmentTarget, Block, ElseBranch, Expression, FormattedStringPart, FunctionDeclaration,
    FunctionVisibility, MatchArmBody, ObjectProperty, Statement,
};
use crate::codegen::trait_registry::TraitOperator;

/// Returns the public functions and every unit they can reach, in trace-id order.
///
/// Calls reach their function, static calls their method, closure expressions their lifted
/// function, and like v1's dispatch, method calls reach every implementation of the method
/// name and the prelude's iterator helper of that name, operators their implementations and
/// formatted strings every `ToString` implementation.
pub(super) fn reachable_units<'u, 'm, 'a>(
    units: &'u [Unit<'m, 'a>],
    dispatch: &Dispatch,
    closures: &HashMap<(String, u32, u32), String>,
) -> Vec<&'u Unit<'m, 'a>> {
    let by_key: HashMap<&str, &Unit<'m, 'a>> =
        units.iter().map(|unit| (unit.key.as_str(), unit)).collect();
    let mut reached = HashSet::new();
    let mut pending: Vec<String> = units
        .iter()
        .filter(|unit| {
            unit.declaration.visibility == FunctionVisibility::Public && !unit.key.contains("::")
        })
        .map(|unit| unit.key.clone())
        .collect();
    while let Some(key) = pending.pop() {
        let Some(unit) = by_key.get(key.as_str()) else {
            continue;
        };
        if !reached.insert(unit.key.as_str()) {
            continue;
        }
        let targets = |targets: &[Target], pending: &mut Vec<String>| {
            pending.extend(targets.iter().map(|target| target.key.clone()));
        };
        visit_block(&unit.declaration.body, &mut |expression| match expression {
            Expression::Call { callee, .. } if !is_datetime_parser(&callee.name) => {
                pending.push(callee.name.clone());
            }
            Expression::StaticMethodCall {
                type_name, method, ..
            } => pending.push(format!("{}::{}", type_name.name, method.name)),
            Expression::MethodCall { method, .. } => {
                targets(dispatch.instance(&method.name), &mut pending);
                if let Some(helper) = crate::prelude::iterator_helper(&method.name) {
                    pending.push(helper.to_owned());
                }
            }
            Expression::Closure { span, .. } => {
                if let Some(key) = closures.get(&closure_span_key(*span)) {
                    pending.push(key.clone());
                }
            }
            Expression::Binary { operator, .. } => {
                if let Some(operator) = TraitOperator::from_binary(*operator) {
                    targets(dispatch.operator(operator), &mut pending);
                }
            }
            Expression::FormattedString { .. } => targets(dispatch.to_string(), &mut pending),
            _ => {}
        });
    }
    units
        .iter()
        .filter(|unit| reached.contains(unit.key.as_str()))
        .collect()
}

/// Returns every distinct String literal, formatted-string text and property name of
/// `functions`, followed by `extra`.
///
/// Formatted strings start from the empty String, so it is always included.
pub(super) fn string_literals<'t>(
    functions: &[&FunctionDeclaration<'_>],
    extra: impl IntoIterator<Item = &'t str>,
) -> Vec<String> {
    let mut literals = vec![String::new()];
    let mut add = |text: &str| {
        if !literals.iter().any(|literal| literal == text) {
            literals.push(text.to_owned());
        }
    };
    for function in functions {
        visit_block_targets(&function.body, &mut |target| {
            if let AssignmentTarget::Property { property, .. } = target {
                add(&property.name);
            }
        });
        visit_block(&function.body, &mut |expression| match expression {
            Expression::String(text, _) => add(text),
            Expression::FormattedString { parts, .. } => {
                for part in parts {
                    if let FormattedStringPart::Text(text) = part {
                        add(text);
                    }
                }
            }
            Expression::Property { property, .. } => add(&property.name),
            Expression::Call { callee, .. } if is_datetime_parser(&callee.name) => {
                for key in DATETIME_KEYS {
                    add(key);
                }
            }
            Expression::Object { properties, .. } => {
                for property in properties {
                    add(&property.key);
                }
            }
            _ => {}
        });
    }
    for text in extra {
        add(text);
    }
    literals
}

/// Returns whether `functions` call `to_lowercase` or `to_uppercase`, which need the case
/// tables.
pub(super) fn changes_case(functions: &[&FunctionDeclaration<'_>]) -> bool {
    let mut found = false;
    for function in functions {
        visit_block(&function.body, &mut |expression| {
            if let Expression::MethodCall { method, .. } = expression {
                found |= matches!(method.name.as_str(), "to_lowercase" | "to_uppercase");
            }
        });
    }
    found
}

/// Returns every distinct Bytes literal of `functions`.
pub(super) fn bytes_literals(functions: &[&FunctionDeclaration<'_>]) -> Vec<String> {
    let mut literals: Vec<String> = Vec::new();
    for function in functions {
        visit_block(&function.body, &mut |expression| {
            if let Expression::Bytes(text, _) = expression
                && !literals.contains(text)
            {
                literals.push(text.clone());
            }
        });
    }
    literals
}

/// Returns the distinct keys of one Object literal in order of first appearance.
pub(super) fn literal_keys(properties: &[ObjectProperty<'_>]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::with_capacity(properties.len());
    for property in properties {
        if !keys.contains(&property.key) {
            keys.push(property.key.clone());
        }
    }
    keys
}

/// Returns every distinct key sequence of the Object literals of `functions`.
pub(super) fn object_key_lists(functions: &[&FunctionDeclaration<'_>]) -> Vec<Vec<String>> {
    let mut lists = Vec::new();
    for function in functions {
        visit_block(&function.body, &mut |expression| {
            let keys = match expression {
                Expression::Object { properties, .. } => literal_keys(properties),
                Expression::Call { callee, .. } if is_datetime_parser(&callee.name) => {
                    DATETIME_KEYS.iter().map(|key| (*key).to_owned()).collect()
                }
                _ => return,
            };
            if !keys.is_empty() && !lists.contains(&keys) {
                lists.push(keys);
            }
        });
    }
    lists
}

/// Calls `visit` for every assignment target inside `block`.
fn visit_block_targets<'e, 'a>(
    block: &'e Block<'a>,
    visit: &mut dyn FnMut(&'e AssignmentTarget<'a>),
) {
    for statement in &block.statements {
        visit_statement_targets(statement, visit);
    }
}

/// Calls `visit` for every assignment target inside `statement`, including nested blocks.
///
fn visit_statement_targets<'e, 'a>(
    statement: &'e Statement<'a>,
    visit: &mut dyn FnMut(&'e AssignmentTarget<'a>),
) {
    match statement {
        Statement::Assign { target, .. } => visit(target),
        Statement::Block { block, .. }
        | Statement::While { body: block, .. }
        | Statement::For { body: block, .. } => visit_block_targets(block, visit),
        Statement::If {
            then_block,
            else_branch,
            ..
        } => {
            visit_block_targets(then_block, visit);
            match else_branch {
                Some(ElseBranch::Block(block)) => visit_block_targets(block, visit),
                Some(ElseBranch::If(statement)) => visit_statement_targets(statement, visit),
                None => {}
            }
        }
        Statement::Let { .. }
        | Statement::Return { .. }
        | Statement::Expression { .. }
        | Statement::Break { .. }
        | Statement::Continue { .. } => {}
    }
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

/// Calls `visit` for `expression` and every expression inside it, outer expressions first.
pub(super) fn visit_expression_tree<'e, 'a>(
    expression: &'e Expression<'a>,
    visit: &mut dyn FnMut(&'e Expression<'a>),
) {
    visit_expression(expression, visit);
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
