//! Trait declaration validation and default-method expansion.

use std::collections::HashMap;

use crate::ast::{FunctionDeclaration, Module, TraitMethodDeclaration, TypeAnnotation};
use crate::codegen::types;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// Collects trait declaration and implementation diagnostics before linking.
pub(super) fn validate<'a>(module: &Module<'a>) -> CompileDiagnostics<'a> {
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
        let key = format!("{}::{}", trait_name.name, implementation.type_name.name);
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
        let Some(declaration) = module
            .traits
            .iter()
            .find(|declaration| declaration.name.name == trait_name.name)
        else {
            diagnostics.push(CompileDiagnostic::new(
                "E0216",
                trait_name.span,
                format!("unknown trait `{}`", trait_name.name),
            ));
            continue;
        };
        if !module
            .types
            .iter()
            .any(|type_declaration| type_declaration.name.name == implementation.type_name.name)
        {
            diagnostics.push(CompileDiagnostic::new(
                "E0216",
                implementation.type_name.span,
                format!("unknown type `{}`", implementation.type_name.name),
            ));
        }
        validate_implementation(module, declaration, implementation, &mut diagnostics);
    }
    validate_exposed_method_names(module, &mut diagnostics);
    diagnostics
}

/// Adds inherited default methods to every valid trait implementation.
pub(super) fn apply_defaults(module: &mut Module<'_>) {
    let defaults = module
        .traits
        .iter()
        .map(|declaration| {
            (
                declaration.name.name.clone(),
                declaration
                    .methods
                    .iter()
                    .filter_map(|method| {
                        method
                            .default_implementation()
                            .map(|function| (method.name.name.clone(), function))
                    })
                    .collect::<HashMap<String, FunctionDeclaration<'_>>>(),
            )
        })
        .collect::<HashMap<_, _>>();
    for implementation in &mut module.implementations {
        let Some(trait_name) = &implementation.trait_name else {
            continue;
        };
        let Some(methods) = defaults.get(&trait_name.name) else {
            continue;
        };
        for (name, method) in methods {
            if !implementation
                .methods
                .iter()
                .any(|implementation_method| implementation_method.name.name == *name)
            {
                implementation.methods.push(method.clone());
            }
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
        types::validate_annotation(
            module,
            parameter.type_annotation.as_ref(),
            parameter.name.span,
            diagnostics,
        );
    }
    types::validate_annotation(
        module,
        method.return_type.as_ref(),
        method.name.span,
        diagnostics,
    );
}

/// Validates methods supplied by one `impl Trait for Type` declaration.
fn validate_implementation<'a>(
    module: &Module<'a>,
    declaration: &crate::ast::TraitDeclaration<'a>,
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
        let Some(required) = declaration
            .methods
            .iter()
            .find(|trait_method| trait_method.name.name == method.name.name)
        else {
            diagnostics.push(CompileDiagnostic::new(
                "E0229",
                method.name.span,
                format!(
                    "method `{}` is not declared by trait `{}`",
                    method.name.name, declaration.name.name
                ),
            ));
            continue;
        };
        if !same_signature(required, method) {
            diagnostics.push(
                CompileDiagnostic::new(
                    "E0230",
                    method.name.span,
                    format!(
                        "method `{}` does not match trait `{}`",
                        method.name.name, declaration.name.name
                    ),
                )
                .with_related(required.name.span, "trait method declaration is here"),
            );
        }
    }
    for method in &declaration.methods {
        if method.body.is_none() && !supplied.contains_key(&method.name.name) {
            diagnostics.push(CompileDiagnostic::new(
                "E0228",
                implementation.type_name.span,
                format!(
                    "implementation of trait `{}` for `{}` is missing method `{}`",
                    declaration.name.name, implementation.type_name.name, method.name.name
                ),
            ));
        }
    }
    for method in &implementation.methods {
        for parameter in &method.parameters {
            types::validate_annotation(
                module,
                parameter.type_annotation.as_ref(),
                parameter.name.span,
                diagnostics,
            );
        }
        types::validate_annotation(
            module,
            method.return_type.as_ref(),
            method.name.span,
            diagnostics,
        );
    }
}

/// Rejects duplicate method names exposed by one nominal type after default inheritance.
fn validate_exposed_method_names<'a>(
    module: &Module<'a>,
    diagnostics: &mut CompileDiagnostics<'a>,
) {
    for type_declaration in &module.types {
        let mut names = HashMap::new();
        for implementation in module
            .implementations
            .iter()
            .filter(|implementation| implementation.type_name.name == type_declaration.name.name)
        {
            let trait_methods = implementation.trait_name.as_ref().and_then(|trait_name| {
                module
                    .traits
                    .iter()
                    .find(|declaration| declaration.name.name == trait_name.name)
            });
            for method in &implementation.methods {
                insert_exposed_method(&mut names, &method.name.name, method.name.span, diagnostics);
            }
            if let Some(trait_declaration) = trait_methods {
                for method in &trait_declaration.methods {
                    if method.body.is_some()
                        && !implementation.methods.iter().any(|implementation_method| {
                            implementation_method.name.name == method.name.name
                        })
                    {
                        insert_exposed_method(
                            &mut names,
                            &method.name.name,
                            method.name.span,
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

/// Returns whether an implementation method has exactly the trait method's source signature.
fn same_signature(
    method: &TraitMethodDeclaration<'_>,
    implementation: &FunctionDeclaration<'_>,
) -> bool {
    method.parameters.len() == implementation.parameters.len()
        && method
            .parameters
            .first()
            .is_some_and(|parameter| parameter.name.name == "self")
            == implementation
                .parameters
                .first()
                .is_some_and(|parameter| parameter.name.name == "self")
        && method
            .parameters
            .iter()
            .zip(&implementation.parameters)
            .all(|(required, supplied)| {
                same_annotation(
                    required.type_annotation.as_ref(),
                    supplied.type_annotation.as_ref(),
                )
            })
        && same_annotation(
            method.return_type.as_ref(),
            implementation.return_type.as_ref(),
        )
}

/// Returns whether two optional source union annotations have the same ordered members.
fn same_annotation(left: Option<&TypeAnnotation<'_>>, right: Option<&TypeAnnotation<'_>>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.members.len() == right.members.len()
                && left
                    .members
                    .iter()
                    .zip(&right.members)
                    .all(|(left, right)| left.name == right.name)
        }
        _ => false,
    }
}
