//! Built-in and nominal type-contract resolution.

use std::collections::HashMap;

use exs_abi::{
    TYPE_ANY, TYPE_BOOL, TYPE_ERROR, TYPE_FLOAT, TYPE_INT, TYPE_LIST, TYPE_NONE, TYPE_OBJECT,
    TYPE_STRING,
};

use crate::ast::{Module, TypeAnnotation};
use crate::codegen::diagnostics;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// Collects independent nominal type declaration and field contract diagnostics.
pub(super) fn validate<'a>(module: &Module<'a>) -> CompileDiagnostics<'a> {
    let mut diagnostics = CompileDiagnostics::new();
    let mut declarations = HashMap::new();
    for declaration in &module.types {
        if builtin_mask(&declaration.name.name).is_some() {
            diagnostics.push(CompileDiagnostic::new(
                "E0219",
                declaration.name.span,
                format!("duplicate or reserved type `{}`", declaration.name.name),
            ));
        } else if let Some(previous) =
            declarations.insert(&declaration.name.name, declaration.name.span)
        {
            diagnostics.push(
                CompileDiagnostic::new(
                    "E0219",
                    declaration.name.span,
                    format!("duplicate or reserved type `{}`", declaration.name.name),
                )
                .with_related(previous, "previous declaration is here"),
            );
        }
    }
    for declaration in &module.traits {
        if builtin_mask(&declaration.name.name).is_some() {
            diagnostics.push(CompileDiagnostic::new(
                "E0219",
                declaration.name.span,
                format!("duplicate or reserved trait `{}`", declaration.name.name),
            ));
        } else if let Some(previous) =
            declarations.insert(&declaration.name.name, declaration.name.span)
        {
            diagnostics.push(
                CompileDiagnostic::new(
                    "E0219",
                    declaration.name.span,
                    format!(
                        "trait `{}` conflicts with an existing type or trait",
                        declaration.name.name
                    ),
                )
                .with_related(previous, "previous declaration is here"),
            );
        }
    }
    for declaration in &module.types {
        let mut fields = HashMap::new();
        for field in &declaration.fields {
            if let Some(previous) = fields.insert(&field.name.name, field.name.span) {
                diagnostics.push(
                    CompileDiagnostic::new(
                        "E0220",
                        field.name.span,
                        format!("duplicate field `{}`", field.name.name),
                    )
                    .with_related(previous, "previous field declaration is here"),
                );
            }
            validate_annotation(
                module,
                field.type_annotation.as_ref(),
                field.name.span,
                &mut diagnostics,
            );
        }
    }
    diagnostics
}

/// Validates every member of one optional source type annotation.
pub(super) fn validate_annotation<'a>(
    module: &Module<'a>,
    annotation: Option<&TypeAnnotation<'a>>,
    default_span: SourceSpan<'a>,
    diagnostics: &mut CompileDiagnostics<'a>,
) {
    let Some(annotation) = annotation else {
        return;
    };
    if annotation.members.is_empty() {
        diagnostics.push(CompileDiagnostic::new(
            "E0216",
            default_span,
            "type annotation cannot be empty",
        ));
        return;
    }
    for member in &annotation.members {
        if builtin_mask(&member.name).is_none()
            && !module
                .types
                .iter()
                .any(|declaration| declaration.name.name == member.name)
            && !module
                .traits
                .iter()
                .any(|declaration| declaration.name.name == member.name)
        {
            diagnostics.push(CompileDiagnostic::new(
                "E0216",
                member.span,
                format!("unknown type `{}`", member.name),
            ));
        }
    }
}

/// One resolved runtime type contract.
#[derive(Debug, Clone)]
pub(super) struct TypeContract {
    /// Accepted built-in runtime value categories.
    pub(super) builtin_mask: u32,
    /// Accepted nominal Object type identifiers.
    pub(super) nominal_type_ids: Vec<u32>,
}

/// One resolved nominal Object field contract.
#[derive(Debug, Clone)]
pub(super) struct NominalField {
    /// Source-visible field name.
    pub(super) name: String,
    /// Accepted value types for this field.
    pub(super) contract: TypeContract,
}

/// One compiler-owned nominal Object type.
#[derive(Debug, Clone)]
pub(super) struct NominalType {
    /// Opaque runtime tag assigned in source order.
    pub(super) id: u32,
    /// Fields in declaration order.
    pub(super) fields: Vec<NominalField>,
}

/// The nominal type registry for one compiled module.
#[derive(Debug, Clone)]
pub(super) struct TypeRegistry {
    types: HashMap<String, NominalType>,
    trait_implementations: HashMap<String, Vec<u32>>,
}

