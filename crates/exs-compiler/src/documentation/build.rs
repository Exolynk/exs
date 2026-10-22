//! Builds the documentation model from the standard-library descriptors and parsed modules.

use std::collections::HashSet;

use super::model::{
    ApiDocumentation, CompactForm, ItemDocs, ItemKind, ItemRef, MemberDocs, ModuleDocs,
    ModuleOrigin, SectionDocs,
};
use super::shared::{documentation_comment, function_signature, module_comment, slug};
use super::standard::{
    StandardFunction, StandardMethod, StandardType, automatic_trait_methods,
    automatic_traits_for_standard_type, standard_library_functions, standard_library_namespaces,
    standard_library_type_static_functions, standard_library_types,
};
use crate::ast::{
    EnumDeclaration, FunctionDeclaration, ImplDeclaration, Module, TraitDeclaration, TypeField,
    Visibility,
};
use crate::codegen::standard::{self as codegen_standard, StandardTraitDescriptor};
use crate::loaded_project::{LoadedProject, parse_source};
use crate::{CompileLimits, ModuleResolver, SourceInput};

/// Name of the standard-library module.
const STD: &str = "std";

/// Group of the sections that list implemented methods.
const IMPLEMENTED: &str = "Implemented Methods";

/// Group of the sections that list methods traits provide automatically.
const AUTOMATIC: &str = "Automatically Provided Methods";

/// Overview of the standard-library module.
const STD_DESCRIPTION: &str = "Built-in standard items are globally available in ExS source and may also be written with the `std::` qualifier; importing `std` is neither required nor allowed. Every source-visible value provides `clone() -> T | Error`, where `T` is the receiver type.";

/// Documents the standard library, `source`, its imports, and the libraries they name.
pub(crate) fn project<R: ModuleResolver>(
    source: SourceInput<'_>,
    resolver: &mut R,
) -> Result<ApiDocumentation, String> {
    let project = LoadedProject::load(source, resolver, CompileLimits::default())?;
    project.validate_no_cycles()?;
    build(&project)
}

/// Documents the standard library and every library module `resolver` lists.
pub(crate) fn libraries<R: ModuleResolver>(resolver: &mut R) -> Result<ApiDocumentation, String> {
    let paths = resolver.libraries();
    let project = LoadedProject::load_libraries(&paths, resolver, CompileLimits::default())?;
    build(&project)
}

/// Builds the documentation of the standard library and every loaded module.
fn build(project: &LoadedProject) -> Result<ApiDocumentation, String> {
    let mut modules = standard()?;
    let parsed = project.parse_modules()?;
    let mut libraries = Vec::new();
    let mut files = Vec::new();
    for (module, file) in parsed.iter().zip(project.files()) {
        match &file.library {
            Some(path) => libraries.push((path.as_str(), module, file.text.as_str())),
            None => files.push((file.display_path.as_str(), module, file.text.as_str())),
        }
    }
    libraries.sort_by(|left, right| left.0.cmp(right.0));
    let std_traits = std_trait_names()?;
    for (path, module, text) in libraries {
        modules.push(ModuleDocs {
            name: path.to_owned(),
            segments: path.split("::").map(slug).collect(),
            origin: ModuleOrigin::Library,
            description: module_comment(text),
            items: source_items(module, text, path, &std_traits),
        });
    }
    for (path, module, text) in files {
        modules.push(ModuleDocs {
            name: path.to_owned(),
            segments: file_segments(path),
            origin: ModuleOrigin::Project,
            description: module_comment(text),
            items: source_items(module, text, path, &std_traits),
        });
    }
    make_unique(&mut modules);
    Ok(ApiDocumentation { modules })
}

/// Returns the path segments of a project file, such as `["lib", "util"]` for
/// `./lib/util.exs`.
fn file_segments(path: &str) -> Vec<String> {
    let path = path.strip_suffix(".exs").unwrap_or(path);
    path.split(['/', '\\'])
        .filter(|segment| !matches!(*segment, "" | "." | ".."))
        .map(slug)
        .collect()
}

