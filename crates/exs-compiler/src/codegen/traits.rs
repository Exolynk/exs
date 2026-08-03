//! Trait implementation validation and default-method expansion.

use std::collections::HashMap;

use crate::ast::{Module, TraitMethodDeclaration};
use crate::codegen::trait_registry::{TraitDefinition, TraitRegistry};
use crate::codegen::types;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// Collects trait declaration and implementation diagnostics before linking.
pub(super) fn validate<'a>(
    module: &Module<'a>,
    registry: &TraitRegistry<'a>,
) -> CompileDiagnostics<'a> {
    let mut diagnostics = CompileDiagnostics::new();
    let mut implementations = HashMap::new();
    for declaration in &module.traits {
        let mut methods = HashMap::new();
        for method in &declaration.methods {
            if let Some(previous) = methods.insert(&method.name.name, method.name.span) {
                diagnostics.push(
                    CompileDiagnostic::new(
                        "E0225",
                        method.name.span,
                        format!("duplicate trait method `{}`", method.name.name),
                    )
                    .with_related(previous, "previous trait method declaration is here"),
                );
            }
            validate_method_signature(module, method, &mut diagnostics);
        }
    }
    for implementation in &module.implementations {
        let Some(trait_name) = &implementation.trait_name else {
            continue;
        };
        let Some(definition) = registry.definition(&trait_name.name) else {
            diagnostics.push(CompileDiagnostic::new(
                "E0216",
                trait_name.span,
                format!("unknown trait `{}`", trait_name.name),
            ));
            continue;
        };
        let key = format!("{}::{}", definition.name, implementation.type_name.name);
        if let Some(previous) = implementations.insert(key.clone(), implementation.span) {
            diagnostics.push(
                CompileDiagnostic::new(
                    "E0227",
                    implementation.span,
                    format!("duplicate implementation of `{key}`"),
                )
                .with_related(previous, "previous trait implementation is here"),
            );
        }
        if !module
            .types
            .iter()
            .any(|type_declaration| type_declaration.name.name == implementation.type_name.name)
            && !module
                .enums
                .iter()
                .any(|enum_declaration| enum_declaration.name.name == implementation.type_name.name)
        {
            diagnostics.push(CompileDiagnostic::new(
                "E0216",
                implementation.type_name.span,
                format!("unknown type `{}`", implementation.type_name.name),
            ));
        }
        validate_implementation(module, definition, implementation, &mut diagnostics);
    }
    validate_exposed_method_names(module, registry, &mut diagnostics);
    diagnostics
}

/// Adds inherited default methods to every valid trait implementation.
pub(super) fn apply_defaults<'a>(module: &mut Module<'a>, registry: &TraitRegistry<'a>) {
    for implementation in &mut module.implementations {
        let Some(trait_name) = &implementation.trait_name else {
            continue;
        };
        let Some(definition) = registry.definition(&trait_name.name) else {
            continue;
        };
        for method in &definition.methods {
            let Some(default) = &method.default_implementation else {
                continue;
            };
            if !implementation
                .methods
                .iter()
                .any(|implementation_method| implementation_method.name.name == method.name)
            {
                implementation.methods.push(default.clone());
            }
        }
    }
    for implementation in &mut module.implementations {
        if implementation.trait_name.is_none() {
            continue;
        }
        for method in &mut implementation.methods {
            replace_self_annotations(method, &implementation.type_name.name);
        }
    }
}

/// Validates one trait method declaration's function-boundary annotations.
fn validate_method_signature<'a>(
    module: &Module<'a>,
    method: &TraitMethodDeclaration<'a>,
    diagnostics: &mut CompileDiagnostics<'a>,
) {
    let mut parameters = HashMap::new();
    for parameter in &method.parameters {
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
        types::validate_annotation_with_self(
            module,
            parameter.type_annotation.as_ref(),
            parameter.name.span,
            true,
            diagnostics,
        );
    }
    types::validate_annotation_with_self(
        module,
        method.return_type.as_ref(),
        method.name.span,
        true,
        diagnostics,
    );
}

