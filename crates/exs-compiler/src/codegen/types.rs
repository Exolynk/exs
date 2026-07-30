//! Function-boundary type annotation resolution.

use exs_abi::{
    TYPE_ANY, TYPE_BOOL, TYPE_ERROR, TYPE_FLOAT, TYPE_INT, TYPE_LIST, TYPE_NONE, TYPE_OBJECT,
    TYPE_STRING,
};

use crate::ast::TypeAnnotation;
use crate::codegen::diagnostics;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// Resolves one optional source annotation to a runtime type mask.
pub(super) fn resolve<'a>(
    annotation: Option<&TypeAnnotation<'a>>,
    default_span: SourceSpan<'a>,
) -> Result<u32, CompileDiagnostics<'a>> {
    let Some(annotation) = annotation else {
        return Ok(TYPE_ANY);
    };
    let mut types = 0;
    for member in &annotation.members {
        let member_types = match member.name.as_str() {
            "Any" => TYPE_ANY,
            "None" => TYPE_NONE,
            "Error" => TYPE_ERROR,
            "Bool" => TYPE_BOOL,
            "Int" => TYPE_INT,
            "Float" => TYPE_FLOAT,
            "String" => TYPE_STRING,
            "List" => TYPE_LIST,
            "Object" => TYPE_OBJECT,
            _ => {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0216",
                    member.span,
                    format!("unknown type `{}`", member.name),
                )));
            }
        };
        types |= member_types;
    }
    if types == 0 {
        return Err(diagnostics(CompileDiagnostic::new(
            "E0216",
            default_span,
            "function type annotation cannot be empty",
        )));
    }
    Ok(types)
}

/// Returns whether one resolved return type permits returning a language Error.
pub(super) const fn permits_error(types: u32) -> bool {
    types & TYPE_ERROR != 0
}
