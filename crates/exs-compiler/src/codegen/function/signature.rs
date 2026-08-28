//! Function declaration validation and Wasm signature construction.

use std::collections::{HashMap, HashSet};

use exs_abi::RESERVED_METHOD_NAMES;
use wasm_encoder::{TypeSection, ValType};

use crate::ast::{FunctionDeclaration, Module};
use crate::codegen::types::{TypeContract, TypeRegistry};
use crate::codegen::{diagnostics, module_span};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// Top-level standard-library functions that are directly available in every module.
const RESERVED_STANDARD_FUNCTION_NAMES: &[&str] = &["assert", "assert_eq"];

/// Collects independent function and implementation declaration diagnostics before linking.
pub(in crate::codegen) fn validate<'a>(module: &Module<'a>) -> CompileDiagnostics<'a> {
    let mut diagnostics = CompileDiagnostics::new();
    let mut signatures = HashMap::new();
    for function in &module.functions {
        if RESERVED_STANDARD_FUNCTION_NAMES.contains(&function.name.name.as_str()) {
            diagnostics.push(CompileDiagnostic::new(
                "E0223",
                function.name.span,
                format!(
                    "`{}` is a reserved standard-library function",
                    function.name.name
                ),
            ));
        }
        validate_function(
            module,
            function,
            None,
            false,
            &mut signatures,
            &mut diagnostics,
        );
    }
    for implementation in &module.implementations {
        let nominal = module
            .types
            .iter()
            .map(|declaration| declaration.name.name.as_str())
            .chain(
                module
                    .enums
                    .iter()
                    .map(|declaration| declaration.name.name.as_str()),
            )
            .find(|name| *name == implementation.type_name.name);
        if nominal.is_none() {
            diagnostics.push(CompileDiagnostic::new(
                "E0216",
                implementation.type_name.span,
                format!("unknown type `{}`", implementation.type_name.name),
            ));
        }
        let allows_self = implementation.trait_name.is_some();
        for method in &implementation.methods {
            if RESERVED_METHOD_NAMES.contains(&method.name.name.as_str()) {
                diagnostics.push(CompileDiagnostic::new(
                    "E0223",
                    method.name.span,
                    format!("method `{}` is reserved by the runtime", method.name.name),
                ));
            }
            validate_function(
                module,
                method,
                nominal,
                allows_self,
                &mut signatures,
                &mut diagnostics,
            );
        }
    }
    if !signatures.contains_key("main") {
        diagnostics.push(CompileDiagnostic::new(
            "E0200",
            module_span(module),
            "missing fn main()",
        ));
    }
    diagnostics
}

/// Validates one function declaration without assigning its linked Wasm index.
fn validate_function<'a>(
    module: &Module<'a>,
    function: &FunctionDeclaration<'a>,
    implementation_type: Option<&str>,
    allows_self: bool,
    signatures: &mut HashMap<String, SourceSpan<'a>>,
    diagnostics: &mut CompileDiagnostics<'a>,
) {
    let key = implementation_type.map_or_else(
        || function.name.name.clone(),
        |type_name| format!("{type_name}::{}", function.name.name),
    );
    if let Some(previous) = signatures.insert(key.clone(), function.name.span) {
        diagnostics.push(
            CompileDiagnostic::new(
                "E0201",
                function.name.span,
                format!("duplicate function `{key}`"),
            )
            .with_related(previous, "previous function declaration is here"),
        );
    }
    let mut parameters = HashMap::new();
    for (index, parameter) in function.parameters.iter().enumerate() {
        if let Some(previous) = parameters.insert(&parameter.name.name, parameter.name.span) {
            diagnostics.push(
                CompileDiagnostic::new(
                    "E0202",
                    parameter.name.span,
                    format!("duplicate parameter `{}`", parameter.name.name),
                )
                .with_related(previous, "previous parameter declaration is here"),
            );
        }
        if parameter.variadic && index + 1 != function.parameters.len() {
            diagnostics.push(CompileDiagnostic::new(
                "E0217",
                parameter.name.span,
                "a variadic parameter must be the final parameter",
            ));
        }
        if index == 0 && implementation_type.is_some() && parameter.name.name == "self" {
            if parameter.type_annotation.is_some() || parameter.variadic {
                diagnostics.push(CompileDiagnostic::new(
                    "E0224",
                    parameter.name.span,
                    "the implicit impl receiver `self` cannot have a type annotation or be variadic",
                ));
            }
        } else {
            crate::codegen::types::validate_annotation_with_self(
                module,
                parameter.type_annotation.as_ref(),
                parameter.name.span,
                allows_self,
                diagnostics,
            );
        }
    }
    crate::codegen::types::validate_annotation_with_self(
        module,
        function.return_type.as_ref(),
        function.name.span,
        allows_self,
        diagnostics,
    );
}