/// Renames modules whose segments repeat an earlier module's or are empty, so every module
/// has its own place in generated output.
fn make_unique(modules: &mut [ModuleDocs]) {
    let mut seen = HashSet::new();
    for module in modules {
        if module.segments.is_empty() {
            module.segments.push(String::from("module"));
        }
        let base = module.segments.last().cloned().unwrap_or_default();
        let mut counter = 1;
        while !seen.insert(module.segments.join("/")) {
            counter += 1;
            if let Some(last) = module.segments.last_mut() {
                *last = format!("{base}-{counter}");
            }
        }
    }
}

/// Returns the prelude modules with their source text.
fn prelude() -> Result<Vec<(Module<'static>, &'static str)>, String> {
    crate::prelude::source_inputs()
        .into_iter()
        .map(|source| {
            parse_source(
                source.source_id,
                source.text,
                CompileLimits::default().max_nesting,
            )
            .map(|module| (module, source.text))
        })
        .collect()
}

/// Returns the names of every standard trait, built in or declared by the prelude.
fn std_trait_names() -> Result<Vec<String>, String> {
    let mut names = codegen_standard::traits()
        .iter()
        .map(|descriptor| descriptor.name.to_owned())
        .collect::<Vec<_>>();
    for (module, _) in prelude()? {
        names.extend(
            module
                .traits
                .iter()
                .map(|declaration| declaration.name.name.clone()),
        );
    }
    Ok(names)
}

/// Documents the standard library: `std` and its `test` submodule.
fn standard() -> Result<Vec<ModuleDocs>, String> {
    let prelude = prelude()?;
    let std_traits = std_trait_names()?;
    let declared = |name: &str| {
        prelude.iter().any(|(module, _)| {
            module
                .types
                .iter()
                .any(|declaration| declaration.name.name == name)
                || module
                    .enums
                    .iter()
                    .any(|declaration| declaration.name.name == name)
                || module
                    .traits
                    .iter()
                    .any(|declaration| declaration.name.name == name)
        })
    };
    let mut prelude_items = Vec::new();
    for (module, text) in &prelude {
        prelude_items.extend(source_items(module, text, STD, &std_traits));
    }
    let mut items = standard_library_types()
        .iter()
        .filter(|type_info| !declared(type_info.name))
        .map(standard_type)
        .collect::<Vec<_>>();
    items.extend(
        prelude_items
            .iter()
            .filter(|item| item.kind == ItemKind::Type)
            .cloned(),
    );
    items.extend(
        codegen_standard::enums()
            .iter()
            .filter(|descriptor| !declared(descriptor.name))
            .map(|descriptor| {
                standard_nominal(
                    ItemKind::Enum,
                    descriptor.name,
                    descriptor.description,
                    enum_declaration(descriptor.name, descriptor.variants),
                    descriptor.usage,
                )
            }),
    );
    items.extend(
        prelude_items
            .iter()
            .filter(|item| item.kind == ItemKind::Enum)
            .cloned(),
    );
    items.extend(
        codegen_standard::traits()
            .iter()
            .filter(|descriptor| !declared(descriptor.name))
            .map(standard_trait),
    );
    items.extend(
        prelude_items
            .iter()
            .filter(|item| item.kind == ItemKind::Trait)
            .cloned(),
    );
    let mut test = Vec::new();
    for namespace in standard_library_namespaces() {
        if namespace.name == "test" {
            test.extend(namespace.functions.iter().map(standard_function));
            continue;
        }
        items.push(ItemDocs {
            kind: ItemKind::Namespace,
            name: namespace.name.to_owned(),
            description: namespace.description.to_owned(),
            declaration: String::new(),
            usage: None,
            sections: vec![SectionDocs {
                compact: CompactForm::Block(String::new()),
                members: namespace
                    .functions
                    .iter()
                    .map(standard_function_member)
                    .collect(),
                ..section("Functions")
            }],
        });
    }
    items.extend(standard_library_functions().iter().map(standard_function));
    items.extend(
        prelude_items
            .into_iter()
            .filter(|item| item.kind == ItemKind::Function),
    );
    Ok(vec![
        ModuleDocs {
            name: STD.to_owned(),
            segments: vec![STD.to_owned()],
            origin: ModuleOrigin::Standard,
            description: STD_DESCRIPTION.to_owned(),
            items,
        },
        ModuleDocs {
            name: "std::test".to_owned(),
            segments: vec![STD.to_owned(), "test".to_owned()],
            origin: ModuleOrigin::Standard,
            description: "Test assertions, available directly in every module and through the `std::test` path.".to_owned(),
            items: test,
        },
    ])
}

