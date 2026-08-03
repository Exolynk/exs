//! Built-in and nominal type-contract resolution.

use std::collections::HashMap;

use exs_abi::{
    TYPE_ANY, TYPE_BOOL, TYPE_ERROR, TYPE_FLOAT, TYPE_FN, TYPE_INT, TYPE_LIST, TYPE_NONE,
    TYPE_OBJECT, TYPE_STRING,
};

use crate::ast::{EnumDeclaration, Module, TypeAnnotation};
use crate::codegen::diagnostics;
use crate::codegen::standard;
use crate::codegen::trait_registry::TraitRegistry;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// Collects independent nominal type declaration and field contract diagnostics.
pub(super) fn validate<'a>(module: &Module<'a>) -> CompileDiagnostics<'a> {
    let mut diagnostics = CompileDiagnostics::new();
    let mut declarations = HashMap::new();
    for declaration in &module.types {
        if is_reserved_type_name(&declaration.name.name) {
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
    for declaration in &module.enums {
        if is_reserved_type_name(&declaration.name.name) {
            diagnostics.push(CompileDiagnostic::new(
                "E0219",
                declaration.name.span,
                format!("duplicate or reserved enum `{}`", declaration.name.name),
            ));
        } else if let Some(previous) =
            declarations.insert(&declaration.name.name, declaration.name.span)
        {
            diagnostics.push(
                CompileDiagnostic::new(
                    "E0219",
                    declaration.name.span,
                    format!(
                        "enum `{}` conflicts with an existing type or trait",
                        declaration.name.name
                    ),
                )
                .with_related(previous, "previous declaration is here"),
            );
        }
        let mut variants = HashMap::new();
        for variant in &declaration.variants {
            if let Some(previous) = variants.insert(&variant.name.name, variant.name.span) {
                diagnostics.push(
                    CompileDiagnostic::new(
                        "E0220",
                        variant.name.span,
                        format!("duplicate enum variant `{}`", variant.name.name),
                    )
                    .with_related(previous, "previous variant is here"),
                );
            }
            for field in &variant.fields {
                validate_annotation(
                    module,
                    field.type_annotation.as_ref(),
                    field.name.span,
                    &mut diagnostics,
                );
            }
        }
    }
    for declaration in &module.traits {
        if is_reserved_type_name(&declaration.name.name) {
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
    validate_annotation_with_self(module, annotation, default_span, false, diagnostics);
}

/// Validates one optional source annotation, optionally accepting contextual `Self` members.
pub(super) fn validate_annotation_with_self<'a>(
    module: &Module<'a>,
    annotation: Option<&TypeAnnotation<'a>>,
    default_span: SourceSpan<'a>,
    allows_self: bool,
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
        if allows_self && member.name == "Self" {
            continue;
        }
        if member.name == "Self" {
            diagnostics.push(CompileDiagnostic::new(
                "E0216",
                member.span,
                "`Self` is valid only in trait declarations and trait implementations",
            ));
            continue;
        }
        if builtin_mask(&member.name).is_none()
            && standard::canonical_trait_name(&member.name).is_none()
            && !module
                .types
                .iter()
                .any(|declaration| declaration.name.name == member.name)
            && !module
                .traits
                .iter()
                .any(|declaration| declaration.name.name == member.name)
            && !module
                .enums
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

/// Returns whether one declaration name is reserved by the type system.
fn is_reserved_type_name(name: &str) -> bool {
    name == "Self" || builtin_mask(name).is_some() || standard::canonical_trait_name(name).is_some()
}

/// One resolved runtime type contract.
#[derive(Debug, Clone)]
pub(super) struct TypeContract {
    /// Accepted built-in runtime value categories.
    pub(super) builtin_mask: u32,
    /// Accepted nominal Object type identifiers.
    pub(super) nominal_type_ids: Vec<u32>,
    /// Accepted stable identities for nominal enum values received from a host.
    pub(super) enum_type_ids: Vec<String>,
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
    /// Whether this nominal type is an Object or enum.
    pub(super) kind: NominalKind,
    /// Stable host-boundary identity when this nominal type is an enum.
    pub(super) enum_type_id: Option<String>,
}

/// The runtime construction category of one nominal type.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum NominalKind {
    /// A field-keyed nominal Object.
    Object,
    /// A tagged enum value.
    Enum,
}

/// One resolved enum-variant constructor.
#[derive(Debug, Clone)]
pub(super) struct EnumVariant {
    /// Nominal enum type tag.
    pub(super) type_id: u32,
    /// Stable host-boundary type identity.
    pub(super) type_identity: String,
    /// Source-visible variant name.
    pub(super) name: String,
    /// Ordered payload contracts.
    pub(super) fields: Vec<TypeContract>,
}

/// The nominal type registry for one compiled module.
#[derive(Debug, Clone)]
pub(super) struct TypeRegistry {
    types: HashMap<String, NominalType>,
    enum_variants: HashMap<String, EnumVariant>,
    trait_implementations: HashMap<String, TraitImplementations>,
}

/// Built-in and nominal implementations that satisfy one resolved trait contract.
#[derive(Debug, Clone)]
struct TraitImplementations {
    /// Runtime value categories supplied by compiler-owned implementations.
    builtin_mask: u32,
    /// Nominal Object tags supplied by source `impl Trait for Type` blocks.
    nominal_type_ids: Vec<u32>,
}

impl TypeRegistry {
    /// Collects named types and resolves every declared field contract.
    pub(super) fn build<'a>(
        module: &Module<'a>,
        traits: &TraitRegistry<'a>,
    ) -> Result<Self, CompileDiagnostics<'a>> {
        let mut types = HashMap::new();
        let mut next_id = 1_u32;
        for declaration in &module.types {
            if is_reserved_type_name(&declaration.name.name)
                || types.contains_key(&declaration.name.name)
            {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0219",
                    declaration.name.span,
                    format!("duplicate or reserved type `{}`", declaration.name.name),
                )));
            }
            let id = next_id;
            next_id = next_id.checked_add(1).ok_or_else(|| {
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
                    kind: NominalKind::Object,
                    enum_type_id: None,
                },
            );
        }
        for declaration in &module.enums {
            if is_reserved_type_name(&declaration.name.name)
                || types.contains_key(&declaration.name.name)
            {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0219",
                    declaration.name.span,
                    format!("duplicate or reserved enum `{}`", declaration.name.name),
                )));
            }
            let id = next_id;
            next_id = next_id.checked_add(1).ok_or_else(|| {
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
                    kind: NominalKind::Enum,
                    enum_type_id: Some(enum_identity(declaration)),
                },
            );
        }
        let mut trait_implementations = traits
            .definitions()
            .map(|definition| {
                (
                    definition.name.clone(),
                    TraitImplementations {
                        builtin_mask: definition.builtin_mask,
                        nominal_type_ids: Vec::new(),
                    },
                )
            })
            .collect::<HashMap<_, _>>();
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
            let canonical_trait = traits.definition(&trait_name.name).map_or_else(
                || trait_name.name.as_str(),
                |definition| definition.name.as_str(),
            );
            let implementations =
                trait_implementations
                    .get_mut(canonical_trait)
                    .ok_or_else(|| {
                        diagnostics(CompileDiagnostic::new(
                            "E0999",
                            trait_name.span,
                            "missing resolved trait declaration",
                        ))
                    })?;
            if !implementations.nominal_type_ids.contains(&nominal.id) {
                implementations.nominal_type_ids.push(nominal.id);
            }
        }
        let mut registry = Self {
            types,
            enum_variants: HashMap::new(),
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
        for declaration in &module.enums {
            registry.register_enum(declaration)?;
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
                enum_type_ids: Vec::new(),
            });
        };
        let mut resolved_builtin_mask = 0;
        let mut nominal_type_ids = Vec::new();
        let mut enum_type_ids = Vec::new();
        for member in &annotation.members {
            if let Some(mask) = builtin_mask(&member.name) {
                resolved_builtin_mask |= mask;
            } else if let Some(nominal) = self.types.get(&member.name) {
                if !nominal_type_ids.contains(&nominal.id) {
                    nominal_type_ids.push(nominal.id);
                }
                if let Some(enum_type_id) = &nominal.enum_type_id
                    && !enum_type_ids.contains(enum_type_id)
                {
                    enum_type_ids.push(enum_type_id.clone());
                }
            } else if let Some(implementations) = self
                .trait_implementations
                .get(standard::canonical_trait_name(&member.name).unwrap_or(&member.name))
            {
                resolved_builtin_mask |= implementations.builtin_mask;
                for type_id in &implementations.nominal_type_ids {
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
            enum_type_ids,
        })
    }

    /// Returns one nominal Object declaration by its source-visible name.
    pub(super) fn get(&self, name: &str) -> Option<&NominalType> {
        self.types.get(name)
    }

    /// Resolves one canonical enum constructor name.
    pub(super) fn enum_variant(&self, name: &str) -> Option<&EnumVariant> {
        self.enum_variants.get(name)
    }

    /// Returns every declared variant name for one canonical enum type.
    pub(super) fn enum_variant_names(&self, type_name: &str) -> Vec<String> {
        let prefix = format!("{type_name}::");
        self.enum_variants
            .keys()
            .filter_map(|name| name.strip_prefix(&prefix).map(ToOwned::to_owned))
            .collect()
    }

    /// Registers every constructor and payload contract for one enum declaration.
    fn register_enum<'a>(
        &mut self,
        declaration: &EnumDeclaration<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let type_id = self
            .types
            .get(&declaration.name.name)
            .map(|item| item.id)
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0999",
                    declaration.name.span,
                    "missing collected enum type",
                ))
            })?;
        let type_identity = enum_identity(declaration);
        for variant in &declaration.variants {
            let fields = variant
                .fields
                .iter()
                .map(|field| self.resolve(field.type_annotation.as_ref(), field.name.span))
                .collect::<Result<Vec<_>, _>>()?;
            self.enum_variants.insert(
                format!("{}::{}", declaration.name.name, variant.name.name),
                EnumVariant {
                    type_id,
                    type_identity: type_identity.clone(),
                    name: variant.name.name.clone(),
                    fields,
                },
            );
        }
        Ok(())
    }
}

/// Returns the resolver-derived identity used for one enum at the host boundary.
fn enum_identity(declaration: &EnumDeclaration<'_>) -> String {
    let type_name = declaration
        .name
        .name
        .rsplit("::")
        .next()
        .unwrap_or(&declaration.name.name);
    format!("{}::{type_name}", declaration.name.span.source_id)
}

/// Returns whether a function return contract permits language Error values.
pub(super) const fn permits_error(contract: &TypeContract) -> bool {
    contract.builtin_mask & TYPE_ERROR != 0
}

/// Resolves one built-in source type spelling to its ABI mask.
fn builtin_mask(name: &str) -> Option<u32> {
    match name.strip_prefix("std::").unwrap_or(name) {
        "Any" => Some(TYPE_ANY),
        "None" => Some(TYPE_NONE),
        "Error" => Some(TYPE_ERROR),
        "Bool" => Some(TYPE_BOOL),
        "Int" => Some(TYPE_INT),
        "Float" => Some(TYPE_FLOAT),
        "String" => Some(TYPE_STRING),
        "List" => Some(TYPE_LIST),
        "Object" => Some(TYPE_OBJECT),
        "Fn" => Some(TYPE_FN),
        _ => None,
    }
}
