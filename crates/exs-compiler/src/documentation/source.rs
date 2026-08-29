use super::shared::*;
use super::standard::{render_clone_method, standard_pages};
use super::*;
use crate::loaded_project::{ImportEdge, LoadedProject, LoadedSource, parse_source};

/// Generates Markdown documentation for one root source and its import graph.
pub(crate) fn generate<R: ModuleResolver>(
    source: SourceInput<'_>,
    resolver: &mut R,
) -> Result<Documentation, String> {
    let project = LoadedProject::load(source, resolver)?;
    project.validate_no_cycles()?;
    let files = project.files();
    let edges = project.edges();
    let modules = project.parse_modules()?;
    let directories = files
        .iter()
        .enumerate()
        .map(|(index, file)| module_directory(index, &file.source_id))
        .collect::<Vec<_>>();
    let mut pages = standard_pages()?;
    for (index, module) in modules.iter().enumerate() {
        pages.extend(module_pages(
            module,
            &files[index],
            &edges[index],
            &directories,
            &directories[index],
        ));
    }
    Ok(Documentation {
        index: render_index(files, &directories),
        pages,
    })
}

/// Parses one documentation source unit without requiring a root entry point.
pub(super) fn parse<'a>(source_id: &'a str, text: &'a str) -> Result<Module<'a>, String> {
    parse_source(source_id, text)
}

/// Renders the project index and concise language overview.
fn render_index(files: &[LoadedSource], directories: &[String]) -> String {
    let mut output = String::from("# ExS API Documentation\n\n");
    output.push_str("This reference is generated from the root module and every reachable relative import. Adjacent `///` comments describe source declarations.\n\n");
    output.push_str("## Language\n\n");
    output.push_str("ExS is a dynamically typed scripting language compiled to WebAssembly. Root modules declare `fn main(...)`; imported modules provide functions, nominal types, traits, and implementations. `Host::call(name, args...)` invokes a runner-provided host function and may suspend. `par { ... }` runs fixed tasks concurrently, while `par(functions)` runs a List of zero-argument closures.\n\n");
    output.push_str("## Modules\n\n");
    output.push_str(
        "- [`std`](modules/std/index.md) - globally available built-in types and operations.\n",
    );
    for (file, directory) in files.iter().zip(directories) {
        output.push_str(&format!(
            "- [`{}`]({directory}/index.md)\n",
            file.display_path
        ));
    }
    output
}

/// Generates a module index and dedicated nominal, trait, and function pages.
fn module_pages(
    module: &Module<'_>,
    source: &LoadedSource,
    imports: &[ImportEdge],
    directories: &[String],
    directory: &str,
) -> Vec<DocumentationPage> {
    let mut pages = Vec::new();
    pages.push(DocumentationPage {
        path: format!("{directory}/index.md"),
        markdown: render_module_index(module, source, imports, directories),
    });
    for declaration in &module.types {
        pages.push(DocumentationPage {
            path: format!("{directory}/types/{}.md", slug(&declaration.name.name)),
            markdown: render_type_page(module, declaration, source),
        });
    }
    for declaration in &module.enums {
        pages.push(DocumentationPage {
            path: format!("{directory}/enums/{}.md", slug(&declaration.name.name)),
            markdown: render_enum_page(module, declaration, source),
        });
    }
    for declaration in &module.traits {
        pages.push(DocumentationPage {
            path: format!("{directory}/traits/{}.md", slug(&declaration.name.name)),
            markdown: render_trait_page(declaration, source),
        });
    }
    for declaration in &module.functions {
        pages.push(DocumentationPage {
            path: format!("{directory}/fn/{}.md", slug(&declaration.name.name)),
            markdown: render_function_page(declaration, source, "Function"),
        });
    }
    pages
}

