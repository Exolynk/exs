//! The functions a v2 module compiles: top-level functions and implementation methods.

use crate::ast::{FunctionDeclaration, Module};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics};

/// One source function the backend compiles.
pub(super) struct Unit<'m, 'a> {
    /// Function key: the name of a top-level function, `Type::method` for a method.
    pub(super) key: String,
    /// The declaration.
    pub(super) declaration: &'m FunctionDeclaration<'a>,
    /// Identifier recorded in Error traces: like v1, the position among the module's
    /// functions followed by its implementation methods.
    pub(super) function_id: u32,
}

/// Returns every function and implementation method of `module` in v1's trace-id order,
/// rejecting duplicate keys and annotated receivers like v1's linker.
pub(super) fn units<'m, 'a>(
    module: &'m Module<'a>,
) -> Result<Vec<Unit<'m, 'a>>, CompileDiagnostics<'a>> {
    let methods = module.implementations.iter().flat_map(|implementation| {
        implementation.methods.iter().map(move |method| {
            (
                format!("{}::{}", implementation.type_name.name, method.name.name),
                method,
            )
        })
    });
    let functions = module
        .functions
        .iter()
        .map(|function| (function.name.name.clone(), function));
    let mut units: Vec<Unit<'m, 'a>> = Vec::new();
    for ((key, declaration), function_id) in functions.chain(methods).zip(0_u32..) {
        if units.iter().any(|unit| unit.key == key) {
            return Err(CompileDiagnostics::from(CompileDiagnostic::new(
                "E0201",
                declaration.name.span,
                format!("duplicate function `{key}`"),
            )));
        }
        let is_method = key.contains("::");
        let receiver = declaration
            .parameters
            .first()
            .filter(|parameter| is_method && parameter.name.name == "self");
        if let Some(receiver) = receiver
            && receiver.type_annotation.is_some()
        {
            return Err(CompileDiagnostics::from(CompileDiagnostic::new(
                "E0224",
                receiver.name.span,
                "the implicit impl receiver `self` cannot have a type annotation",
            )));
        }
        units.push(Unit {
            key,
            declaration,
            function_id,
        });
    }
    Ok(units)
}
