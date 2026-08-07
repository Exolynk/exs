//! Compiler-owned passive data segments for string literals.

use std::collections::HashMap;

use exs_abi::STANDARD_ORDERING_TYPE_IDENTITY;
use exs_runtime::WASM_TEMPLATE;
use wasmparser::{Parser as WasmParser, Payload};

use crate::ast::{AssignmentTarget, Block, Expression, Module, Statement};
use crate::codegen::{diagnostics, module_span, standard};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics};

/// Compiler-owned passive data segments for unique UTF-8 string literals.
pub(super) struct LiteralPool {
    pub(super) bytes: Vec<Vec<u8>>,
    pub(super) indices: HashMap<String, u32>,
    pub(super) data_index_base: u32,
}

impl LiteralPool {
    /// Collects each unique string literal in source traversal order.
    pub(super) fn collect(module: &Module<'_>) -> Self {
        let mut pool = Self {
            bytes: Vec::new(),
            indices: HashMap::new(),
            data_index_base: 0,
        };
        for declaration in &module.types {
            for field in &declaration.fields {
                pool.insert(&field.name.name);
            }
        }
        for declaration in &module.enums {
            let type_name = declaration
                .name
                .name
                .rsplit("::")
                .next()
                .unwrap_or(&declaration.name.name);
            pool.insert(&format!("{}::{type_name}", declaration.name.span.source_id));
            for variant in &declaration.variants {
                pool.insert(&variant.name.name);
            }
        }
        for descriptor in standard::enums() {
            let identity = if descriptor.name == standard::ORDERING_ENUM {
                STANDARD_ORDERING_TYPE_IDENTITY.to_owned()
            } else {
                format!("std::{}", descriptor.name)
            };
            pool.insert(&identity);
            for variant in descriptor.variants {
                pool.insert(variant);
            }
        }
        for function in &module.functions {
            collect_block_literals(&function.body, &mut pool);
        }
        for implementation in &module.implementations {
            for method in &implementation.methods {
                collect_block_literals(&method.body, &mut pool);
            }
        }
        pool
    }

    /// Assigns final Wasm data indexes after the runtime template's segments.
    pub(super) fn with_data_index_base(mut self, data_index_base: u32) -> Self {
        self.data_index_base = data_index_base;
        for index in self.indices.values_mut() {
            *index += data_index_base;
        }
        self
    }

    /// Adds one literal if it was not already collected.
    fn insert(&mut self, literal: &str) {
        if self.indices.contains_key(literal) {
            return;
        }
        let index = match u32::try_from(self.bytes.len()) {
            Ok(index) => index,
            Err(_) => return,
        };
        self.bytes.push(literal.as_bytes().to_vec());
        self.indices.insert(literal.to_owned(), index);
    }
}

/// Runtime-template data-section metadata needed for compiler passive segments.
pub(super) struct TemplateDataLayout {
    pub(super) count: u32,
    pub(super) has_data_count: bool,
    pub(super) has_data_section: bool,
}

/// Counts runtime-template data segments before assigning compiler literal indexes.
pub(super) fn template_data_layout<'a>(
    module: &Module<'a>,
) -> Result<TemplateDataLayout, CompileDiagnostics<'a>> {
    let mut layout = TemplateDataLayout {
        count: 0,
        has_data_count: false,
        has_data_section: false,
    };
    for payload in WasmParser::new(0).parse_all(WASM_TEMPLATE) {
        let payload = payload.map_err(|error| {
            diagnostics(CompileDiagnostic::new(
                "E1001",
                module_span(module),
                format!("could not inspect runtime template data sections: {error}"),
            ))
        })?;
        match payload {
            Payload::DataCountSection { .. } => layout.has_data_count = true,
            Payload::DataSection(section) => {
                layout.has_data_section = true;
                layout.count = section.count();
            }
            _ => {}
        }
    }
    Ok(layout)
}

/// Collects literals recursively from one statement block.
fn collect_block_literals(block: &Block<'_>, pool: &mut LiteralPool) {
    for statement in &block.statements {
        collect_statement_literals(statement, pool);
    }
}

