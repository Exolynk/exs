//! Renders the documentation model as one compact reference for an LLM context window.

use super::model::{ApiDocumentation, CompactForm, ItemDocs, ItemKind, MemberDocs, ModuleDocs};

/// Compact source-language reference that starts the generated reference.
const LANGUAGE_SPECIFICATION: &str = include_str!("../../../../LLM_SPECIFICATION.md");

/// Renders the language specification followed by the compact API of `documentation`.
pub(crate) fn render(documentation: &ApiDocumentation) -> String {
    let mut output = String::from(LANGUAGE_SPECIFICATION.trim_end());
    output.push_str("\n\n# Compact API Reference\n\n");
    output.push_str(
        "This reference lists callable signatures, type contracts, enum variants, and source documentation. It intentionally omits examples, navigation, and repeated implementation details.\n\n",
    );
    for module in documentation
        .modules
        .iter()
        .filter(|module| !module.items.is_empty())
    {
        render_module(&mut output, module);
    }
    output
}

/// Renders the language specification, the complete `full_modules`, and an index of the other
/// modules' declarations, whose entries `render_items` returns on request.
pub(crate) fn render_core(documentation: &ApiDocumentation, full_modules: &[&str]) -> String {
    let mut output = String::from(LANGUAGE_SPECIFICATION.trim_end());
    output.push_str("\n\n# API Index\n\n");
    output.push_str(
        "The other modules with their types, enums, traits and namespaces, then their constants and functions(). Look up the entries you use by their module path, such as `http::get` or `exo::dom::Card`, by a bare name, or a whole module.\n\n",
    );
    let modules = documentation
        .modules
        .iter()
        .filter(|module| !module.items.is_empty());
    for module in modules
        .clone()
        .filter(|module| !full_modules.contains(&module.name.as_str()))
    {
        let names = module
            .items
            .iter()
            .filter(|item| !item.kind.is_inline())
            .map(|item| item.name.clone())
            .chain(
                module
                    .items
                    .iter()
                    .filter(|item| item.kind == ItemKind::Constant)
                    .map(|item| item.name.clone()),
            )
            .chain(
                module
                    .items
                    .iter()
                    .filter(|item| item.kind == ItemKind::Function)
                    .map(|item| format!("{}()", item.name)),
            )
            .collect::<Vec<_>>()
            .join(", ");
        output.push_str(&format!("- `{}`: {names}\n", module.name));
    }
    output.push_str("\n# Compact API Reference\n\n");
    for module in modules.filter(|module| full_modules.contains(&module.name.as_str())) {
        render_module(&mut output, module);
    }
    output
}

/// Renders the requested entries: a whole module, `module::name`, or a bare name in any module.
/// Returns the rendered reference and the names that matched nothing.
pub(crate) fn render_items(
    documentation: &ApiDocumentation,
    names: &[&str],
) -> (String, Vec<String>) {
    let mut output = String::new();
    let mut missing = Vec::new();
    for name in names {
        let name = name.trim();
        if let Some(module) = documentation.module(name) {
            render_module(&mut output, module);
            continue;
        }
        let (module_name, item_name) = match name.rsplit_once("::") {
            Some((module, item)) => (Some(module), item),
            None => (None, name),
        };
        let mut found = false;
        for module in documentation
            .modules
            .iter()
            .filter(|module| module_name.is_none_or(|wanted| module.name == wanted))
        {
            for item in module.items.iter().filter(|item| item.name == item_name) {
                output.push_str(&format!("## `{}::{}`\n\n", module.name, item.name));
                if item.kind.is_inline() {
                    render_inline(&mut output, item);
                } else {
                    render_item(&mut output, item);
                }
                found = true;
            }
        }
        if !found {
            missing.push(name.to_owned());
        }
    }
    (output, missing)
}

/// Appends one module with its description, declarations, constants and functions.
fn render_module(output: &mut String, module: &ModuleDocs) {
    output.push_str(&format!("## Module `{}`\n\n", module.name));
    if !module.description.is_empty() {
        output.push_str(&module.description);
        output.push_str("\n\n");
    }
    for item in module.items.iter().filter(|item| !item.kind.is_inline()) {
        render_item(output, item);
    }
    for (kind, heading, suffix) in [
        (ItemKind::Constant, "Constants", ""),
        (ItemKind::Function, "Functions", " { ... }"),
    ] {
        let items = module
            .items
            .iter()
            .filter(|item| item.kind == kind)
            .collect::<Vec<_>>();
        if items.is_empty() {
            continue;
        }
        output.push_str(&format!("### {heading}\n\n```exs\n"));
        for (index, item) in items.iter().enumerate() {
            if index > 0 && kind == ItemKind::Function {
                output.push('\n');
            }
            push_comment(output, &item.description, "");
            output.push_str(&item.declaration);
            output.push_str(suffix);
            output.push('\n');
        }
        output.push_str("```\n\n");
    }
}

/// Appends one function or constant as a documented declaration.
fn render_inline(output: &mut String, item: &ItemDocs) {
    output.push_str("```exs\n");
    push_comment(output, &item.description, "");
    output.push_str(&item.declaration);
    if item.kind == ItemKind::Function {
        output.push_str(" { ... }");
    }
    output.push_str("\n```\n\n");
}

/// Appends one type, enum, trait, or namespace.
fn render_item(output: &mut String, item: &ItemDocs) {
    output.push_str(&format!("### {} `{}`\n\n", item.kind.label(), item.name));
    if !item.description.is_empty() {
        output.push_str(&item.description);
        output.push_str("\n\n");
    }
    let mut code = item.declaration.clone();
    let mut lists = String::new();
    for section in &item.sections {
        match &section.compact {
            CompactForm::Block(header) => {
                if !code.is_empty() {
                    code.push_str("\n\n");
                }
                let indentation = if header.is_empty() {
                    ""
                } else {
                    code.push_str(header);
                    code.push_str(" {\n");
                    "    "
                };
                for member in &section.members {
                    push_member(&mut code, member, indentation);
                }
                if !header.is_empty() {
                    code.push('}');
                }
            }
            CompactForm::List => {
                lists.push_str(&format!("{}:\n\n", section.title));
                for member in &section.members {
                    lists.push_str(&format!("- `{}`\n", member.declaration));
                }
                lists.push('\n');
            }
            CompactForm::Omit => {}
        }
    }
    if !code.is_empty() {
        output.push_str(&format!("```exs\n{}\n```\n\n", code.trim_end()));
    }
    output.push_str(&lists);
}

/// Appends one member declaration with its documentation comment.
fn push_member(code: &mut String, member: &MemberDocs, indentation: &str) {
    push_comment(code, &member.description, indentation);
    code.push_str(indentation);
    code.push_str(&member.declaration);
    code.push_str(" { ... }\n");
}

/// Appends `documentation` as `///` comment lines at `indentation`.
fn push_comment(output: &mut String, documentation: &str, indentation: &str) {
    for line in documentation.lines() {
        output.push_str(indentation);
        output.push_str("///");
        if !line.is_empty() {
            output.push(' ');
            output.push_str(line);
        }
        output.push('\n');
    }
}
