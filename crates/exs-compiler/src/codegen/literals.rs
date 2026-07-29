//! Compiler-owned passive data segments for string literals.

use std::collections::HashMap;

use exs_runtime::WASM_TEMPLATE;
use wasmparser::{Parser as WasmParser, Payload};

use crate::ast::{Block, Expression, Module, Statement};
use crate::codegen::{diagnostics, module_span};
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
        for function in &module.functions {
            collect_block_literals(&function.body, &mut pool);
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
        match statement {
            Statement::Let { value, .. }
            | Statement::Assign { value, .. }
            | Statement::Expression {
                expression: value, ..
            } => collect_expression_literals(value, pool),
            Statement::Return { value, .. } => {
                if let Some(value) = value {
                    collect_expression_literals(value, pool);
                }
            }
            Statement::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                collect_expression_literals(condition, pool);
                collect_block_literals(then_block, pool);
                if let Some(else_block) = else_block {
                    collect_block_literals(else_block, pool);
                }
            }
        }
    }
}

/// Collects literals recursively from one expression.
fn collect_expression_literals(expression: &Expression<'_>, pool: &mut LiteralPool) {
    match expression {
        Expression::String(value, _) => pool.insert(value),
        Expression::Unary { operand, .. } => collect_expression_literals(operand, pool),
        Expression::Binary { left, right, .. } => {
            collect_expression_literals(left, pool);
            collect_expression_literals(right, pool);
        }
        Expression::Call { arguments, .. } => {
            for argument in arguments {
                collect_expression_literals(argument, pool);
            }
        }
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::Bool(_, _)
        | Expression::Variable(_) => {}
    }
}