/// Validates methods supplied by one `impl Trait for Type` declaration.
fn validate_implementation<'a>(
    module: &Module<'a>,
    definition: &TraitDefinition<'a>,
    implementation: &crate::ast::ImplDeclaration<'a>,
    diagnostics: &mut CompileDiagnostics<'a>,
) {
    let mut supplied = HashMap::new();
    for method in &implementation.methods {
        if let Some(previous) = supplied.insert(&method.name.name, method.name.span) {
            diagnostics.push(
                CompileDiagnostic::new(
                    "E0201",
                    method.name.span,
                    format!("duplicate method `{}`", method.name.name),
                )
                .with_related(previous, "previous implementation method is here"),
            );
        }
        let Some(required) = definition
            .methods
            .iter()
            .find(|trait_method| trait_method.name == method.name.name)
        else {
            diagnostics.push(CompileDiagnostic::new(
                "E0229",
                method.name.span,
                format!(
                    "method `{}` is not declared by trait `{}`",
                    method.name.name, definition.name
                ),
            ));
            continue;
        };
        if !required
            .signature
            .matches(method, &implementation.type_name.name)
        {
            let message = required.display_signature.as_ref().map_or_else(
                || {
                    format!(
                        "method `{}` does not match trait `{}`",
                        method.name.name, definition.name
                    )
                },
                |signature| {
                    format!(
                        "method `{}` must have signature `{signature}`",
                        method.name.name
                    )
                },
            );
            let diagnostic = CompileDiagnostic::new("E0230", method.name.span, message);
            diagnostics.push(
                required
                    .declaration_span
                    .map_or(diagnostic.clone(), |span| {
                        diagnostic.with_related(span, "trait method declaration is here")
                    }),
            );
        }
    }
    for method in &definition.methods {
        if method.default_implementation.is_none() && !supplied.contains_key(&method.name) {
            diagnostics.push(CompileDiagnostic::new(
                "E0228",
                implementation.type_name.span,
                format!(
                    "implementation of trait `{}` for `{}` is missing method `{}`",
                    definition.name, implementation.type_name.name, method.name
                ),
            ));
        }
    }
    for method in &implementation.methods {
        for parameter in &method.parameters {
            types::validate_annotation_with_self(
                module,
                parameter.type_annotation.as_ref(),
                parameter.name.span,
                true,
                diagnostics,
            );
        }
        types::validate_annotation_with_self(
            module,
            method.return_type.as_ref(),
            method.name.span,
            true,
            diagnostics,
        );
    }
}

/// Rejects duplicate method names exposed by one nominal type after default inheritance.
fn validate_exposed_method_names<'a>(
    module: &Module<'a>,
    registry: &TraitRegistry<'a>,
    diagnostics: &mut CompileDiagnostics<'a>,
) {
    for type_declaration in module
        .types
        .iter()
        .map(|declaration| &declaration.name.name)
        .chain(
            module
                .enums
                .iter()
                .map(|declaration| &declaration.name.name),
        )
    {
        let mut names = HashMap::new();
        for implementation in module
            .implementations
            .iter()
            .filter(|implementation| implementation.type_name.name == *type_declaration)
        {
            let trait_methods = implementation
                .trait_name
                .as_ref()
                .and_then(|trait_name| registry.definition(&trait_name.name));
            for method in &implementation.methods {
                insert_exposed_method(&mut names, &method.name.name, method.name.span, diagnostics);
            }
            if let Some(definition) = trait_methods {
                for method in &definition.methods {
                    if method.default_implementation.is_some()
                        && !implementation.methods.iter().any(|implementation_method| {
                            implementation_method.name.name == method.name
                        })
                    {
                        insert_exposed_method(
                            &mut names,
                            &method.name,
                            implementation.type_name.span,
                            diagnostics,
                        );
                    }
                }
            }
        }
    }
}

/// Records one type-exposed method name or emits a duplicate-name diagnostic.
fn insert_exposed_method<'a>(
    names: &mut HashMap<String, SourceSpan<'a>>,
    name: &str,
    span: SourceSpan<'a>,
    diagnostics: &mut CompileDiagnostics<'a>,
) {
    if let Some(previous) = names.insert(name.to_owned(), span) {
        diagnostics.push(
            CompileDiagnostic::new("E0226", span, format!("duplicate exposed method `{name}`"))
                .with_related(previous, "previous method is here"),
        );
    }
}

/// Replaces contextual `Self` annotations with the concrete implementation target.
fn replace_self_annotations(method: &mut crate::ast::FunctionDeclaration<'_>, self_type: &str) {
    for parameter in &mut method.parameters {
        replace_self_annotation(parameter.type_annotation.as_mut(), self_type);
    }
    replace_self_annotation(method.return_type.as_mut(), self_type);
}

/// Rewrites each contextual `Self` union member in one optional type annotation.
fn replace_self_annotation(
    annotation: Option<&mut crate::ast::TypeAnnotation<'_>>,
    self_type: &str,
) {
    let Some(annotation) = annotation else {
        return;
    };
    for member in &mut annotation.members {
        if member.name == "Self" {
            member.name = self_type.to_owned();
        }
    }
}
