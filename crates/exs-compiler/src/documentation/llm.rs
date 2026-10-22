//! Renders the documentation model as one compact reference for an LLM context window.

use super::model::{ApiDocumentation, CompactForm, ItemDocs, ItemKind, MemberDocs};

/// Compact source-language reference that starts the generated reference.
const LANGUAGE_SPECIFICATION: &str = include_str!("../../../../LLM_SPECIFICATION.md");

/// Renders the language specification followed by the compact API of `documentation`.
pub(crate) fn render(documentation: &ApiDocumentation) -> String {
    let mut output = String::from(LANGUAGE_SPECIFICATION.trim_end());
    output.push_str("\n\n# Compact API Reference\n\n");
    output.push_str(
        "This reference lists callable signatures, type contracts, enum variants, and source documentation. It intentionally omits examples, navigation, and repeated implementation details.\n\n",
    );
    for module in &documentation.modules {
        if module.items.is_empty() {
            continue;
        }
        output.push_str(&format!("## Module `{}`\n\n", module.name));
        if !module.description.is_empty() {
            output.push_str(&module.description);
            output.push_str("\n\n");
        }
        for item in module.items.iter().filter(|item| !item.kind.is_inline()) {
            render_item(&mut output, item);
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
                push_comment(&mut output, &item.description, "");
                output.push_str(&item.declaration);
                output.push_str(suffix);
                output.push('\n');
            }
            output.push_str("```\n\n");
        }
    }
    output
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