/// Returns an empty section titled `title` that the compact reference leaves out.
fn section(title: &str) -> SectionDocs {
    SectionDocs {
        title: title.to_owned(),
        group: None,
        link: None,
        description: String::new(),
        compact: CompactForm::Omit,
        members: Vec::new(),
        references: Vec::new(),
    }
}

/// Returns a reference to the standard item `name`.
fn std_ref(name: &str) -> ItemRef {
    ItemRef {
        module: STD.to_owned(),
        name: name.to_owned(),
    }
}

/// Documents one runtime-owned standard type.
fn standard_type(type_info: &StandardType) -> ItemDocs {
    let mut item = standard_nominal(
        ItemKind::Type,
        type_info.name,
        type_info.description,
        format!("type {}", type_info.name),
        type_info.usage,
    );
    let clone = item.sections.pop();
    if !type_info.methods.is_empty() {
        item.sections.push(SectionDocs {
            group: Some(IMPLEMENTED.to_owned()),
            compact: CompactForm::Block(format!("impl {}", type_info.name)),
            members: type_info.methods.iter().map(standard_method).collect(),
            ..section("Methods")
        });
    }
    item.sections.extend(automatic_sections(
        automatic_traits_for_standard_type(type_info.name),
        "this type",
    ));
    let statics = standard_library_type_static_functions(type_info.name);
    if !statics.is_empty() {
        item.sections.push(SectionDocs {
            compact: CompactForm::Block(format!("impl {}", type_info.name)),
            members: statics.iter().map(standard_function_member).collect(),
            ..section("Static Functions")
        });
    }
    item.sections.extend(clone);
    item
}

/// Documents one runtime-owned standard type or enum with its built-in trait implementations
/// and its `clone` method.
fn standard_nominal(
    kind: ItemKind,
    name: &str,
    description: &str,
    declaration: String,
    usage: &str,
) -> ItemDocs {
    let mut sections = codegen_standard::traits()
        .iter()
        .filter(|descriptor| descriptor.implemented_by.contains(&name))
        .map(|descriptor| SectionDocs {
            group: Some(IMPLEMENTED.to_owned()),
            link: Some(std_ref(descriptor.name)),
            members: trait_members(descriptor),
            ..section(&format!("Trait `{}`", descriptor.name))
        })
        .collect::<Vec<_>>();
    sections.push(clone_section(name));
    ItemDocs {
        kind,
        name: name.to_owned(),
        description: description.to_owned(),
        declaration,
        usage: Some(usage.to_owned()),
        sections,
    }
}

/// Returns the members of a built-in trait.
fn trait_members(descriptor: &StandardTraitDescriptor) -> Vec<MemberDocs> {
    descriptor
        .methods
        .iter()
        .map(|method| MemberDocs {
            name: method.name.to_owned(),
            description: method.description.to_owned(),
            declaration: method.signature.trim_end_matches(';').to_owned(),
            example: None,
        })
        .collect()
}

/// Documents one built-in trait.
fn standard_trait(descriptor: &StandardTraitDescriptor) -> ItemDocs {
    let mut declaration = format!("trait {} {{\n", descriptor.name);
    for method in descriptor.methods {
        declaration.push_str(&format!("    {}\n", method.signature));
    }
    declaration.push('}');
    let mut sections = vec![SectionDocs {
        members: trait_members(descriptor),
        ..section("Required Methods")
    }];
    sections.extend(automatic_trait_section(descriptor.name));
    sections.push(SectionDocs {
        references: descriptor
            .implemented_by
            .iter()
            .map(|name| std_ref(name))
            .collect(),
        ..section("Built-in Implementations")
    });
    ItemDocs {
        kind: ItemKind::Trait,
        name: descriptor.name.to_owned(),
        description: descriptor.description.to_owned(),
        declaration,
        usage: Some(descriptor.usage.to_owned()),
        sections,
    }
}