impl TypeRegistry {
    /// Collects named types and resolves every declared field contract.
    pub(super) fn build<'a>(module: &Module<'a>) -> Result<Self, CompileDiagnostics<'a>> {
        let mut types = HashMap::new();
        for (offset, declaration) in module.types.iter().enumerate() {
            if builtin_mask(&declaration.name.name).is_some()
                || types.contains_key(&declaration.name.name)
            {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0219",
                    declaration.name.span,
                    format!("duplicate or reserved type `{}`", declaration.name.name),
                )));
            }
            let id = u32::try_from(offset)
                .ok()
                .and_then(|id| id.checked_add(1))
                .ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0212",
                        declaration.name.span,
                        "too many nominal types in one module",
                    ))
                })?;
            types.insert(
                declaration.name.name.clone(),
                NominalType {
                    id,
                    fields: Vec::new(),
                },
            );
        }
        let mut trait_implementations = HashMap::new();
        for declaration in &module.traits {
            trait_implementations.insert(declaration.name.name.clone(), Vec::new());
        }
        for implementation in &module.implementations {
            let Some(trait_name) = &implementation.trait_name else {
                continue;
            };
            let nominal = types.get(&implementation.type_name.name).ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0999",
                    implementation.type_name.span,
                    "missing resolved trait implementation type",
                ))
            })?;
            let implementations =
                trait_implementations
                    .get_mut(&trait_name.name)
                    .ok_or_else(|| {
                        diagnostics(CompileDiagnostic::new(
                            "E0999",
                            trait_name.span,
                            "missing resolved trait declaration",
                        ))
                    })?;
            if !implementations.contains(&nominal.id) {
                implementations.push(nominal.id);
            }
        }
        let mut registry = Self {
            types,
            trait_implementations,
        };
        for declaration in &module.types {
            let mut fields = Vec::new();
            for field in &declaration.fields {
                if fields
                    .iter()
                    .any(|existing: &NominalField| existing.name == field.name.name)
                {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0220",
                        field.name.span,
                        format!("duplicate field `{}`", field.name.name),
                    )));
                }
                fields.push(NominalField {
                    name: field.name.name.clone(),
                    contract: registry.resolve(field.type_annotation.as_ref(), field.name.span)?,
                });
            }
            let Some(registered) = registry.types.get_mut(&declaration.name.name) else {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0999",
                    declaration.name.span,
                    "missing collected nominal type",
                )));
            };
            registered.fields = fields;
        }
        Ok(registry)
    }

    /// Resolves one optional source union annotation against this module's named types.
    pub(super) fn resolve<'a>(
        &self,
        annotation: Option<&TypeAnnotation<'a>>,
        _default_span: SourceSpan<'a>,
    ) -> Result<TypeContract, CompileDiagnostics<'a>> {
        let Some(annotation) = annotation else {
            return Ok(TypeContract {
                builtin_mask: TYPE_ANY,
                nominal_type_ids: Vec::new(),
            });
        };
        let mut resolved_builtin_mask = 0;
        let mut nominal_type_ids = Vec::new();
        for member in &annotation.members {
            if let Some(mask) = builtin_mask(&member.name) {
                resolved_builtin_mask |= mask;
            } else if let Some(nominal) = self.types.get(&member.name) {
                if !nominal_type_ids.contains(&nominal.id) {
                    nominal_type_ids.push(nominal.id);
                }
            } else if let Some(implementations) = self.trait_implementations.get(&member.name) {
                for type_id in implementations {
                    if !nominal_type_ids.contains(type_id) {
                        nominal_type_ids.push(*type_id);
                    }
                }
            } else {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0216",
                    member.span,
                    format!("unknown type `{}`", member.name),
                )));
            }
        }
        Ok(TypeContract {
            builtin_mask: resolved_builtin_mask,
            nominal_type_ids,
        })
    }

    /// Returns one nominal Object declaration by its source-visible name.
    pub(super) fn get(&self, name: &str) -> Option<&NominalType> {
        self.types.get(name)
    }
}

/// Returns whether a function return contract permits language Error values.
pub(super) const fn permits_error(contract: &TypeContract) -> bool {
    contract.builtin_mask & TYPE_ERROR != 0
}

/// Resolves one built-in source type spelling to its ABI mask.
fn builtin_mask(name: &str) -> Option<u32> {
    match name {
        "Any" => Some(TYPE_ANY),
        "None" => Some(TYPE_NONE),
        "Error" => Some(TYPE_ERROR),
        "Bool" => Some(TYPE_BOOL),
        "Int" => Some(TYPE_INT),
        "Float" => Some(TYPE_FLOAT),
        "String" => Some(TYPE_STRING),
        "List" => Some(TYPE_LIST),
        "Object" => Some(TYPE_OBJECT),
        _ => None,
    }
}
