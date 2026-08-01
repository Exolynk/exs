//! Markdown language and source-API documentation generation.

use std::collections::HashMap;
use std::path::Path;

use crate::ast::{
    EnumDeclaration, FunctionDeclaration, Module, Parameter, TraitDeclaration,
    TraitMethodDeclaration, TypeAnnotation, TypeDeclaration,
};
use crate::{Documentation, DocumentationPage, ModuleResolver, SourceInput, SourceSpan};

/// One source file retained while its module graph is documented.
struct SourceFile {
    /// Resolver-provided canonical module identity.
    source_id: String,
    /// Source-defined path displayed in generated Markdown.
    display_path: String,
    /// Complete UTF-8 source text.
    text: String,
}

/// One resolved import edge used when rendering module links.
struct ImportEdge {
    /// Namespace available in the importing module.
    namespace: String,
    /// Resolved target module index.
    target: usize,
}

/// Generates Markdown documentation for one root source and its import graph.
pub(super) fn generate<R: ModuleResolver>(
    source: SourceInput<'_>,
    resolver: &mut R,
) -> Result<Documentation, String> {
    let (files, edges) = load_graph(source, resolver)?;
    let modules = files
        .iter()
        .map(|file| parse(&file.source_id, &file.text))
        .collect::<Result<Vec<_>, _>>()?;
    let directories = files
        .iter()
        .enumerate()
        .map(|(index, file)| module_directory(index, &file.source_id))
        .collect::<Vec<_>>();
    let mut pages = standard_pages();
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
        index: render_index(&files, &directories),
        pages,
    })
}

/// Loads one root source and its complete relative-import graph.
fn load_graph<R: ModuleResolver>(
    source: SourceInput<'_>,
    resolver: &mut R,
) -> Result<(Vec<SourceFile>, Vec<Vec<ImportEdge>>), String> {
    let mut files = vec![SourceFile {
        source_id: source.source_id.to_owned(),
        display_path: root_display_path(source.source_id),
        text: source.text.to_owned(),
    }];
    let mut edges = vec![Vec::new()];
    let mut indices = HashMap::from([(source.source_id.to_owned(), 0_usize)]);
    let mut index = 0;
    while index < files.len() {
        let source_id = files[index].source_id.clone();
        let text = files[index].text.clone();
        let module = parse(&source_id, &text)?;
        for import in module.imports {
            let resolved = resolver
                .resolve(&source_id, &import.path)
                .map_err(|error| {
                    format!(
                        "{}:{}-{}: could not resolve import `{}`: {error}",
                        source_id, import.span.start_byte, import.span.end_byte, import.path
                    )
                })?;
            let target = if let Some(target) = indices.get(&resolved.source_id) {
                *target
            } else {
                let target = files.len();
                indices.insert(resolved.source_id.clone(), target);
                files.push(SourceFile {
                    source_id: resolved.source_id,
                    display_path: import.path.clone(),
                    text: resolved.text,
                });
                edges.push(Vec::new());
                target
            };
            edges[index].push(ImportEdge {
                namespace: import
                    .alias
                    .map_or_else(|| default_namespace(&import.path), |alias| alias.name),
                target,
            });
        }
        index += 1;
    }
    if let Some(cycle) = find_cycle(&edges) {
        let chain = cycle
            .iter()
            .map(|index| files[*index].source_id.as_str())
            .collect::<Vec<_>>()
            .join(" -> ");
        return Err(format!("{}: import cycle: {chain}", files[0].source_id));
    }
    Ok((files, edges))
}

/// Parses one documentation source unit without requiring a root entry point.
fn parse<'a>(source_id: &'a str, text: &'a str) -> Result<Module<'a>, String> {
    let lexed = crate::lexer::lex(SourceInput { source_id, text });
    if !lexed.diagnostics.is_empty() {
        return Err(lexed.diagnostics.render(text));
    }
    crate::parser::parse(source_id, lexed.tokens, false).map_err(|error| error.render(text))
}