/// Returns the section of the methods the trait `name` provides automatically, if any.
fn automatic_trait_section(name: &str) -> Option<SectionDocs> {
    let methods = automatic_trait_methods(name);
    (!methods.is_empty()).then(|| SectionDocs {
        description: "The standard library provides these eager methods for every iterable value, including user-defined types that satisfy this trait. They do not need to be implemented by user-defined types.".to_owned(),
        compact: CompactForm::List,
        members: methods.iter().map(standard_method).collect(),
        ..section(AUTOMATIC)
    })
}

/// Returns one section per trait in `traits` listing the methods it provides automatically
/// to `receiver`.
fn automatic_sections(traits: &[&str], receiver: &str) -> Vec<SectionDocs> {
    traits
        .iter()
        .filter(|name| !automatic_trait_methods(name).is_empty())
        .map(|name| SectionDocs {
            group: Some(AUTOMATIC.to_owned()),
            link: Some(std_ref(name)),
            description: format!(
                "These methods are supplied by the trait and do not need to be implemented by {receiver}."
            ),
            members: automatic_trait_methods(name)
                .iter()
                .map(standard_method)
                .collect(),
            ..section(&format!("Trait `{name}`"))
        })
        .collect()
}

/// Returns the section documenting the runtime-owned `clone` method of `type_name`.
fn clone_section(type_name: &str) -> SectionDocs {
    SectionDocs {
        members: vec![MemberDocs {
            name: "clone".to_owned(),
            description: "Creates a deep copy of the value's reachable mutable graph, preserving aliases and cycles inside the copy. Returns `CloneError` when a reachable host-owned resource cannot be cloned.".to_owned(),
            declaration: format!("fn clone(self) -> {type_name} | Error"),
            example: None,
        }],
        ..section("Runtime Methods")
    }
}

/// Documents one standard instance method.
fn standard_method(method: &StandardMethod) -> MemberDocs {
    MemberDocs {
        name: call_name(method.signature).to_owned(),
        description: method.description.to_owned(),
        declaration: format!("fn {}", method.signature),
        example: Some(script_example(method.example)),
    }
}

/// Documents one standard static or namespace function as a member.
fn standard_function_member(function: &StandardFunction) -> MemberDocs {
    MemberDocs {
        name: function.name.to_owned(),
        description: function.description.to_owned(),
        declaration: format!("fn {}", function.signature),
        example: Some(script_example(function.example)),
    }
}

/// Documents one standard function as a module item; its declaration names it without the
/// path that qualifies it.
fn standard_function(function: &StandardFunction) -> ItemDocs {
    let (path, rest) = function
        .signature
        .split_once('(')
        .unwrap_or((function.signature, ""));
    let name = path.rsplit("::").next().unwrap_or(path);
    ItemDocs {
        kind: ItemKind::Function,
        name: function.name.to_owned(),
        description: function.description.to_owned(),
        declaration: format!("fn {name}({rest}"),
        usage: Some(script_example(function.example)),
        sections: Vec::new(),
    }
}

/// Returns the name a signature such as `len() -> Int` calls.
fn call_name(signature: &str) -> &str {
    signature
        .split_once('(')
        .map_or(signature, |(name, _)| name)
        .trim()
}

/// Wraps a method-body example into an independently runnable ExS script.
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

/// Renders an enum declaration with the given variant spellings.
fn enum_declaration(name: &str, variants: &[&str]) -> String {
    let mut declaration = format!("enum {name} {{\n");
    for variant in variants {
        declaration.push_str(&format!("    {variant},\n"));
    }
    declaration.push('}');
    declaration
}

