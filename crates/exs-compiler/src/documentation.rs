//! Markdown language and source-API documentation generation.

use std::collections::HashMap;
use std::path::Path;

use crate::ast::{
    FunctionDeclaration, ImplDeclaration, Module, Parameter, TraitDeclaration,
    TraitMethodDeclaration, TypeAnnotation, TypeDeclaration,
};
use crate::{Documentation, DocumentationModule, ModuleResolver, SourceInput, SourceSpan};

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
    let modules = files
        .iter()
        .map(|file| parse(&file.source_id, &file.text))
        .collect::<Result<Vec<_>, _>>()?;
    let paths = files
        .iter()
        .enumerate()
        .map(|(index, file)| module_path(index, &file.source_id))
        .collect::<Vec<_>>();
    let pages = modules
        .iter()
        .enumerate()
        .map(|(index, module)| DocumentationModule {
            source_id: files[index].source_id.clone(),
            path: paths[index].clone(),
            markdown: render_module(module, &files[index], &edges[index], &paths),
        })
        .collect::<Vec<_>>();
    Ok(Documentation {
        index: render_index(&pages, &files),
        modules: pages,
    })
}

/// Parses one documentation source unit without requiring a root entry point.
fn parse<'a>(source_id: &'a str, text: &'a str) -> Result<Module<'a>, String> {
    let lexed = crate::lexer::lex(SourceInput { source_id, text });
    if !lexed.diagnostics.is_empty() {
        return Err(lexed.diagnostics.render(text));
    }
    crate::parser::parse(source_id, lexed.tokens, false).map_err(|error| error.render(text))
}

/// Renders the project index and concise implemented language reference.
fn render_index(modules: &[DocumentationModule], files: &[SourceFile]) -> String {
    let mut output = String::from("# ExS API Documentation\n\n");
    output.push_str("This reference is generated from the root module and every reachable relative import. Declarations use adjacent `///` comments as their documentation.\n\n");
    output.push_str("## Language\n\n");
    output.push_str("ExS is a dynamically typed scripting language compiled to WebAssembly. A root module declares `fn main(...)`; imported modules declare reusable functions, nominal types, traits, and implementations. `host.call(name, args...)` invokes a runner-provided host function and may suspend. `par { ... }` runs fixed tasks concurrently, while `par(functions)` runs a List of zero-argument closures.\n\n");
    output.push_str("## Available Types\n\n");
    output.push_str("| Type | Purpose |\n| --- | --- |\n| `Any` | Omitted annotation; accepts every value. |\n| `None` | Absence value. |\n| `Error` | Recoverable or fatal language error value. |\n| `Bool` | Boolean value. |\n| `Int` | Signed 56-bit integer. |\n| `Float` | IEEE-754 floating-point value. |\n| `String` | UTF-8 string. |\n| `List` | Mutable ordered collection. |\n| `Object` | Mutable keyed collection. |\n| `Fn` | Callable closure contract. |\n\n");
    output.push_str("## Built-in Functions and Operations\n\n");
    output.push_str("- `Error(kind, message, data)`: constructs a recoverable Error.\n- `host.call(name, arguments...)`: invokes a runner-registered synchronous or asynchronous host function.\n- `value is Error`: tests whether a value is an Error.\n- `value?`: propagates an Error and converts None to MissingValue.\n\n");
    output.push_str("### List Methods\n\n`push(value)`, `pop()`, `insert(index, value)`, `remove(index)`, and `clear()`.\n\n");
    output
        .push_str("### Object Methods\n\n`has(key)`, `delete(key)`, `keys()`, and `values()`.\n\n");
    output.push_str("## Modules\n\n");
    for (module, file) in modules.iter().zip(files) {
        output.push_str(&format!("- [`{}`]({})\n", file.display_path, module.path));
    }
    output
}

/// Renders one source module's declarations and resolved import links.
fn render_module(
    module: &Module<'_>,
    source: &SourceFile,
    imports: &[ImportEdge],
    paths: &[String],
) -> String {
    let mut output = format!("# Module `{}`\n\n", source.display_path);
    if !imports.is_empty() {
        output.push_str("## Imports\n\n");
        for import in imports {
            let target = Path::new(&paths[import.target])
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(paths[import.target].as_str());
            output.push_str(&format!(
                "- `{}` -> [module]({})\n",
                import.namespace, target
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
    if !module.types.is_empty() {
        output.push_str("## Types\n\n");
        for declaration in &module.types {
            render_type(&mut output, declaration, &source.text);
        }
    }
    if !module.traits.is_empty() {
        output.push_str("## Traits\n\n");
        for declaration in &module.traits {
            render_trait(&mut output, declaration, &source.text);
        }
    }
    if !module.implementations.is_empty() {
        output.push_str("## Implementations\n\n");
        for declaration in &module.implementations {
            render_implementation(&mut output, declaration, &source.text);
        }
    }
    if !module.functions.is_empty() {
        output.push_str("## Functions\n\n");
        for declaration in &module.functions {
            render_function(&mut output, declaration, &source.text, 3);
        }
    }
    if module.types.is_empty()
        && module.traits.is_empty()
        && module.implementations.is_empty()
        && module.functions.is_empty()
    {
        output.push_str("No public declarations.\n");
    }
    output
}

/// Renders one nominal type and its declared fields.
fn render_type(output: &mut String, declaration: &TypeDeclaration<'_>, source: &str) {
    output.push_str(&format!("### `{}`\n\n", declaration.name.name));
    append_comment(output, source, declaration.span);
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
}

/// Renders one trait and its required/default method signatures.
fn render_trait(output: &mut String, declaration: &TraitDeclaration<'_>, source: &str) {
    output.push_str(&format!("### `{}`\n\n", declaration.name.name));
    append_comment(output, source, declaration.span);
    for method in &declaration.methods {
        render_trait_method(output, method, source);
    }
}

/// Renders one trait method signature.
fn render_trait_method(
    output: &mut String,
    declaration: &TraitMethodDeclaration<'_>,
    source: &str,
) {
    output.push_str(&format!("#### `{}`\n\n", declaration.name.name));
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

/// Renders one inherent or trait implementation and its method API.
fn render_implementation(output: &mut String, declaration: &ImplDeclaration<'_>, source: &str) {
    let heading = declaration.trait_name.as_ref().map_or_else(
        || format!("impl {}", declaration.type_name.name),
        |trait_name| {
            format!(
                "impl {} for {}",
                trait_name.name, declaration.type_name.name
            )
        },
    );
    output.push_str(&format!("### `{heading}`\n\n"));
    append_comment(output, source, declaration.span);
    for method in &declaration.methods {
        render_function(output, method, source, 4);
    }
}

/// Renders one direct or implementation function signature and its comment.
fn render_function(
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
    let lines = source[..start].lines().rev();
    let mut comment = Vec::new();
    for line in lines {
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

/// Builds a deterministic relative page path for one source module.
fn module_path(index: usize, source_id: &str) -> String {
    let stem = Path::new(source_id)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("module");
    let stem = stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("modules/{index:02}-{stem}.md")
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