/// Collects literals recursively from one statement.
fn collect_statement_literals(statement: &Statement<'_>, pool: &mut LiteralPool) {
    match statement {
        Statement::Let { value, .. }
        | Statement::Expression {
            expression: value, ..
        } => collect_expression_literals(value, pool),
        Statement::Assign { target, value, .. } => {
            collect_assignment_target_literals(target, pool);
            collect_expression_literals(value, pool);
        }
        Statement::Return { value, .. } => {
            if let Some(value) = value {
                collect_expression_literals(value, pool);
            }
        }
        Statement::Block { block, .. } => collect_block_literals(block, pool),
        Statement::If {
            condition,
            then_block,
            else_branch,
            ..
        } => {
            collect_expression_literals(condition, pool);
            collect_block_literals(then_block, pool);
            if let Some(else_branch) = else_branch {
                match else_branch {
                    crate::ast::ElseBranch::Block(block) => collect_block_literals(block, pool),
                    crate::ast::ElseBranch::If(statement) => {
                        collect_statement_literals(statement, pool)
                    }
                }
            }
        }
        Statement::While {
            condition, body, ..
        } => {
            collect_expression_literals(condition, pool);
            collect_block_literals(body, pool);
        }
        Statement::For { iterable, body, .. } => {
            collect_expression_literals(iterable, pool);
            collect_block_literals(body, pool);
        }
        Statement::Break { .. } | Statement::Continue { .. } => {}
    }
}

/// Collects literal-bearing expressions contained in one assignment target.
fn collect_assignment_target_literals(target: &AssignmentTarget<'_>, pool: &mut LiteralPool) {
    if let AssignmentTarget::Index {
        receiver, index, ..
    } = target
    {
        collect_expression_literals(receiver, pool);
        collect_expression_literals(index, pool);
    }
    if let AssignmentTarget::Property {
        receiver, property, ..
    } = target
    {
        collect_expression_literals(receiver, pool);
        pool.insert(&property.name);
    }
}

/// Collects literals recursively from one expression.
fn collect_expression_literals(expression: &Expression<'_>, pool: &mut LiteralPool) {
    match expression {
        Expression::String(value, _) => pool.insert(value),
        Expression::Unary { operand, .. }
        | Expression::IsError { value: operand, .. }
        | Expression::Propagate { value: operand, .. } => {
            collect_expression_literals(operand, pool)
        }
        Expression::Binary { left, right, .. } => {
            collect_expression_literals(left, pool);
            collect_expression_literals(right, pool);
        }
        Expression::Call { arguments, .. } => {
            for argument in arguments {
                collect_expression_literals(argument, pool);
            }
        }
        Expression::HostCall {
            name, arguments, ..
        } => {
            collect_expression_literals(name, pool);
            for argument in arguments {
                collect_expression_literals(argument, pool);
            }
        }
        Expression::List { elements, .. } => {
            for element in elements {
                collect_expression_literals(element, pool);
            }
        }
        Expression::Object { properties, .. } | Expression::TypedObject { properties, .. } => {
            for property in properties {
                pool.insert(&property.key);
                collect_expression_literals(&property.value, pool);
            }
        }
        Expression::Match { value, arms, .. } => {
            collect_expression_literals(value, pool);
            for arm in arms {
                match &arm.body {
                    crate::ast::MatchArmBody::Expression(value) => {
                        collect_expression_literals(value, pool);
                    }
                    crate::ast::MatchArmBody::Block(block) => collect_block_literals(block, pool),
                }
            }
        }
        Expression::MethodCall {
            receiver,
            method,
            arguments,
            ..
        } => {
            collect_expression_literals(receiver, pool);
            pool.insert(&method.name);
            for argument in arguments {
                collect_expression_literals(argument, pool);
            }
        }
        Expression::StaticMethodCall { arguments, .. } => {
            for argument in arguments {
                collect_expression_literals(argument, pool);
            }
        }
        Expression::Index {
            receiver, index, ..
        } => {
            collect_expression_literals(receiver, pool);
            collect_expression_literals(index, pool);
        }
        Expression::Property {
            receiver, property, ..
        } => {
            collect_expression_literals(receiver, pool);
            pool.insert(&property.name);
        }
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::Bool(_, _)
        | Expression::None(_)
        | Expression::Variable(_) => {}
        Expression::Closure { body, .. } => collect_block_literals(body, pool),
        Expression::ParallelStatic { tasks, .. } => {
            for task in tasks {
                collect_expression_literals(task, pool);
            }
        }
        Expression::ParallelDynamic { functions, .. } => {
            collect_expression_literals(functions, pool)
        }
    }
}