/// Renders the project index and concise language overview.
fn render_index(files: &[SourceFile], directories: &[String]) -> String {
    let mut output = String::from("# ExS API Documentation\n\n");
    output.push_str("This reference is generated from the root module and every reachable relative import. Adjacent `///` comments describe source declarations.\n\n");
    output.push_str("## Language\n\n");
    output.push_str("ExS is a dynamically typed scripting language compiled to WebAssembly. Root modules declare `fn main(...)`; imported modules provide functions, nominal types, traits, and implementations. `host.call(name, args...)` invokes a runner-provided host function and may suspend. `par { ... }` runs fixed tasks concurrently, while `par(functions)` runs a List of zero-argument closures.\n\n");
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
    source: &SourceFile,
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
    source: &SourceFile,
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
fn render_enum_page(
    module: &Module<'_>,
    declaration: &EnumDeclaration<'_>,
    source: &SourceFile,
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
fn render_type_page(
    module: &Module<'_>,
    declaration: &TypeDeclaration<'_>,
    source: &SourceFile,
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
    output
}

/// Renders all inherent and trait-provided methods of one nominal declaration.
fn render_nominal_implementations(
    output: &mut String,
    module: &Module<'_>,
    name: &str,
    source: &SourceFile,
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
                |trait_name| format!("Trait `{}`", trait_name.name),
            );
            output.push_str(&format!("### {label}\n\n"));
            for method in &implementation.methods {
                render_function_details(output, method, &source.text, 4);
            }
        }
    }
}

