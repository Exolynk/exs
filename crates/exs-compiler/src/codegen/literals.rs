//! Compiler-owned passive data segments for string literals.

use std::collections::HashMap;

use exs_abi::STANDARD_ORDERING_TYPE_IDENTITY;
use exs_runtime::WASM_TEMPLATE;
use wasmparser::{Parser as WasmParser, Payload};

use crate::ast::{AssignmentTarget, Block, Expression, Module, Statement};
use crate::codegen::types::TypeRegistry;
use crate::codegen::{diagnostics, module_span, standard};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// Compiler-owned passive data segments for unique UTF-8 string literals.
pub(super) struct LiteralPool {
    pub(super) bytes: Vec<Vec<u8>>,
    pub(super) indices: HashMap<String, u32>,
    pub(super) data_index_base: u32,
}

impl LiteralPool {
    /// Collects each unique string literal in source traversal order.
    pub(super) fn collect<'a>(
        module: &Module<'a>,
        types: &TypeRegistry,
    ) -> Result<Self, CompileDiagnostics<'a>> {
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
        // Formatted interpolation dispatches through this compiler-inserted method name.
        pool.insert(standard::TO_STRING_METHOD);
        // Iterator dispatches through this compiler-inserted method name.
        pool.insert("next");
        pool.insert(standard::ASSERT_DEFAULT_DESCRIPTION);
        pool.insert(standard::ASSERT_EQ_DEFAULT_DESCRIPTION);
        for function in &module.functions {
            collect_block_literals(&function.body, &mut pool, types)?;
        }
        for implementation in &module.implementations {
            for method in &implementation.methods {
                collect_block_literals(&method.body, &mut pool, types)?;
            }
        }
        Ok(pool)
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
fn collect_block_literals<'a>(
    block: &Block<'a>,
    pool: &mut LiteralPool,
    types: &TypeRegistry,
) -> Result<(), CompileDiagnostics<'a>> {
    for statement in &block.statements {
        collect_statement_literals(statement, pool, types)?;
    }
    Ok(())
}

/// Collects literals recursively from one statement.
fn collect_statement_literals<'a>(
    statement: &Statement<'a>,
    pool: &mut LiteralPool,
    types: &TypeRegistry,
) -> Result<(), CompileDiagnostics<'a>> {
    match statement {
        Statement::Let {
            type_annotation,
            value,
            span,
            ..
        } => {
            collect_wire_schema(type_annotation.as_ref(), *span, pool, types)?;
            collect_expression_literals(value, pool, types)?;
        }
        Statement::Expression {
            expression: value, ..
        } => collect_expression_literals(value, pool, types)?,
        Statement::Assign { target, value, .. } => {
            collect_assignment_target_literals(target, pool, types)?;
            collect_expression_literals(value, pool, types)?;
        }
        Statement::Return { value, .. } => {
            if let Some(value) = value {
                collect_expression_literals(value, pool, types)?;
            }
        }
        Statement::Block { block, .. } => collect_block_literals(block, pool, types)?,
        Statement::If {
            condition,
            then_block,
            else_branch,
            ..
        } => {
            collect_expression_literals(condition, pool, types)?;
            collect_block_literals(then_block, pool, types)?;
            if let Some(else_branch) = else_branch {
                match else_branch {
                    crate::ast::ElseBranch::Block(block) => {
                        collect_block_literals(block, pool, types)?
                    }
                    crate::ast::ElseBranch::If(statement) => {
                        collect_statement_literals(statement, pool, types)?
                    }
                }
            }
        }
        Statement::While {
            condition, body, ..
        } => {
            collect_expression_literals(condition, pool, types)?;
            collect_block_literals(body, pool, types)?;
        }
        Statement::For {
            type_annotation,
            iterable,
            body,
            span,
            ..
        } => {
            collect_wire_schema(type_annotation.as_ref(), *span, pool, types)?;
            collect_expression_literals(iterable, pool, types)?;
            collect_block_literals(body, pool, types)?;
        }
        Statement::Break { .. } | Statement::Continue { .. } => {}
    }
    Ok(())
}

/// Adds the compiler-owned schema literal for one typed host-boundary binding.
fn collect_wire_schema<'a>(
    annotation: Option<&crate::ast::TypeAnnotation<'a>>,
    span: SourceSpan<'a>,
    pool: &mut LiteralPool,
    types: &TypeRegistry,
) -> Result<(), CompileDiagnostics<'a>> {
    let Some(annotation) = annotation else {
        return Ok(());
    };
    let contract = types.resolve(Some(annotation), span)?;
    pool.insert(&types.wire_schema(&contract));
    Ok(())
}