/// The linked Wasm function index and source arity of one ExS function.
#[derive(Debug, Clone)]
pub(in crate::codegen) struct FunctionSignature {
    pub(in crate::codegen) index: u32,
    /// Number of required source arguments before an optional variadic tail.
    pub(in crate::codegen) arity: usize,
    /// Whether the final Wasm parameter receives a packed List of trailing source arguments.
    pub(in crate::codegen) variadic: bool,
    pub(in crate::codegen) capture_count: usize,
    pub(in crate::codegen) function_id: u32,
    pub(in crate::codegen) parameter_types: Vec<TypeContract>,
    pub(in crate::codegen) return_type: TypeContract,
}

impl FunctionSignature {
    /// Returns whether this signature accepts a call with the supplied source argument count.
    pub(in crate::codegen) fn accepts_arity(&self, count: usize) -> bool {
        if self.variadic {
            count >= self.arity
        } else {
            count == self.arity
        }
    }

    /// Returns the fixed Wasm parameter count after packing an optional variadic tail.
    pub(in crate::codegen) fn wasm_arity(&self) -> usize {
        self.arity + usize::from(self.variadic)
    }

    /// Renders the source-level argument count accepted by this signature.
    pub(in crate::codegen) fn expected_arity_description(&self) -> String {
        if self.variadic {
            format!("at least {}", self.arity)
        } else {
            self.arity.to_string()
        }
    }
}

/// One compiler-private function lifted from a source closure expression.
pub(in crate::codegen) struct LiftedFunction<'a> {
    /// Private linker key used by continuation lowering and dispatch.
    pub(in crate::codegen) key: String,
    /// Synthetic declaration preserving the source closure body and parameters.
    pub(in crate::codegen) declaration: FunctionDeclaration<'a>,
    /// Captured binding names in first-use closure environment order.
    pub(in crate::codegen) captures: Vec<String>,
}

/// Validates declarations and assigns their final linked Wasm function indexes.
pub(in crate::codegen) fn build_signatures<'a>(
    module: &Module<'a>,
    lifted: &[LiftedFunction<'a>],
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
            0,
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
                0,
            )?;
            offset = offset
                .checked_add(1)
                .ok_or_else(|| too_many_functions(method.name.span))?;
        }
    }
    for closure in lifted {
        insert_signature(
            &mut signatures,
            &closure.key,
            &closure.declaration,
            program_base,
            offset,
            types,
            None,
            closure.captures.len(),
        )?;
        offset = offset
            .checked_add(1)
            .ok_or_else(|| too_many_functions(closure.declaration.span))?;
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
#[allow(clippy::too_many_arguments)] // Linked function metadata is intentionally passed explicitly.
fn insert_signature<'a>(
    signatures: &mut HashMap<String, FunctionSignature>,
    key: &str,
    function: &FunctionDeclaration<'a>,
    program_base: u32,
    function_id: u32,
    types: &TypeRegistry,
    receiver_type: Option<u32>,
    capture_count: usize,
) -> Result<(), CompileDiagnostics<'a>> {
    if signatures.contains_key(key) {
        return Err(diagnostics(CompileDiagnostic::new(
            "E0201",
            function.name.span,
            format!("duplicate function `{key}`"),
        )));
    }
    let variadic = function
        .parameters
        .last()
        .is_some_and(|parameter| parameter.variadic);
    if function
        .parameters
        .iter()
        .take(function.parameters.len().saturating_sub(1))
        .any(|parameter| parameter.variadic)
    {
        return Err(diagnostics(CompileDiagnostic::new(
            "E0217",
            function.span,
            "a variadic parameter must be the final parameter",
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
                enum_type_ids: Vec::new(),
                list_item: None,
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
            arity: function.parameters.len() - usize::from(variadic),
            variadic,
            capture_count,
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
    lifted: &[LiftedFunction<'_>],
    types: &mut TypeSection,
    suspendable_functions: &HashSet<String>,
) -> Vec<u32> {
    let step_type = types.len();
    types.ty().function([ValType::I32], [ValType::I32]);
    module
        .functions
        .iter()
        .map(|function| (function.name.name.clone(), function))
        .chain(module.implementations.iter().flat_map(|implementation| {
            implementation.methods.iter().map(|function| {
                (
                    format!("{}::{}", implementation.type_name.name, function.name.name),
                    function,
                )
            })
        }))
        .map(|(key, function)| {
            if suspendable_functions.contains(&key) {
                step_type
            } else {
                let index = types.len();
                types.ty().function(
                    std::iter::repeat_n(ValType::I32, function.parameters.len()),
                    [ValType::I32],
                );
                index
            }
        })
        .chain(std::iter::repeat_n(step_type, lifted.len()))
        .collect()
}