/// Renders one user-defined trait and all of its method details.
fn render_trait_page(declaration: &TraitDeclaration<'_>, source: &SourceFile) -> String {
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
    source: &SourceFile,
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
) {
    output.push_str(&format!(
        "{} `{}`\n\n",
        "#".repeat(level),
        declaration.name.name
    ));
    append_comment(output, source, declaration.span);
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

/// Generates the synthetic standard-library module and its declaration pages.
fn standard_pages() -> Vec<DocumentationPage> {
    let types = [
        (
            "Any",
            "The unconstrained annotation accepted when a type annotation is omitted.",
            &[][..],
        ),
        ("None", "The globally available absence value.", &[][..]),
        (
            "Error",
            "A structured recoverable language error value.",
            &[][..],
        ),
        ("Bool", "A globally available Boolean value.", &[][..]),
        (
            "Int",
            "A globally available signed 56-bit integer value.",
            &[][..],
        ),
        (
            "Float",
            "A globally available IEEE-754 floating-point value.",
            &[][..],
        ),
        (
            "String",
            "A globally available UTF-8 string value.",
            &[][..],
        ),
        (
            "List",
            "A globally available mutable ordered collection.",
            &[
                "push(value) - appends one value and returns the new length.",
                "pop() - removes and returns the final value, or None.",
                "insert(index, value) - inserts one value.",
                "remove(index) - removes and returns one value.",
                "clear() - removes every value.",
            ][..],
        ),
        (
            "Object",
            "A globally available mutable keyed collection.",
            &[
                "has(key) - returns whether the key exists.",
                "delete(key) - removes and returns the key value, or None.",
                "keys() - returns keys in insertion order.",
                "values() - returns values in insertion order.",
            ][..],
        ),
        (
            "Fn",
            "The callable closure contract used in type annotations.",
            &[][..],
        ),
    ];
    let mut pages = vec![DocumentationPage {
        path: "modules/std/index.md".to_owned(),
        markdown: render_standard_index(&types),
    }];
    for (name, description, methods) in types {
        pages.push(DocumentationPage {
            path: format!("modules/std/types/{}.md", slug(name)),
            markdown: if name == "Error" {
                render_standard_error_type(description)
            } else {
                render_standard_type(name, description, methods)
            },
        });
    }
    pages.push(standard_function_page(
        "host-call",
        "host.call",
        "host.call(name, arguments...)",
        "Invokes a runner-registered host function. `name` must evaluate to String. A host call may suspend and returns its value or an Error.",
    ));
    pages
}

/// Renders the synthetic standard-library module index.
fn render_standard_index(types: &[(&str, &str, &[&str])]) -> String {
    let mut output = String::from(
        "# Module `std`\n\nBuilt-in types are globally available in ExS source and may also be written with the `std::` qualifier. Importing `std` is not required or allowed.\n\n## Types\n\n",
    );
    for (name, _, _) in types {
        output.push_str(&format!("- [`{name}`](types/{}.md)\n", slug(name)));
    }
    output.push_str("\n## Functions\n\n- [`host.call`](fn/host-call.md)\n");
    output
}

/// Renders the Error type page with its source-level constructor.
fn render_standard_error_type(description: &str) -> String {
    let mut output = render_standard_type("Error", description, &[]);
    output.push_str("\n## Constructor\n\n```exs\nError(kind, message, data)\n```\n\nConstructs a recoverable Error. `kind` and `message` must be Strings; `data` may be any value.\n");
    output
}

/// Renders one synthetic standard-library type page.
fn render_standard_type(name: &str, description: &str, methods: &[&str]) -> String {
    let mut output = format!("# Type `std::{name}`\n\n{description}\n\n");
    output.push_str("```exs\n");
    output.push_str(&format!("type {name}\n```\n"));
    if !methods.is_empty() {
        output.push_str("\n## Implemented Methods\n\n");
        for method in methods {
            let (signature, description) = method.split_once(" - ").unwrap_or((method, ""));
            output.push_str(&format!("### `{signature}`\n\n{description}\n\n"));
        }
    }
    output
}

/// Builds one synthetic standard-library function page.
fn standard_function_page(
    path: &str,
    name: &str,
    signature: &str,
    description: &str,
) -> DocumentationPage {
    DocumentationPage {
        path: format!("modules/std/fn/{path}.md"),
        markdown: format!("# Function `{name}`\n\n```exs\n{signature}\n```\n\n{description}\n"),
    }
}

/// Renders one declaration signature without its executable body.
fn function_signature(
    name: &str,
    parameters: &[Parameter<'_>],
    return_type: Option<&TypeAnnotation<'_>>,
) -> String {
    let parameters = parameters
        .iter()
        .map(|parameter| {
            parameter.type_annotation.as_ref().map_or_else(
                || parameter.name.name.clone(),
                |annotation| format!("{}: {}", parameter.name.name, type_annotation(annotation)),
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let return_type = return_type.map_or_else(String::new, |annotation| {
        format!(" -> {}", type_annotation(annotation))
    });
    format!("fn {name}({parameters}){return_type}")
}

/// Renders one union type annotation.
fn type_annotation(annotation: &TypeAnnotation<'_>) -> String {
    annotation
        .members
        .iter()
        .map(|member| member.name.as_str())
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Appends the consecutive preceding `///` documentation comment, if present.
fn append_comment(output: &mut String, source: &str, span: SourceSpan<'_>) {
    let start = usize::try_from(span.start_byte)
        .unwrap_or_default()
        .min(source.len());
    let mut comment = Vec::new();
    for line in source[..start].lines().rev() {
        let line = line.trim_start();
        let Some(line) = line.strip_prefix("///") else {
            break;
        };
        comment.push(line.strip_prefix(' ').unwrap_or(line).to_owned());
    }
    if !comment.is_empty() {
        comment.reverse();
        output.push_str(&comment.join("\n"));
        output.push_str("\n\n");
    }
}

/// Builds a deterministic documentation directory for one source module.
fn module_directory(index: usize, source_id: &str) -> String {
    format!(
        "modules/{index:02}-{}",
        slug(
            Path::new(source_id)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("module")
        )
    )
}

/// Produces a portable lowercase page-name segment.
fn slug(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
}

/// Converts an absolute root source identity into a concise local documentation label.
fn root_display_path(source_id: &str) -> String {
    let path = Path::new(source_id);
    if path.is_absolute() {
        path.file_name()
            .and_then(|name| name.to_str())
            .map_or_else(|| source_id.to_owned(), |name| format!("./{name}"))
    } else {
        source_id.to_owned()
    }
}

/// Derives the default namespace from a relative `.exs` import path.
fn default_namespace(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .strip_suffix(".exs")
        .unwrap_or(path)
        .to_owned()
}

/// Finds one directed cycle in the documentation import graph.
fn find_cycle(edges: &[Vec<ImportEdge>]) -> Option<Vec<usize>> {
    fn visit(
        node: usize,
        edges: &[Vec<ImportEdge>],
        states: &mut [u8],
        stack: &mut Vec<usize>,
    ) -> Option<Vec<usize>> {
        states[node] = 1;
        stack.push(node);
        for edge in &edges[node] {
            if states[edge.target] == 1 {
                let start = stack.iter().position(|item| *item == edge.target)?;
                let mut cycle = stack[start..].to_vec();
                cycle.push(edge.target);
                return Some(cycle);
            }
            if states[edge.target] == 0
                && let Some(cycle) = visit(edge.target, edges, states, stack)
            {
                return Some(cycle);
            }
        }
        stack.pop();
        states[node] = 2;
        None
    }
    let mut states = vec![0; edges.len()];
    let mut stack = Vec::new();
    for node in 0..edges.len() {
        if states[node] == 0
            && let Some(cycle) = visit(node, edges, &mut states, &mut stack)
        {
            return Some(cycle);
        }
    }
    None
}