/// Renders one user-module declaration index without inline API details.
fn render_module_index(
    module: &Module<'_>,
    source: &LoadedSource,
    imports: &[ImportEdge],
    directories: &[String],
) -> String {
    let mut output = format!("# Module `{}`\n\n", source.display_path);
    if !imports.is_empty() {
        output.push_str("## Imports\n\n");
        for import in imports {
            let target = Path::new(&directories[import.target])
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(directories[import.target].as_str());
            output.push_str(&format!(
                "- `{}` -> [module](../{target}/index.md)\n",
                import.namespace
            ));
        }
        output.push('\n');
    }
    if !module.uses.is_empty() {
        output.push_str("## Used Declarations\n\n");
        for declaration in &module.uses {
            let items = declaration
                .items
                .iter()
                .map(|item| {
                    item.alias.as_ref().map_or_else(
                        || item.name.name.clone(),
                        |alias| format!("{} as {}", item.name.name, alias.name),
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            output.push_str(&format!("- `{}::{items}`\n", declaration.namespace.name));
        }
        output.push('\n');
    }
    link_section(
        &mut output,
        "Types",
        &module.types,
        |declaration| format!("types/{}.md", slug(&declaration.name.name)),
        |declaration| declaration.name.name.as_str(),
    );
    link_section(
        &mut output,
        "Enums",
        &module.enums,
        |declaration| format!("enums/{}.md", slug(&declaration.name.name)),
        |declaration| declaration.name.name.as_str(),
    );
    link_section(
        &mut output,
        "Traits",
        &module.traits,
        |declaration| format!("traits/{}.md", slug(&declaration.name.name)),
        |declaration| declaration.name.name.as_str(),
    );
    link_section(
        &mut output,
        "Functions",
        &module.functions,
        |declaration| format!("fn/{}.md", slug(&declaration.name.name)),
        |declaration| declaration.name.name.as_str(),
    );
    if module.types.is_empty()
        && module.enums.is_empty()
        && module.traits.is_empty()
        && module.functions.is_empty()
    {
        output.push_str("No public declarations.\n");
    }
    output
}

/// Renders one user-defined enum and all methods implemented for it.
pub(super) fn render_enum_page(
    module: &Module<'_>,
    declaration: &EnumDeclaration<'_>,
    source: &LoadedSource,
) -> String {
    let mut output = format!("# Enum `{}`\n\n", declaration.name.name);
    append_comment(&mut output, &source.text, declaration.span);
    output.push_str("```exs\n");
    output.push_str(&format!("enum {} {{\n", declaration.name.name));
    for variant in &declaration.variants {
        output.push_str(&format!("    {}", variant.name.name));
        if !variant.fields.is_empty() {
            output.push('(');
            output.push_str(
                &variant
                    .fields
                    .iter()
                    .map(|field| {
                        field.type_annotation.as_ref().map_or_else(
                            || field.name.name.clone(),
                            |annotation| {
                                format!("{}: {}", field.name.name, type_annotation(annotation))
                            },
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            output.push(')');
        }
        output.push_str(",\n");
    }
    output.push_str("}\n```\n\n");
    if !declaration.variants.is_empty() {
        output.push_str("## Variants\n\n");
        for variant in &declaration.variants {
            output.push_str(&format!("### `{}`\n\n", variant.name.name));
            append_comment(&mut output, &source.text, variant.span);
        }
    }
    render_nominal_implementations(&mut output, module, &declaration.name.name, source);
    output.push_str("\n## Runtime Methods\n\n");
    render_clone_method(&mut output, &declaration.name.name);
    output
}

/// Appends a declaration list that links to dedicated API pages.
fn link_section<T>(
    output: &mut String,
    heading: &str,
    declarations: &[T],
    path: impl Fn(&T) -> String,
    name: impl Fn(&T) -> &str,
) {
    if declarations.is_empty() {
        return;
    }
    output.push_str(&format!("## {heading}\n\n"));
    for declaration in declarations {
        output.push_str(&format!(
            "- [`{}`]({})\n",
            name(declaration),
            path(declaration)
        ));
    }
    output.push('\n');
}

/// Renders one user-defined nominal type and all methods implemented for it.
pub(super) fn render_type_page(
    module: &Module<'_>,
    declaration: &TypeDeclaration<'_>,
    source: &LoadedSource,
) -> String {
    let mut output = format!("# Type `{}`\n\n", declaration.name.name);
    append_comment(&mut output, &source.text, declaration.span);
    output.push_str("```exs\n");
    output.push_str(&format!("type {} {{\n", declaration.name.name));
    for field in &declaration.fields {
        output.push_str(&format!(
            "    {}{},\n",
            field.name.name,
            field
                .type_annotation
                .as_ref()
                .map_or_else(String::new, |annotation| format!(
                    ": {}",
                    type_annotation(annotation)
                ))
        ));
    }
    output.push_str("}\n```\n\n");
    render_nominal_implementations(&mut output, module, &declaration.name.name, source);
    output.push_str("\n## Runtime Methods\n\n");
    render_clone_method(&mut output, &declaration.name.name);
    output
}

/// Renders all inherent and trait-provided methods of one nominal declaration.
fn render_nominal_implementations(
    output: &mut String,
    module: &Module<'_>,
    name: &str,
    source: &LoadedSource,
) {
    let implementations = module
        .implementations
        .iter()
        .filter(|implementation| implementation.type_name.name == name)
        .collect::<Vec<_>>();
    if !implementations.is_empty() {
        output.push_str("## Implemented Methods\n\n");
        for implementation in implementations {
            let label = implementation.trait_name.as_ref().map_or_else(
                || "Inherent methods".to_owned(),
                |trait_name| {
                    trait_documentation_link(module, &trait_name.name).map_or_else(
                        || format!("Trait `{}`", trait_name.name),
                        |link| format!("Trait [`{}`]({})", trait_name.name, link),
                    )
                },
            );
            output.push_str(&format!("### {label}\n\n"));
            for method in &implementation.methods {
                let fallback = implementation.trait_name.as_ref().and_then(|trait_name| {
                    trait_method_documentation(module, &trait_name.name, &method.name.name, source)
                });
                render_function_details(output, method, &source.text, 4, fallback.as_deref());
            }
        }
    }
}

/// Returns the relative API-page link for one trait implemented by a local nominal declaration.
fn trait_documentation_link(module: &Module<'_>, trait_name: &str) -> Option<String> {
    if let Some(descriptor) = codegen_standard::trait_descriptor(trait_name) {
        Some(format!("../../std/traits/{}.md", slug(descriptor.name)))
    } else if module
        .traits
        .iter()
        .any(|declaration| declaration.name.name == trait_name)
    {
        Some(format!("../traits/{}.md", slug(trait_name)))
    } else {
        None
    }
}

/// Returns documentation inherited by one trait implementation method, when available.
fn trait_method_documentation(
    module: &Module<'_>,
    trait_name: &str,
    method_name: &str,
    source: &LoadedSource,
) -> Option<String> {
    if let Some(descriptor) = codegen_standard::trait_descriptor(trait_name) {
        return descriptor
            .methods
            .iter()
            .find(|method| method.name == method_name)
            .map(|method| method.description.to_owned());
    }
    module
        .traits
        .iter()
        .find(|declaration| declaration.name.name == trait_name)
        .and_then(|declaration| {
            declaration
                .methods
                .iter()
                .find(|method| method.name.name == method_name)
        })
        .and_then(|method| documentation_comment(&source.text, method.span))
}

/// Renders one user-defined trait and all of its method details.
pub(super) fn render_trait_page(
    declaration: &TraitDeclaration<'_>,
    source: &LoadedSource,
) -> String {
    let mut output = format!("# Trait `{}`\n\n", declaration.name.name);
    append_comment(&mut output, &source.text, declaration.span);
    if declaration.methods.is_empty() {
        output.push_str("This trait declares no methods.\n");
        return output;
    }
    output.push_str("## Methods\n\n");
    for method in &declaration.methods {
        render_trait_method_details(&mut output, method, &source.text);
    }
    output
}

/// Renders one direct-function page.
fn render_function_page(
    declaration: &FunctionDeclaration<'_>,
    source: &LoadedSource,
    kind: &str,
) -> String {
    let mut output = format!("# {kind} `{}`\n\n", declaration.name.name);
    append_comment(&mut output, &source.text, declaration.span);
    output.push_str("```exs\n");
    output.push_str(&function_signature(
        &declaration.name.name,
        &declaration.parameters,
        declaration.return_type.as_ref(),
    ));
    output.push_str(" { ... }\n```\n");
    output
}

/// Renders one function signature and associated doc comment inside a type page.
fn render_function_details(
    output: &mut String,
    declaration: &FunctionDeclaration<'_>,
    source: &str,
    level: usize,
    fallback_description: Option<&str>,
) {
    output.push_str(&format!(
        "{} `{}`\n\n",
        "#".repeat(level),
        declaration.name.name
    ));
    if !append_comment(output, source, declaration.span)
        && let Some(description) = fallback_description
    {
        output.push_str(description);
        output.push_str("\n\n");
    }
    output.push_str("```exs\n");
    output.push_str(&function_signature(
        &declaration.name.name,
        &declaration.parameters,
        declaration.return_type.as_ref(),
    ));
    output.push_str(" { ... }\n```\n\n");
}

/// Renders one trait method signature and associated doc comment.
fn render_trait_method_details(
    output: &mut String,
    declaration: &TraitMethodDeclaration<'_>,
    source: &str,
) {
    output.push_str(&format!("### `{}`\n\n", declaration.name.name));
    append_comment(output, source, declaration.span);
    output.push_str("```exs\n");
    output.push_str(&function_signature(
        &declaration.name.name,
        &declaration.parameters,
        declaration.return_type.as_ref(),
    ));
    output.push_str(if declaration.body.is_some() {
        " { ... }\n"
    } else {
        ";\n"
    });
    output.push_str("```\n\n");
}
