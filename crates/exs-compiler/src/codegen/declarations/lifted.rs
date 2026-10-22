//! Lifting closures into function declarations of their own.

use crate::ast::{
    AssignmentTarget, Block, Expression, FunctionDeclaration, Identifier, Module, Parameter,
    Statement, Visibility,
};
use crate::hir::HirModule;

use super::LiftedFunction;

/// Copies closure source bodies and HIR capture data into linker-owned lifted declarations.
pub(in crate::codegen) fn lifted_functions<'source>(
    module: &Module<'source>,
    hir: &HirModule<'_>,
) -> Vec<LiftedFunction<'source>> {
    let mut sources = Vec::new();
    for function in &module.functions {
        collect_closures_block(&function.body, &mut sources);
    }
    for implementation in &module.implementations {
        for method in &implementation.methods {
            collect_closures_block(&method.body, &mut sources);
        }
    }
    let metadata = hir.closures().collect::<Vec<_>>();
    debug_assert_eq!(sources.len(), metadata.len());
    sources
        .into_iter()
        .zip(metadata)
        .map(|((parameters, body, span), closure)| LiftedFunction {
            key: format!("$closure:{}", closure.id().0),
            declaration: FunctionDeclaration {
                visibility: Visibility::Private,
                entry: false,
                name: Identifier {
                    name: format!("$closure:{}", closure.id().0),
                    span,
                },
                parameters: parameters.to_vec(),
                return_type: None,
                body: body.clone(),
                span,
            },
            captures: closure
                .captures()
                .iter()
                .map(|capture| capture.name.to_owned())
                .collect(),
        })
        .collect()
}

/// Collects closure expressions in the same pre-order traversal used by HIR lowering.
fn collect_closures_block<'source, 'ast>(
    block: &'ast Block<'source>,
    closures: &mut Vec<(
        &'ast [Parameter<'source>],
        &'ast Block<'source>,
        crate::SourceSpan<'source>,
    )>,
) {
    for statement in &block.statements {
        collect_closures_statement(statement, closures);
    }
}

/// Collects closure expressions nested inside one source statement.
fn collect_closures_statement<'source, 'ast>(
    statement: &'ast Statement<'source>,
    closures: &mut Vec<(
        &'ast [Parameter<'source>],
        &'ast Block<'source>,
        crate::SourceSpan<'source>,
    )>,
) {
    match statement {
        Statement::Let { value, .. }
        | Statement::Expression {
            expression: value, ..
        } => collect_closures_expression(value, closures),
        Statement::Assign { target, value, .. } => {
            match target {
                AssignmentTarget::Variable(_) => {}
                AssignmentTarget::Index {
                    receiver, index, ..
                } => {
                    collect_closures_expression(receiver, closures);
                    collect_closures_expression(index, closures);
                }
                AssignmentTarget::Property { receiver, .. } => {
                    collect_closures_expression(receiver, closures);
                }
            }
            collect_closures_expression(value, closures);
        }
        Statement::Return { value, .. } => {
            if let Some(value) = value {
                collect_closures_expression(value, closures);
            }
        }
        Statement::Block { block, .. } => collect_closures_block(block, closures),
        Statement::If {
            condition,
            then_block,
            else_branch,
            ..
        } => {
            collect_closures_expression(condition, closures);
            collect_closures_block(then_block, closures);
            if let Some(else_branch) = else_branch {
                match else_branch {
                    crate::ast::ElseBranch::Block(block) => collect_closures_block(block, closures),
                    crate::ast::ElseBranch::If(statement) => {
                        collect_closures_statement(statement, closures)
                    }
                }
            }
        }
        Statement::While {
            condition, body, ..
        } => {
            collect_closures_expression(condition, closures);
            collect_closures_block(body, closures);
        }
        Statement::For { iterable, body, .. } => {
            collect_closures_expression(iterable, closures);
            collect_closures_block(body, closures);
        }
        Statement::Break { .. } | Statement::Continue { .. } => {}
    }
}

/// Collects closure expressions nested inside one source expression.
fn collect_closures_expression<'source, 'ast>(
    expression: &'ast Expression<'source>,
    closures: &mut Vec<(
        &'ast [Parameter<'source>],
        &'ast Block<'source>,
        crate::SourceSpan<'source>,
    )>,
) {
    match expression {
        Expression::Closure {
            parameters,
            body,
            span,
        } => {
            closures.push((parameters, body, *span));
            collect_closures_block(body, closures);
        }
        Expression::FormattedString { parts, .. } => {
            for part in parts {
                if let crate::ast::FormattedStringPart::Expression(expression) = part {
                    collect_closures_expression(expression, closures);
                }
            }
        }
        Expression::ParallelStatic { tasks, .. } => {
            for task in tasks {
                collect_closures_expression(task, closures);
            }
        }
        Expression::ParallelDynamic { functions, .. } => {
            collect_closures_expression(functions, closures);
        }
        Expression::IsType { value, .. }
        | Expression::Propagate { value, .. }
        | Expression::Unary { operand: value, .. }
        | Expression::Property {
            receiver: value, ..
        } => collect_closures_expression(value, closures),
        Expression::Binary { left, right, .. }
        | Expression::Index {
            receiver: left,
            index: right,
            ..
        } => {
            collect_closures_expression(left, closures);
            collect_closures_expression(right, closures);
        }
        Expression::Call { arguments, .. } | Expression::StaticMethodCall { arguments, .. } => {
            for argument in arguments {
                collect_closures_expression(argument, closures);
            }
        }
        Expression::HostCall {
            name, arguments, ..
        } => {
            collect_closures_expression(name, closures);
            for argument in arguments {
                collect_closures_expression(argument, closures);
            }
        }
        Expression::HostStream { arguments, .. } => {
            for argument in arguments {
                collect_closures_expression(argument, closures);
            }
        }
        Expression::HostTime { arguments, .. } => {
            for argument in arguments {
                collect_closures_expression(argument, closures);
            }
        }
        Expression::MethodCall {
            receiver,
            arguments,
            ..
        } => {
            collect_closures_expression(receiver, closures);
            for argument in arguments {
                collect_closures_expression(argument, closures);
            }
        }
        Expression::List { elements, .. } => {
            for element in elements {
                collect_closures_expression(element, closures);
            }
        }
        Expression::Object { properties, .. } | Expression::TypedObject { properties, .. } => {
            for property in properties {
                collect_closures_expression(&property.value, closures);
            }
        }
        Expression::Match { value, arms, .. } => {
            collect_closures_expression(value, closures);
            for arm in arms {
                match &arm.body {
                    crate::ast::MatchArmBody::Expression(value) => {
                        collect_closures_expression(value, closures);
                    }
                    crate::ast::MatchArmBody::Block(block) => {
                        collect_closures_block(block, closures);
                    }
                }
            }
        }
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::String(_, _)
        | Expression::Bytes(_, _)
        | Expression::Bool(_, _)
        | Expression::None(_)
        | Expression::Variable(_) => {}
    }
}