/// Documents the public declarations of one parsed module named `module_name`.
fn source_items(
    module: &Module<'_>,
    text: &str,
    module_name: &str,
    std_traits: &[String],
) -> Vec<ItemDocs> {
    let context = SourceContext {
        module,
        text,
        module_name,
        std_traits,
    };
    let mut items = Vec::new();
    for declaration in module
        .types
        .iter()
        .filter(|declaration| declaration.visibility == Visibility::Public)
    {
        let mut source = format!("type {} {{\n", declaration.name.name);
        for field in &declaration.fields {
            source.push_str(&format!("    {},\n", field_declaration(field)));
        }
        source.push('}');
        items.push(ItemDocs {
            kind: ItemKind::Type,
            name: declaration.name.name.clone(),
            description: context.comment(declaration.span),
            declaration: source,
            usage: None,
            sections: context.nominal_sections(&declaration.name.name, Vec::new()),
        });
    }
    for declaration in module
        .enums
        .iter()
        .filter(|declaration| declaration.visibility == Visibility::Public)
    {
        items.push(context.enumeration(declaration));
    }
    for declaration in module
        .traits
        .iter()
        .filter(|declaration| declaration.visibility == Visibility::Public)
    {
        items.push(context.trait_item(declaration));
    }
    for declaration in module
        .constants
        .iter()
        .filter(|declaration| declaration.visibility == Visibility::Public)
    {
        items.push(ItemDocs {
            kind: ItemKind::Constant,
            name: declaration.name.name.clone(),
            description: context.comment(declaration.span),
            declaration: format!(
                "const {} = {};",
                declaration.name.name,
                crate::formatter::render_expression(&declaration.value)
            ),
            usage: None,
            sections: Vec::new(),
        });
    }
    for declaration in module
        .functions
        .iter()
        .filter(|declaration| declaration.visibility == Visibility::Public)
    {
        items.push(ItemDocs {
            kind: ItemKind::Function,
            name: declaration.name.name.clone(),
            description: context.comment(declaration.span),
            declaration: signature(declaration),
            usage: None,
            sections: Vec::new(),
        });
    }
    items
}

/// Renders one field or variant payload field, such as `x: Int`.
fn field_declaration(field: &TypeField<'_>) -> String {
    field.type_annotation.as_ref().map_or_else(
        || field.name.name.clone(),
        |annotation| {
            format!(
                "{}: {}",
                field.name.name,
                super::shared::type_annotation(annotation)
            )
        },
    )
}

/// Renders the signature of a function declaration.
fn signature(declaration: &FunctionDeclaration<'_>) -> String {
    function_signature(
        &declaration.name.name,
        &declaration.parameters,
        declaration.return_type.as_ref(),
    )
}

/// One parsed module being documented.
struct SourceContext<'m, 'a> {
    /// The parsed module.
    module: &'m Module<'a>,
    /// Its source text.
    text: &'m str,
    /// Its documented name.
    module_name: &'m str,
    /// Names of every standard trait.
    std_traits: &'m [String],
}

impl SourceContext<'_, '_> {
    /// Returns the `///` comment before `span`, or an empty string.
    fn comment(&self, span: crate::SourceSpan<'_>) -> String {
        documentation_comment(self.text, span).unwrap_or_default()
    }

    /// Documents one enum with its variants.
    fn enumeration(&self, declaration: &EnumDeclaration<'_>) -> ItemDocs {
        let mut source = format!("enum {} {{\n", declaration.name.name);
        let mut variants = Vec::new();
        for variant in &declaration.variants {
            let mut spelling = variant.name.name.clone();
            if !variant.fields.is_empty() {
                let fields = variant
                    .fields
                    .iter()
                    .map(field_declaration)
                    .collect::<Vec<_>>()
                    .join(", ");
                spelling = format!("{spelling}({fields})");
            }
            source.push_str(&format!("    {spelling},\n"));
            variants.push(MemberDocs {
                name: variant.name.name.clone(),
                description: self.comment(variant.span),
                declaration: spelling,
                example: None,
            });
        }
        source.push('}');
        let variants = SectionDocs {
            members: variants,
            ..section("Variants")
        };
        ItemDocs {
            kind: ItemKind::Enum,
            name: declaration.name.name.clone(),
            description: self.comment(declaration.span),
            declaration: source,
            usage: None,
            sections: self.nominal_sections(&declaration.name.name, vec![variants]),
        }
    }

