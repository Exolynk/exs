//! Markdown language and source-API documentation generation.

use std::collections::HashMap;
use std::path::Path;

use crate::ast::{
    EnumDeclaration, FunctionDeclaration, Module, Parameter, TraitDeclaration,
    TraitMethodDeclaration, TypeAnnotation, TypeDeclaration,
};
use crate::codegen::standard::{self, StandardEnumDescriptor, StandardTraitDescriptor};
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

/// One source-visible standard-library type and its documentation metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StandardType {
    /// Source-visible built-in type name.
    pub name: &'static str,
    /// Overview rendered before the type declaration.
    pub description: &'static str,
    /// Short valid ExS example that introduces the type.
    pub usage: &'static str,
    /// Runtime-owned methods exposed by this type.
    pub methods: &'static [StandardMethod],
}

/// One documented runtime-owned standard-library method.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StandardMethod {
    /// Source-level method signature.
    pub signature: &'static str,
    /// Detailed observable behavior and error conditions.
    pub description: &'static str,
    /// Short valid ExS example of a call to the method.
    pub example: &'static str,
}

/// One globally callable standard-library function and its documentation metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StandardFunction {
    /// Source-visible function name.
    pub name: &'static str,
    /// Source-level call signature.
    pub signature: &'static str,
    /// Detailed observable behavior and error conditions.
    pub description: &'static str,
    /// Short valid ExS example of a call to the function.
    pub example: &'static str,
}

/// One source-visible standard-library namespace and its static operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StandardNamespace {
    /// Source-visible namespace name.
    pub name: &'static str,
    /// Overview rendered before the namespace operations.
    pub description: &'static str,
    /// Static operations available through the namespace separator.
    pub functions: &'static [StandardFunction],
}

/// One source-visible standard-library trait.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StandardTrait {
    /// Source-visible trait name.
    pub name: &'static str,
}

/// One source-visible standard-library enum and its variants.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StandardEnum {
    /// Source-visible enum name.
    pub name: &'static str,
    /// Source-visible variants in declaration order.
    pub variants: &'static [&'static str],
}

/// Returns every globally callable standard-library function.
#[must_use]
pub fn standard_library_functions() -> &'static [StandardFunction] {
    &[StandardFunction {
        name: "Error",
        signature: "Error(kind, message, data)",
        description: "Constructs a recoverable Error with a stable category, a human-readable message, and any related language value. `kind` and `message` must be Strings. The constructor does not assign a cause; `cause()` consequently returns None for directly constructed Errors.",
        example: "let error = Error(\"InvalidInput\", \"age must be positive\", -1);\nret error.message();",
    }]
}

/// Returns every source-visible standard-library namespace.
#[must_use]
pub fn standard_library_namespaces() -> &'static [StandardNamespace] {
    &[
        StandardNamespace {
            name: "test",
            description: "Test assertions available directly in every module and through the std::test namespace.",
            functions: &[
                StandardFunction {
                    name: "assert",
                    signature: "std::test::assert(condition: Bool[, description: String]) -> None",
                    description: "Returns None when condition is true. When condition is false, it creates a fatal AssertionFailed Error with the supplied description, or the default message \"assert failed\" when omitted. The direct assert spelling is equivalent.",
                    example: "assert(total > 0, \"total must be positive\");",
                },
                StandardFunction {
                    name: "assert_eq",
                    signature: "std::test::assert_eq(actual: Any, expected: Any[, description: String]) -> None",
                    description: "Returns None when actual and expected compare equal with ExS equality semantics. Otherwise it creates a fatal AssertionFailed Error whose data contains actual and expected values and uses the supplied description, or the default message \"assert_eq failed\" when omitted. The direct assert_eq spelling is equivalent.",
                    example: "assert_eq(total, 42, \"total must include every line item\");",
                },
            ],
        },
        StandardNamespace {
            name: "Host",
            description: "The runner-provided boundary for application-specific operations.",
            functions: &[
                StandardFunction {
                    name: "call",
                    signature: "Host::call(name, arguments...)",
                    description: "Invokes a runner-registered host function selected by a runtime String name. Arguments are collected into a List and transported through the runner CBOR boundary. A call may complete immediately or suspend; it returns the host result or a recoverable Error such as `HostFunctionNotFound`.",
                    example: "let greeting = Host::call(\"greet\", \"Ada\");\nret greeting;",
                },
                StandardFunction {
                    name: "sleep",
                    signature: "Host::sleep(duration: Duration) -> None",
                    description: "Suspends the current ExS task until the supplied normalized Duration elapses, then returns None. This capability is built into every runner and does not use the application host-function registry.",
                    example: "Host::sleep(Duration::milliseconds(250));\nret None;",
                },
                StandardFunction {
                    name: "now",
                    signature: "Host::now() -> DateTime",
                    description: "Returns a snapshot of the runner wall clock. The result always carries the Unix instant and observed UTC offset; `timezone` contains the runner-resolved IANA name when available.",
                    example: "let now = Host::now();\nret now.unix_seconds;",
                },
                StandardFunction {
                    name: "elapsed",
                    signature: "Host::elapsed() -> Duration",
                    description: "Returns monotonic time elapsed since this root execution began. It is unaffected by wall-clock changes and is suitable for measuring work inside one execution.",
                    example: "Host::sleep(Duration::milliseconds(250));\nret Host::elapsed();",
                },
                StandardFunction {
                    name: "stream",
                    signature: "Host::stream(name, arguments...) -> HostStream | Error",
                    description: "Opens a runner-registered pull stream selected by a runtime String name. Arguments are collected into a List and passed to the stream factory without the name. HostStream implements Iterator; each advance may suspend, and the runner drops the stream after IteratorStep::Done or execution cleanup.",
                    example: "let events = Host::stream(\"events.subscribe\", user_id)?;\nfor event in events {\n    Host::call(\"events.record\", event);\n}",
                },
            ],
        },
    ]
}

