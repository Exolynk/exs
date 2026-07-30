//! Function declaration validation and Wasm signature construction.

use std::collections::HashMap;

use wasm_encoder::{TypeSection, ValType};

use crate::ast::Module;
use crate::codegen::types;
use crate::codegen::{diagnostics, module_span};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics};

/// The linked Wasm function index and source arity of one ExS function.
#[derive(Debug, Clone)]
pub(in crate::codegen) struct FunctionSignature {
    pub(in crate::codegen) index: u32,
    pub(in crate::codegen) arity: usize,
    pub(in crate::codegen) function_id: u32,
    pub(in crate::codegen) parameter_types: Vec<u32>,
    pub(in crate::codegen) return_type: u32,
}

/// Validates declarations and assigns their final linked Wasm function indexes.
pub(in crate::codegen) fn build_signatures<'a>(
    module: &Module<'a>,
    program_base: u32,
) -> Result<HashMap<String, FunctionSignature>, CompileDiagnostics<'a>> {
    let mut signatures = HashMap::new();
    for (offset, function) in module.functions.iter().enumerate() {
        if signatures.contains_key(&function.name.name) {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0201",
                function.name.span,
                format!("duplicate function `{}`", function.name.name),
            )));
        }
        let mut parameters = HashMap::new();
        let mut parameter_types = Vec::new();
        for parameter in &function.parameters {
            if parameters.insert(&parameter.name.name, ()).is_some() {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0202",
                    parameter.name.span,
                    format!("duplicate parameter `{}`", parameter.name.name),
                )));
            }
            parameter_types.push(types::resolve(
                parameter.type_annotation.as_ref(),
                parameter.name.span,
            )?);
        }
        let return_type = types::resolve(function.return_type.as_ref(), function.name.span)?;
        signatures.insert(
            function.name.name.clone(),
            FunctionSignature {
                index: program_base + offset as u32,
                arity: function.parameters.len(),
                function_id: offset as u32,
                parameter_types,
                return_type,
            },
        );
    }
    if signatures.contains_key("main") {
        Ok(signatures)
    } else {
        Err(diagnostics(CompileDiagnostic::new(
            "E0200",
            module_span(module),
            "missing fn main()",
        )))
    }
}

/// Adds one ValueRef-based Wasm signature for every source function.
pub(in crate::codegen) fn add_program_types(
    module: &Module<'_>,
    types: &mut TypeSection,
) -> Vec<u32> {
    module
        .functions
        .iter()
        .map(|function| {
            let index = types.len();
            types.ty().function(
                std::iter::repeat_n(ValType::I32, function.parameters.len()),
                [ValType::I32],
            );
            index
        })
        .collect()
}