    /// Documents one trait with its required methods.
    fn trait_item(&self, declaration: &TraitDeclaration<'_>) -> ItemDocs {
        let mut source = format!("trait {} {{\n", declaration.name.name);
        let mut members = Vec::new();
        for method in &declaration.methods {
            let signature = function_signature(
                &method.name.name,
                &method.parameters,
                method.return_type.as_ref(),
            );
            let body = if method.body.is_some() {
                " { ... }"
            } else {
                ";"
            };
            source.push_str(&format!("    {signature}{body}\n"));
            members.push(MemberDocs {
                name: method.name.name.clone(),
                description: self.comment(method.span),
                declaration: signature,
                example: None,
            });
        }
        source.push('}');
        let mut sections = Vec::new();
        if !members.is_empty() {
            sections.push(SectionDocs {
                members,
                ..section("Required Methods")
            });
        }
        sections.extend(automatic_trait_section(&declaration.name.name));
        ItemDocs {
            kind: ItemKind::Trait,
            name: declaration.name.name.clone(),
            description: self.comment(declaration.span),
            declaration: source,
            usage: None,
            sections,
        }
    }

    /// Returns `sections` followed by the implemented, automatically provided, and runtime
    /// methods of the type or enum `name`.
    fn nominal_sections(&self, name: &str, mut sections: Vec<SectionDocs>) -> Vec<SectionDocs> {
        let implementations = self
            .module
            .implementations
            .iter()
            .filter(|implementation| implementation.type_name.name == name)
            .collect::<Vec<_>>();
        let mut automatic = Vec::new();
        for implementation in &implementations {
            if let Some(section) = self.implementation(implementation) {
                sections.push(section);
            }
            if let Some(trait_name) = &implementation.trait_name
                && !automatic.contains(&trait_name.name.as_str())
            {
                automatic.push(trait_name.name.as_str());
            }
        }
        sections.extend(automatic_sections(&automatic, "this type"));
        sections.push(clone_section(name));
        sections
    }

    /// Documents the public methods of one implementation block, if it has any.
    fn implementation(&self, implementation: &ImplDeclaration<'_>) -> Option<SectionDocs> {
        let type_name = &implementation.type_name.name;
        let members = implementation
            .methods
            .iter()
            .filter(|method| method.visibility == Visibility::Public)
            .map(|method| {
                let mut description = self.comment(method.span);
                if description.is_empty()
                    && let Some(trait_name) = &implementation.trait_name
                {
                    description = self
                        .trait_method_comment(&trait_name.name, &method.name.name)
                        .unwrap_or_default();
                }
                MemberDocs {
                    name: method.name.name.clone(),
                    description,
                    declaration: signature(method),
                    example: None,
                }
            })
            .collect::<Vec<_>>();
        if members.is_empty() {
            return None;
        }
        let (title, header, link) = match &implementation.trait_name {
            Some(trait_name) => (
                format!("Trait `{}`", trait_name.name),
                format!("impl {} for {type_name}", trait_name.name),
                self.trait_ref(&trait_name.name),
            ),
            None => (
                "Inherent methods".to_owned(),
                format!("impl {type_name}"),
                None,
            ),
        };
        Some(SectionDocs {
            group: Some(IMPLEMENTED.to_owned()),
            link,
            compact: CompactForm::Block(header),
            members,
            ..section(&title)
        })
    }

    /// Returns a reference to the trait `name` as this module spells it.
    fn trait_ref(&self, name: &str) -> Option<ItemRef> {
        if let Some((module, item)) = name.rsplit_once("::") {
            return Some(ItemRef {
                module: module.to_owned(),
                name: item.to_owned(),
            });
        }
        if self
            .module
            .traits
            .iter()
            .any(|declaration| declaration.name.name == name)
        {
            return Some(ItemRef {
                module: self.module_name.to_owned(),
                name: name.to_owned(),
            });
        }
        self.std_traits
            .iter()
            .any(|known| known == name)
            .then(|| std_ref(name))
    }

    /// Returns the documentation of the method `method` of the trait `trait_name`, which
    /// an implementation method without its own comment inherits.
    fn trait_method_comment(&self, trait_name: &str, method: &str) -> Option<String> {
        if let Some(descriptor) = codegen_standard::trait_descriptor(trait_name) {
            return descriptor
                .methods
                .iter()
                .find(|candidate| candidate.name == method)
                .map(|candidate| candidate.description.to_owned());
        }
        self.module
            .traits
            .iter()
            .find(|declaration| declaration.name.name == trait_name)
            .and_then(|declaration| {
                declaration
                    .methods
                    .iter()
                    .find(|candidate| candidate.name.name == method)
            })
            .and_then(|candidate| documentation_comment(self.text, candidate.span))
    }
}