/// Returns one standard namespace by its source-visible name.
#[must_use]
pub fn standard_library_namespace(name: &str) -> Option<&'static StandardNamespace> {
    standard_library_namespaces()
        .iter()
        .find(|namespace| namespace.name == name)
}

/// Returns static functions declared by one documented standard type.
#[must_use]
pub fn standard_library_type_static_functions(name: &str) -> &'static [StandardFunction] {
    match name {
        "Duration" => &[
            StandardFunction {
                name: "nanoseconds",
                signature: "Duration::nanoseconds(value: Int) -> Duration | Error",
                description: "Constructs a Duration from a non-negative exact nanosecond count. Negative input returns ValueError.",
                example: "let pause = Duration::nanoseconds(500);",
            },
            StandardFunction {
                name: "microseconds",
                signature: "Duration::microseconds(value: Int) -> Duration | Error",
                description: "Constructs a Duration from a non-negative exact microsecond count. Negative input returns ValueError and conversion overflow returns IntOverflowError.",
                example: "let pause = Duration::microseconds(500);",
            },
            StandardFunction {
                name: "milliseconds",
                signature: "Duration::milliseconds(value: Int) -> Duration | Error",
                description: "Constructs a Duration from a non-negative exact millisecond count. Negative input returns ValueError and non-Int input returns TypeError.",
                example: "let timeout = Duration::milliseconds(500);",
            },
            StandardFunction {
                name: "seconds",
                signature: "Duration::seconds(value: Int) -> Duration | Error",
                description: "Constructs a Duration from a non-negative exact second count. Negative input returns ValueError.",
                example: "let interval = Duration::seconds(2);",
            },
        ],
        _ => &[],
    }
}

/// Returns every documented source-visible standard-library trait.
#[must_use]
pub fn standard_library_traits() -> Vec<StandardTrait> {
    standard::traits()
        .iter()
        .map(|descriptor| StandardTrait {
            name: descriptor.name,
        })
        .collect()
}

