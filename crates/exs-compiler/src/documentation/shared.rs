use super::*;

/// Renders one declaration signature without its executable body.
pub(super) fn function_signature(
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
pub(super) fn type_annotation(annotation: &TypeAnnotation<'_>) -> String {
    annotation
        .members
        .iter()
        .map(type_name)
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Renders one recursive source type name.
pub(super) fn type_name(member: &crate::ast::TypeName<'_>) -> String {
    member.argument.as_deref().map_or_else(
        || member.name.clone(),
        |argument| format!("{}<{}>", member.name, type_annotation(argument)),
    )
}

/// Appends the consecutive preceding `///` documentation comment and reports whether one exists.
pub(super) fn append_comment(output: &mut String, source: &str, span: SourceSpan<'_>) -> bool {
    let Some(comment) = documentation_comment(source, span) else {
        return false;
    };
    output.push_str(&comment);
    output.push_str("\n\n");
    true
}

/// Returns the consecutive preceding `///` documentation comment, if present.
pub(super) fn documentation_comment(source: &str, span: SourceSpan<'_>) -> Option<String> {
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
pub(super) fn module_directory(index: usize, source_id: &str) -> String {
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
pub(super) fn slug(value: &str) -> String {
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
