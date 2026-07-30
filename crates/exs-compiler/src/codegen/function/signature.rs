//! Function declaration validation and Wasm signature construction.

use std::collections::HashMap;

use exs_abi::RESERVED_METHOD_NAMES;
use wasm_encoder::{TypeSection, ValType};

use crate::ast::{FunctionDeclaration, Module};
use crate::codegen::types::{TypeContract, TypeRegistry};
use crate::codegen::{diagnostics, module_span};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// The linked Wasm function index and source arity of one ExS function.
#[derive(Debug, Clone)]
pub(in crate::codegen) struct FunctionSignature {
    pub(in crate::codegen) index: u32,
    pub(in crate::codegen) arity: usize,
    pub(in crate::codegen) function_id: u32,
    pub(in crate::codegen) parameter_types: Vec<TypeContract>,
    pub(in crate::codegen) return_type: TypeContract,
}

/// Validates declarations and assigns their final linked Wasm function indexes.
pub(in crate::codegen) fn build_signatures<'a>(
    module: &Module<'a>,
    program_base: u32,
    types: &TypeRegistry,
) -> Result<HashMap<String, FunctionSignature>, CompileDiagnostics<'a>> {
    let mut signatures = HashMap::new();
    let mut offset = 0_u32;
    for function in &module.functions {
        insert_signature(
            &mut signatures,
            &function.name.name,
            function,
            program_base,
            offset,
            types,
            None,
        )?;
        offset = offset
            .checked_add(1)
            .ok_or_else(|| too_many_functions(function.name.span))?;
    }
    for implementation in &module.implementations {
        let nominal = types.get(&implementation.type_name.name).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0216",
                implementation.type_name.span,
                format!("unknown type `{}`", implementation.type_name.name),
            ))
        })?;
        for method in &implementation.methods {
            if RESERVED_METHOD_NAMES.contains(&method.name.name.as_str()) {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0223",
                    method.name.span,
                    format!("method `{}` is reserved by the runtime", method.name.name),
                )));
            }
            let key = format!("{}::{}", implementation.type_name.name, method.name.name);
            let receiver_type = method
                .parameters
                .first()
                .filter(|parameter| parameter.name.name == "self")
                .map(|_| nominal.id);
            insert_signature(
                &mut signatures,
                &key,
                method,
                program_base,
                offset,
                types,
                receiver_type,
            )?;
            offset = offset
                .checked_add(1)
                .ok_or_else(|| too_many_functions(method.name.span))?;
        }
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

/// Inserts one direct or implementation method signature into the linked function table.
fn insert_signature<'a>(
    signatures: &mut HashMap<String, FunctionSignature>,
    key: &str,
    function: &FunctionDeclaration<'a>,
    program_base: u32,
    function_id: u32,
    types: &TypeRegistry,
    receiver_type: Option<u32>,
) -> Result<(), CompileDiagnostics<'a>> {
    if signatures.contains_key(key) {
        return Err(diagnostics(CompileDiagnostic::new(
            "E0201",
            function.name.span,
            format!("duplicate function `{key}`"),
        )));
    }
    let mut parameters = HashMap::new();
    let mut parameter_types = Vec::new();
    for (index, parameter) in function.parameters.iter().enumerate() {
        if parameters.insert(&parameter.name.name, ()).is_some() {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0202",
                parameter.name.span,
                format!("duplicate parameter `{}`", parameter.name.name),
            )));
        }
        if index == 0 && receiver_type.is_some() {
            if parameter.type_annotation.is_some() {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0224",
                    parameter.name.span,
                    "the implicit impl receiver `self` cannot have a type annotation",
                )));
            }
            parameter_types.push(TypeContract {
                builtin_mask: 0,
                nominal_type_ids: vec![receiver_type.unwrap_or_default()],
            });
        } else {
            parameter_types
                .push(types.resolve(parameter.type_annotation.as_ref(), parameter.name.span)?);
        }
    }
    signatures.insert(
        key.to_owned(),
        FunctionSignature {
            index: program_base + function_id,
            arity: function.parameters.len(),
            function_id,
            parameter_types,
            return_type: types.resolve(function.return_type.as_ref(), function.name.span)?,
        },
    );
    Ok(())
}

/// Creates the shared diagnostic used when a module has too many linked functions.
fn too_many_functions<'a>(span: SourceSpan<'a>) -> CompileDiagnostics<'a> {
    diagnostics(CompileDiagnostic::new(
        "E0212",
        span,
        "too many functions in one module",
    ))
}

/// Adds one ValueRef-based Wasm signature for every source function.
pub(in crate::codegen) fn add_program_types(
    module: &Module<'_>,
    types: &mut TypeSection,
) -> Vec<u32> {
    module
        .functions
        .iter()
        .chain(
            module
                .implementations
                .iter()
                .flat_map(|implementation| implementation.methods.iter()),
        )
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