/// Returns every documented source-visible standard-library enum.
#[must_use]
pub fn standard_library_enums() -> Vec<StandardEnum> {
    standard::enums()
        .iter()
        .map(|descriptor| StandardEnum {
            name: descriptor.name,
            variants: descriptor.variants,
        })
        .collect()
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
    output.push_str("\n## Runtime Methods\n\n");
    render_clone_method(&mut output, &declaration.name.name);
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
    if let Some(descriptor) = standard::trait_descriptor(trait_name) {
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
    source: &SourceFile,
) -> Option<String> {
    if let Some(descriptor) = standard::trait_descriptor(trait_name) {
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

/// Returns every documented source-visible standard-library type.
#[must_use]
pub fn standard_library_types() -> Vec<StandardType> {
    vec![
        StandardType {
            name: "Any",
            description: "`Any` accepts every ExS value. It is the implicit contract when a parameter or return annotation is omitted, and is useful when a function deliberately forwards values without narrowing their type.",
            usage: "fn main(value: Any) -> Any {\n    ret value;\n}",
            methods: &[],
        },
        StandardType {
            name: "None",
            description: "`None` is the single absence value. It is used for missing optional results, empty mutations, and Object reads whose key does not exist; ExS has no `null` source literal.",
            usage: "fn main() -> None {\n    let absent = None;\n    ret absent;\n}",
            methods: &[],
        },
        StandardType {
            name: "Duration",
            description: "`Duration` is a normal prelude type representing a non-negative interval as normalized `seconds` and `nanoseconds` Int fields. Create values through its static factories before passing them to `Host::sleep`.",
            usage: "fn main() -> None {\n    let pause = Duration::milliseconds(250);\n    Host::sleep(pause);\n    ret None;\n}",
            methods: &[
                StandardMethod {
                    signature: "as_seconds() -> Int",
                    description: "Returns the normalized whole-second component. Any fractional second remains available through the `nanoseconds` field.",
                    example: "let seconds = Duration::milliseconds(1500).as_seconds(); // 1",
                },
                StandardMethod {
                    signature: "as_milliseconds() -> Int | Error",
                    description: "Returns the total duration truncated to whole milliseconds. IntOverflowError is returned when the exact total cannot fit in an Int.",
                    example: "let milliseconds = Duration::nanoseconds(1_500_000).as_milliseconds(); // 1",
                },
                StandardMethod {
                    signature: "as_microseconds() -> Int | Error",
                    description: "Returns the total duration truncated to whole microseconds. IntOverflowError is returned when the exact total cannot fit in an Int.",
                    example: "let microseconds = Duration::nanoseconds(1_500).as_microseconds(); // 1",
                },
                StandardMethod {
                    signature: "as_nanoseconds() -> Int | Error",
                    description: "Returns the total duration as exact nanoseconds. IntOverflowError is returned when the exact total cannot fit in an Int.",
                    example: "let nanoseconds = Duration::milliseconds(1).as_nanoseconds(); // 1_000_000",
                },
            ],
        },
        StandardType {
            name: "Error",
            description: "`Error` is a structured language failure value. Operations return it instead of throwing, and functions that may return an Error should include `Error` in their return contract or use the implicit `Any` contract.",
            usage: "fn main() -> Int | Error {\n    ret Error(\"DivisionByZeroError\", \"cannot divide by zero\", None);\n}",
            methods: &[
                StandardMethod {
                    signature: "kind() -> String",
                    description: "Returns the stable machine-readable category assigned when the Error was created. Use it to distinguish expected failures without parsing a human-facing message.",
                    example: "let error = Error(\"MissingValue\", \"value is required\", None);\nlet category = error.kind();",
                },
                StandardMethod {
                    signature: "message() -> String",
                    description: "Returns the human-readable explanation stored in the Error. The message is intended for diagnostics and user-facing reporting, not control-flow classification.",
                    example: "let error = Error(\"MissingValue\", \"value is required\", None);\nlet text = error.message();",
                },
                StandardMethod {
                    signature: "data() -> Any",
                    description: "Returns the language value attached to the Error. Runtime operations use this to retain the invalid input, index, or other relevant context that caused the failure.",
                    example: "let error = Error(\"InvalidInput\", \"age must be positive\", -1);\nlet invalid_age = error.data();",
                },
                StandardMethod {
                    signature: "cause() -> Error | None",
                    description: "Returns a related prior Error or value when one is present. Errors created directly with `Error(...)` have no cause and therefore return None.",
                    example: "let error = Error(\"Example\", \"no prior failure\", None);\nlet previous = error.cause();",
                },
            ],
        },
        StandardType {
            name: "Bool",
            description: "`Bool` has exactly the values `true` and `false`. Conditions require Bool explicitly; ExS does not apply implicit truthiness to numbers, strings, collections, or None.",
            usage: "fn main() {\n    let ready = true;\n    if ready {\n        Host::call(\"println\", \"ready\");\n    }\n}",
            methods: &[],
        },
        StandardType {
            name: "Int",
            description: "`Int` is a signed 64-bit exact integer. It supports numeric operators and reports `IntOverflowError` when an operation overflows the ExS integer range.",
            usage: "fn main() -> Int {\n    let quantity = 42;\n    ret quantity + 8;\n}",
            methods: &[
                StandardMethod {
                    signature: "abs() -> Int",
                    description: "Returns the non-negative magnitude of the receiver. The smallest representable Int has no representable positive counterpart, so calling `abs()` on it returns `IntOverflowError`.",
                    example: "let change = -42;\nlet magnitude = change.abs(); // 42",
                },
                StandardMethod {
                    signature: "div_euclid(other: Int) -> Int | Error",
                    description: "Returns the Euclidean quotient of two Int values. The result satisfies `self == other * quotient + self.rem_euclid(other)`. A zero divisor returns DivisionByZeroError and the only overflowing pair, the smallest Int divided by -1, returns IntOverflowError.",
                    example: "let seconds = 1001.div_euclid(1000); // 1",
                },
                StandardMethod {
                    signature: "rem_euclid(other: Int) -> Int | Error",
                    description: "Returns the non-negative Euclidean remainder of two Int values. A zero divisor returns DivisionByZeroError and the only overflowing pair, the smallest Int divided by -1, returns IntOverflowError.",
                    example: "let milliseconds = 1001.rem_euclid(1000); // 1",
                },
            ],
        },
        StandardType {
            name: "Float",
            description: "`Float` uses IEEE-754 binary64 values, including infinities, signed zero, and NaN. Mixed arithmetic promotes the other numeric operand to Float.",
            usage: "fn main() -> Float {\n    let price = 19.95;\n    ret price * 1.19;\n}",
            methods: &[
                StandardMethod {
                    signature: "abs() -> Float",
                    description: "Returns the non-negative floating-point magnitude. It preserves Float semantics for signed zero, infinities, and NaN.",
                    example: "let delta = -1.5;\nlet magnitude = delta.abs(); // 1.5",
                },
                StandardMethod {
                    signature: "floor() -> Float",
                    description: "Rounds down to the greatest integral Float that is not greater than the receiver. The result remains Float so it composes with floating-point arithmetic.",
                    example: "let page = 3.8;\nlet first_index = page.floor(); // 3.0",
                },
                StandardMethod {
                    signature: "ceil() -> Float",
                    description: "Rounds up to the least integral Float that is not less than the receiver. The result remains Float.",
                    example: "let pages = 3.2;\nlet required = pages.ceil(); // 4.0",
                },
                StandardMethod {
                    signature: "round() -> Float",
                    description: "Rounds to the nearest integral Float. Exact halfway values are rounded away from zero, so `1.5` becomes `2.0` and `-1.5` becomes `-2.0`.",
                    example: "let rating = 4.5;\nlet displayed = rating.round(); // 5.0",
                },
            ],
        },
        StandardType {
            name: "String",
            description: "`String` is an immutable UTF-8 sequence. Indexing and `length()` operate on Unicode scalar values rather than UTF-8 byte positions.",
            usage: "fn main() -> String {\n    let greeting = \"Hello\";\n    ret greeting[0];\n}",
            methods: &[
                StandardMethod {
                    signature: "length() -> Int",
                    description: "Returns the number of Unicode scalar values in the String. This is not the UTF-8 byte length, so a single emoji scalar counts as one.",
                    example: "let symbol = \"🙂\";\nlet count = symbol.length(); // 1",
                },
                StandardMethod {
                    signature: "is_empty() -> Bool",
                    description: "Returns true when the String contains no Unicode scalar values and false otherwise. It does not trim or normalize the String.",
                    example: "let input = \"\";\nif input.is_empty() {\n    Host::call(\"println\", \"missing input\");\n}",
                },
            ],
        },
        StandardType {
            name: "List",
            description: "`List` is a mutable ordered collection. Variables and closure captures preserve List identity, so mutations through one alias are visible through every alias of the same List.",
            usage: "fn main() -> Int {\n    let items = [\"Ada\", \"Lin\"];\n    ret items.push(\"Mia\");\n}",
            methods: &[
                StandardMethod {
                    signature: "length() -> Int",
                    description: "Returns the current number of elements. The count changes immediately after List mutations such as `push`, `pop`, `insert`, `remove`, and `clear`.",
                    example: "let items = [\"Ada\", \"Lin\"];\nlet count = items.length(); // 2",
                },
                StandardMethod {
                    signature: "is_empty() -> Bool",
                    description: "Returns true when the List has no elements. It does not mutate the List.",
                    example: "let queue = [];\nif queue.is_empty() {\n    Host::call(\"println\", \"queue is empty\");\n}",
                },
                StandardMethod {
                    signature: "push(value) -> Int",
                    description: "Appends one value to the end of the List and returns the new element count. The operation mutates the existing List rather than allocating a replacement.",
                    example: "let items = [\"Ada\"];\nlet count = items.push(\"Lin\"); // 2",
                },
                StandardMethod {
                    signature: "pop() -> Any | None",
                    description: "Removes and returns the final element. Calling `pop()` on an empty List leaves it unchanged and returns None.",
                    example: "let items = [\"Ada\", \"Lin\"];\nlet last = items.pop(); // \"Lin\"",
                },
                StandardMethod {
                    signature: "insert(index: Int, value) -> None | Error",
                    description: "Inserts one value before the zero-based index and returns None. The index may equal the current length to append; invalid indexes return `IndexError`.",
                    example: "let items = [\"Ada\", \"Mia\"];\nitems.insert(1, \"Lin\"); // [\"Ada\", \"Lin\", \"Mia\"]",
                },
                StandardMethod {
                    signature: "remove(index: Int) -> Any | Error",
                    description: "Removes and returns the element at a zero-based index. Invalid indexes return `IndexError` and leave the List unchanged.",
                    example: "let items = [\"Ada\", \"Lin\"];\nlet removed = items.remove(0); // \"Ada\"",
                },
                StandardMethod {
                    signature: "clear() -> None",
                    description: "Removes every element from the existing List and returns None. Aliases continue to refer to the now-empty same List.",
                    example: "let items = [1, 2, 3];\nitems.clear();\nlet empty = items.is_empty(); // true",
                },
            ],
        },
        StandardType {
            name: "Object",
            description: "`Object` is a mutable insertion-ordered mapping from String keys to values. Dot properties and bracket access operate on the same ordered collection.",
            usage: "fn main() -> Object {\n    let user = { name: \"Ada\" };\n    user.role = \"admin\";\n    ret user;\n}",
            methods: &[
                StandardMethod {
                    signature: "length() -> Int",
                    description: "Returns the number of present keys. Replacing an existing key does not increase the count; creating or deleting a key does.",
                    example: "let user = { name: \"Ada\" };\nlet count = user.length(); // 1",
                },
                StandardMethod {
                    signature: "is_empty() -> Bool",
                    description: "Returns true when the Object has no keys. It does not mutate the Object.",
                    example: "let options = {};\nif options.is_empty() {\n    Host::call(\"println\", \"using defaults\");\n}",
                },
                StandardMethod {
                    signature: "has(key: String) -> Bool | Error",
                    description: "Returns whether a String key is present. A non-String key returns `TypeError` rather than coercing the key.",
                    example: "let user = { name: \"Ada\" };\nlet has_name = user.has(\"name\"); // true",
                },
                StandardMethod {
                    signature: "delete(key: String) -> Any | None | Error",
                    description: "Removes a String key and returns its previous value. When the key is absent, it returns None; a non-String key returns `TypeError`.",
                    example: "let user = { name: \"Ada\" };\nlet name = user.delete(\"name\"); // \"Ada\"",
                },
                StandardMethod {
                    signature: "keys() -> List",
                    description: "Returns a new List of String keys in insertion order. Changing the returned List does not change the Object.",
                    example: "let user = { name: \"Ada\", role: \"admin\" };\nlet keys = user.keys(); // [\"name\", \"role\"]",
                },
                StandardMethod {
                    signature: "values() -> List",
                    description: "Returns a new shallow List of values in the same insertion order as `keys()`. The values themselves retain their original identity.",
                    example: "let user = { name: \"Ada\", role: \"admin\" };\nlet values = user.values(); // [\"Ada\", \"admin\"]",
                },
            ],
        },
        StandardType {
            name: "Fn",
            description: "`Fn` is the callable closure contract used in annotations. A closure captures lexical bindings and can be called with its declared parameter count.",
            usage: "fn apply(function: Fn, value: Int) -> Int {\n    ret function(value);\n}\n\nfn main() -> Int {\n    let increment = (value) => { ret value + 1; };\n    ret apply(increment, 1);\n}",
            methods: &[],
        },
    ]
}

/// Generates runtime-owned and source-prelude declaration pages for the standard library.
fn standard_pages() -> Result<Vec<DocumentationPage>, String> {
    let prelude_modules = standard_prelude_modules()?;
    let prelude_type_names = prelude_modules
        .iter()
        .flat_map(|(module, _)| {
            module
                .types
                .iter()
                .map(|declaration| declaration.name.name.as_str())
        })
        .collect::<Vec<_>>();
    let prelude_enum_names = prelude_modules
        .iter()
        .flat_map(|(module, _)| {
            module
                .enums
                .iter()
                .map(|declaration| declaration.name.name.as_str())
        })
        .collect::<Vec<_>>();
    let prelude_trait_names = prelude_modules
        .iter()
        .flat_map(|(module, _)| {
            module
                .traits
                .iter()
                .map(|declaration| declaration.name.name.as_str())
        })
        .collect::<Vec<_>>();
    let types = standard_library_types()
        .into_iter()
        .filter(|type_info| !prelude_type_names.contains(&type_info.name))
        .collect::<Vec<_>>();
    let functions = standard_library_functions();
    let namespaces = standard_library_namespaces();
    let traits = standard::traits()
        .iter()
        .filter(|descriptor| !prelude_trait_names.contains(&descriptor.name))
        .collect::<Vec<_>>();
    let enums = standard::enums()
        .iter()
        .filter(|descriptor| !prelude_enum_names.contains(&descriptor.name))
        .collect::<Vec<_>>();
    let type_names = types
        .iter()
        .map(|type_info| type_info.name)
        .chain(prelude_type_names.iter().copied())
        .collect::<Vec<_>>();
    let trait_names = traits
        .iter()
        .map(|descriptor| descriptor.name)
        .chain(prelude_trait_names.iter().copied())
        .collect::<Vec<_>>();
    let enum_names = enums
        .iter()
        .map(|descriptor| descriptor.name)
        .chain(prelude_enum_names.iter().copied())
        .collect::<Vec<_>>();
    let mut pages = vec![DocumentationPage {
        path: "modules/std/index.md".to_owned(),
        markdown: render_standard_index(
            &type_names,
            functions,
            namespaces,
            &enum_names,
            &trait_names,
        ),
    }];
    for type_info in &types {
        pages.push(DocumentationPage {
            path: format!("modules/std/types/{}.md", slug(type_info.name)),
            markdown: if type_info.name == "Error" {
                render_standard_error_type(type_info)
            } else {
                render_standard_type(type_info)
            },
        });
    }
    for trait_info in traits {
        pages.push(standard_trait_page(trait_info));
    }
    for enum_info in enums {
        pages.push(DocumentationPage {
            path: format!("modules/std/enums/{}.md", slug(enum_info.name)),
            markdown: render_standard_enum(enum_info),
        });
    }
    for namespace in namespaces {
        pages.push(standard_namespace_page(namespace));
    }
    for (module, source) in &prelude_modules {
        pages.extend(standard_prelude_pages(module, source));
    }
    Ok(pages)
}

/// Parses every bundled ExS prelude source for standard-library documentation rendering.
fn standard_prelude_modules() -> Result<Vec<(Module<'static>, SourceFile)>, String> {
    crate::prelude::source_inputs()
        .into_iter()
        .map(|source| {
            let module = parse(source.source_id, source.text)?;
            Ok((
                module,
                SourceFile {
                    source_id: source.source_id.to_owned(),
                    display_path: source.source_id.to_owned(),
                    text: source.text.to_owned(),
                },
            ))
        })
        .collect()
}

/// Builds standard-library declaration pages from one bundled ExS prelude source.
fn standard_prelude_pages(module: &Module<'_>, source: &SourceFile) -> Vec<DocumentationPage> {
    let mut pages = Vec::new();
    for declaration in &module.types {
        pages.push(DocumentationPage {
            path: format!("modules/std/types/{}.md", slug(&declaration.name.name)),
            markdown: render_type_page(module, declaration, source),
        });
    }
    for declaration in &module.enums {
        pages.push(DocumentationPage {
            path: format!("modules/std/enums/{}.md", slug(&declaration.name.name)),
            markdown: render_enum_page(module, declaration, source),
        });
    }
    for declaration in &module.traits {
        pages.push(DocumentationPage {
            path: format!("modules/std/traits/{}.md", slug(&declaration.name.name)),
            markdown: render_trait_page(declaration, source),
        });
    }
    pages
}

/// Renders the synthetic standard-library module index.
fn render_standard_index(
    types: &[&str],
    functions: &[StandardFunction],
    namespaces: &[StandardNamespace],
    enums: &[&str],
    traits: &[&str],
) -> String {
    let mut output = String::from(
        "# Module `std`\n\nBuilt-in standard items are globally available in ExS source and may also be written with the `std::` qualifier. Importing `std` is not required or allowed.\n\n## Types\n\n",
    );
    for type_name in types {
        output.push_str(&format!(
            "- [`{}`](types/{}.md)\n",
            type_name,
            slug(type_name)
        ));
    }
    if !namespaces.is_empty() {
        output.push_str("\n## Namespaces\n\n");
        for namespace in namespaces {
            output.push_str(&format!(
                "- [`{}`](namespaces/{}.md)\n",
                namespace.name,
                slug(namespace.name)
            ));
        }
    }
    if !enums.is_empty() {
        output.push_str("\n## Enums\n\n");
        for enum_name in enums {
            output.push_str(&format!(
                "- [`{}`](enums/{}.md)\n",
                enum_name,
                slug(enum_name)
            ));
        }
    }
    output.push_str("\n## Traits\n\n");
    for trait_name in traits {
        output.push_str(&format!(
            "- [`{}`](traits/{}.md)\n",
            trait_name,
            slug(trait_name)
        ));
    }
    output.push_str("\n## Functions\n\n");
    for function in functions {
        if function.name == "Error" {
            output.push_str("- [`Error`](types/error.md#constructor)\n");
        } else {
            render_standard_function(&mut output, function, "###");
        }
    }
    output
}

/// Builds one dedicated compiler-owned standard-trait API page.
fn standard_trait_page(descriptor: &StandardTraitDescriptor) -> DocumentationPage {
    let implementations = descriptor
        .implemented_by
        .iter()
        .map(|type_name| {
            let directory = if standard::enum_descriptor(type_name).is_some() {
                "enums"
            } else {
                "types"
            };
            format!(
                "- [`std::{type_name}`](../{directory}/{}.md)",
                slug(type_name)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let methods = descriptor
        .methods
        .iter()
        .map(|method| {
            format!(
                "### `{}`\n\n```exs\n{}\n```\n\n{}",
                method.name, method.signature, method.description
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let method_heading = if descriptor.methods.len() == 1 {
        "Required Method"
    } else {
        "Required Methods"
    };
    DocumentationPage {
        path: format!("modules/std/traits/{}.md", slug(descriptor.name)),
        markdown: format!(
            "# Trait `std::{}`\n\n{}\n\n## {method_heading}\n\n{methods}\n\n## Built-in Implementations\n\n{implementations}\n\n## Usage\n\n```exs\n{}\n```\n",
            descriptor.name, descriptor.description, descriptor.usage,
        ),
    }
}

/// Renders one compiler-owned standard enum page.
fn render_standard_enum(descriptor: &StandardEnumDescriptor) -> String {
    let mut output = format!(
        "# Enum `std::{}`\n\n{}\n\n```exs\nenum {} {{\n",
        descriptor.name, descriptor.description, descriptor.name
    );
    for variant in descriptor.variants {
        output.push_str(&format!("    {variant},\n"));
    }
    output.push_str("}\n```\n\n## Usage\n\n```exs\n");
    output.push_str(descriptor.usage);
    output.push_str("\n```\n");
    let traits = standard::traits()
        .iter()
        .filter(|trait_info| trait_info.implemented_by.contains(&descriptor.name));
    if traits.clone().next().is_some() {
        output.push_str("\n## Implemented Methods\n\n");
        for trait_info in traits {
            render_standard_trait_implementation(&mut output, trait_info);
        }
    }
    output.push_str("\n## Runtime Methods\n\n");
    render_clone_method(&mut output, descriptor.name);
    output
}

/// Renders the Error type page with its source-level constructor.
fn render_standard_error_type(type_info: &StandardType) -> String {
    let mut output = render_standard_type(type_info);
    if let Some(constructor) = standard_library_functions()
        .iter()
        .find(|function| function.name == "Error")
    {
        output.push_str("\n## Constructor\n\n```exs\n");
        output.push_str(constructor.signature);
        output.push_str("\n```\n\n");
        output.push_str(constructor.description);
        output.push_str("\n\n```exs\n");
        output.push_str(&script_example(constructor.example));
        output.push_str("\n```\n");
    }
    output
}

/// Renders one synthetic standard-library type page.
fn render_standard_type(type_info: &StandardType) -> String {
    let mut output = format!(
        "# Type `std::{}`\n\n{}\n\n",
        type_info.name, type_info.description
    );
    output.push_str("```exs\n");
    output.push_str(&format!("type {}\n```\n", type_info.name));
    output.push_str("\n## Usage\n\n```exs\n");
    output.push_str(type_info.usage);
    output.push_str("\n```\n");
    let traits = standard::traits()
        .iter()
        .filter(|descriptor| descriptor.implemented_by.contains(&type_info.name))
        .collect::<Vec<_>>();
    output.push_str("\n## Implemented Methods\n\n");
    for descriptor in traits {
        render_standard_trait_implementation(&mut output, descriptor);
    }
    for method in type_info.methods {
        output.push_str(&format!(
            "### `{}`\n\n{}\n\n```exs\n{}\n```\n\n",
            method.signature,
            method.description,
            script_example(method.example)
        ));
    }
    let static_functions = standard_library_type_static_functions(type_info.name);
    if !static_functions.is_empty() {
        output.push_str("## Static Functions\n\n");
        for function in static_functions {
            render_standard_function(&mut output, function, "###");
        }
    }
    render_clone_method(&mut output, type_info.name);
    output
}

/// Renders one standard static function under a caller-selected Markdown heading level.
fn render_standard_function(output: &mut String, function: &StandardFunction, heading: &str) {
    output.push_str(&format!(
        "{heading} `{}`\n\n{}\n\n```exs\n{}\n```\n\n",
        function.signature,
        function.description,
        script_example(function.example)
    ));
}

/// Renders the automatic runtime-owned deep clone method for one source-visible type.
fn render_clone_method(output: &mut String, type_name: &str) {
    output.push_str(&format!(
        "### `clone() -> {type_name} | Error`\n\nCreates a synchronous deep copy of this value's reachable mutable graph. Lists, Objects, nominal values, enum payloads, Errors, Cells, and Closures are copied while preserving aliases and cycles inside the copy; immutable values such as None, Bool, Int, Float, and String are reused. `clone()` never mutates its source, cannot be overridden, and returns `CloneError` when a reachable host-owned resource cannot be cloned.\n\n```exs\nfn main(value: {type_name}) -> {type_name} | Error {{\n    ret value.clone();\n}}\n```\n\n"
    ));
}

/// Renders one built-in trait implementation with the same method detail as nominal pages.
fn render_standard_trait_implementation(output: &mut String, descriptor: &StandardTraitDescriptor) {
    output.push_str(&format!(
        "### Trait [`{}`](../traits/{}.md)\n\n",
        descriptor.name,
        slug(descriptor.name)
    ));
    for method in descriptor.methods {
        output.push_str(&format!("#### `{}`\n\n", method.name));
        output.push_str(method.description);
        output.push_str("\n\n```exs\n");
        output.push_str(method.signature.trim_end_matches(';'));
        output.push_str(" { ... }\n```\n\n");
    }
}

/// Renders one method-body example as an independently runnable ExS script.
fn script_example(body: &str) -> String {
    let mut output = String::from("fn main() {\n");
    for line in body.lines() {
        output.push_str("    ");
        output.push_str(line);
        output.push('\n');
    }
    output.push('}');
    output
}

/// Builds one synthetic standard-library namespace page.
fn standard_namespace_page(namespace: &StandardNamespace) -> DocumentationPage {
    let mut markdown = format!(
        "# Namespace `std::{}`\n\n{}\n\n## Functions\n\n",
        namespace.name, namespace.description
    );
    for function in namespace.functions {
        render_standard_function(&mut markdown, function, "###");
    }
    DocumentationPage {
        path: format!("modules/std/namespaces/{}.md", slug(namespace.name)),
        markdown,
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
            let rendered = parameter.type_annotation.as_ref().map_or_else(
                || parameter.name.name.clone(),
                |annotation| format!("{}: {}", parameter.name.name, type_annotation(annotation)),
            );
            if parameter.variadic {
                format!("{rendered}...")
            } else {
                rendered
            }
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

/// Appends the consecutive preceding `///` documentation comment and reports whether one exists.
fn append_comment(output: &mut String, source: &str, span: SourceSpan<'_>) -> bool {
    let Some(comment) = documentation_comment(source, span) else {
        return false;
    };
    output.push_str(&comment);
    output.push_str("\n\n");
    true
}

/// Returns the consecutive preceding `///` documentation comment, if present.
fn documentation_comment(source: &str, span: SourceSpan<'_>) -> Option<String> {
    let start = usize::try_from(span.start_byte)
        .unwrap_or_default()
        .min(source.len());
    let mut comment = Vec::new();
    for line in source[..start].lines().rev() {
        let line = line.trim_start();
        if line.is_empty() && comment.is_empty() {
            continue;
        }
        let Some(line) = line.strip_prefix("///") else {
            break;
        };
        comment.push(line.strip_prefix(' ').unwrap_or(line).to_owned());
    }
    if !comment.is_empty() {
        comment.reverse();
        Some(comment.join("\n"))
    } else {
        None
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