/// Collects literal-bearing expressions contained in one assignment target.
fn collect_assignment_target_literals<'a>(
    target: &AssignmentTarget<'a>,
    pool: &mut LiteralPool,
    types: &TypeRegistry,
) -> Result<(), CompileDiagnostics<'a>> {
    if let AssignmentTarget::Index {
        receiver, index, ..
    } = target
    {
        collect_expression_literals(receiver, pool, types)?;
        collect_expression_literals(index, pool, types)?;
    }
    if let AssignmentTarget::Property {
        receiver, property, ..
    } = target
    {
        collect_expression_literals(receiver, pool, types)?;
        pool.insert(&property.name);
    }
    Ok(())
}

/// Collects literals recursively from one expression.
fn collect_expression_literals<'a>(
    expression: &Expression<'a>,
    pool: &mut LiteralPool,
    types: &TypeRegistry,
) -> Result<(), CompileDiagnostics<'a>> {
    match expression {
        Expression::String(value, _) | Expression::Bytes(value, _) => pool.insert(value),
        Expression::FormattedString { parts, .. } => {
            pool.insert("");
            for part in parts {
                match part {
                    crate::ast::FormattedStringPart::Text(value) => pool.insert(value),
                    crate::ast::FormattedStringPart::Expression(expression) => {
                        collect_expression_literals(expression, pool, types)?;
                    }
                }
            }
        }
        Expression::Unary { operand, .. }
        | Expression::IsError { value: operand, .. }
        | Expression::Propagate { value: operand, .. } => {
            collect_expression_literals(operand, pool, types)?
        }
        Expression::Binary { left, right, .. } => {
            collect_expression_literals(left, pool, types)?;
            collect_expression_literals(right, pool, types)?;
        }
        Expression::Call { arguments, .. } => {
            for argument in arguments {
                collect_expression_literals(argument, pool, types)?;
            }
        }
        Expression::HostCall {
            name, arguments, ..
        } => {
            collect_expression_literals(name, pool, types)?;
            for argument in arguments {
                collect_expression_literals(argument, pool, types)?;
            }
        }
        Expression::HostStream { arguments, .. } => {
            pool.insert(exs_abi::HOST_STREAM_OPEN_HOST_NAME);
            for argument in arguments {
                collect_expression_literals(argument, pool, types)?;
            }
        }
        Expression::HostTime {
            operation,
            arguments,
            ..
        } => {
            match operation {
                crate::ast::HostTimeOperation::Now => pool.insert(exs_abi::HOST_NOW_HOST_NAME),
                crate::ast::HostTimeOperation::Elapsed => {
                    pool.insert(exs_abi::HOST_ELAPSED_HOST_NAME);
                }
                crate::ast::HostTimeOperation::InTimezone => {
                    pool.insert(exs_abi::HOST_DATETIME_IN_TIMEZONE_HOST_NAME);
                }
                crate::ast::HostTimeOperation::FromComponents => {
                    pool.insert(exs_abi::HOST_DATETIME_FROM_COMPONENTS_HOST_NAME);
                }
            }
            for argument in arguments {
                collect_expression_literals(argument, pool, types)?;
            }
        }
        Expression::List { elements, .. } => {
            for element in elements {
                collect_expression_literals(element, pool, types)?;
            }
        }
        Expression::Object { properties, .. } | Expression::TypedObject { properties, .. } => {
            for property in properties {
                pool.insert(&property.key);
                collect_expression_literals(&property.value, pool, types)?;
            }
        }
        Expression::Match { value, arms, .. } => {
            collect_expression_literals(value, pool, types)?;
            for arm in arms {
                match &arm.body {
                    crate::ast::MatchArmBody::Expression(value) => {
                        collect_expression_literals(value, pool, types)?;
                    }
                    crate::ast::MatchArmBody::Block(block) => {
                        collect_block_literals(block, pool, types)?
                    }
                }
            }
        }
        Expression::MethodCall {
            receiver,
            method,
            arguments,
            ..
        } => {
            collect_expression_literals(receiver, pool, types)?;
            pool.insert(&method.name);
            for argument in arguments {
                collect_expression_literals(argument, pool, types)?;
            }
        }
        Expression::StaticMethodCall { arguments, .. } => {
            for argument in arguments {
                collect_expression_literals(argument, pool, types)?;
            }
        }
        Expression::Index {
            receiver, index, ..
        } => {
            collect_expression_literals(receiver, pool, types)?;
            collect_expression_literals(index, pool, types)?;
        }
        Expression::Property {
            receiver, property, ..
        } => {
            collect_expression_literals(receiver, pool, types)?;
            pool.insert(&property.name);
        }
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::Bool(_, _)
        | Expression::None(_)
        | Expression::Variable(_) => {}
        Expression::Closure { body, .. } => collect_block_literals(body, pool, types)?,
        Expression::ParallelStatic { tasks, .. } => {
            for task in tasks {
                collect_expression_literals(task, pool, types)?;
            }
        }
        Expression::ParallelDynamic { functions, .. } => {
            collect_expression_literals(functions, pool, types)?
        }
    }
    Ok(())
}
