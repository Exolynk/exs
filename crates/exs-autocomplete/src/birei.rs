use std::future::Future;
use std::pin::Pin;

use birei::code_editor::{
    CodeCompletionItem, CodeCompletionKind, CodeLanguageService, CompletionRequest,
    CompletionResponse, DiagnosticsRequest, DiagnosticsResponse, HighlightRequest,
    HighlightResponse, HighlightSpan, IndentAction, IndentRequest, IndentResponse, TextEdit,
};
use exs_compiler::{HighlightKind, SourceInput, highlight, source_lex};

use crate::syntax::tokenize;
use crate::{CompletionEngine, CompletionKind, CompletionRequest as ExsCompletionRequest};

/// Birei language service backed by the reusable ExS completion engine.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExsBireiLanguageService {
    /// Stateless ExS completion engine shared across every editor request.
    engine: CompletionEngine,
}

impl ExsBireiLanguageService {
    /// Produces one Birei completion response for the active editor selection.
    fn completion_response(
        &self,
        source: &str,
        cursor: usize,
        selection_is_collapsed: bool,
    ) -> CompletionResponse {
        let lexed = source_lex(SourceInput {
            source_id: "<birei>",
            text: source,
        });
        if !selection_is_collapsed || lexed.is_comment_position(cursor) {
            return CompletionResponse::default();
        }
        let response = self
            .engine
            .complete(ExsCompletionRequest { source, cursor });
        CompletionResponse {
            items: response
                .items
                .into_iter()
                .map(|item| CodeCompletionItem {
                    label: item.label,
                    detail: item.detail,
                    insert_text: item.insert_text,
                    cursor: item.cursor,
                    kind: map_kind(item.kind),
                })
                .collect(),
            replace: response.replace,
        }
    }
}

impl CodeLanguageService for ExsBireiLanguageService {
    /// Returns Birei's stable language identifier for ExS source documents.
    fn language_id(&self) -> &'static str {
        "exs"
    }

    /// Returns lexical and parser-aware ExS syntax highlighting spans.
    fn highlight<'a>(
        &'a self,
        request: HighlightRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = HighlightResponse> + 'a>> {
        Box::pin(async move {
            HighlightResponse {
                spans: highlight(SourceInput {
                    source_id: "<birei>",
                    text: request.text,
                })
                .into_iter()
                .map(|span| HighlightSpan {
                    range: span.start..span.end,
                    class_name: highlight_class(span.kind),
                })
                .collect(),
            }
        })
    }

    /// Maps one Birei completion request into an ExS completion response.
    fn complete<'a>(
        &'a self,
        request: CompletionRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = CompletionResponse> + 'a>> {
        Box::pin(async move {
            self.completion_response(
                request.text,
                request.cursor.offset,
                request.selection.start == request.selection.end,
            )
        })
    }

    /// Inserts an ExS-indented line when Birei handles an Enter key press.
    fn indent<'a>(
        &'a self,
        request: IndentRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = IndentResponse> + 'a>> {
        Box::pin(async move {
            let edit = match request.action {
                IndentAction::NewLine => Some(newline_indent(request.text, request.selection)),
                IndentAction::Indent | IndentAction::Outdent => None,
            };
            IndentResponse { edit }
        })
    }

    /// Leaves compiler diagnostics separate from completion until incremental diagnostics are added.
    fn diagnostics<'a>(
        &'a self,
        _request: DiagnosticsRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = DiagnosticsResponse> + 'a>> {
        Box::pin(async { DiagnosticsResponse::default() })
    }
}

/// Maps ExS semantic highlighting categories to Birei's built-in editor token classes.
fn highlight_class(kind: HighlightKind) -> &'static str {
    match kind {
        HighlightKind::Comment => "birei-code-editor__token--comment",
        HighlightKind::String | HighlightKind::Number => "birei-code-editor__token--string",
        HighlightKind::Operator => "birei-code-editor__token--operator",
        HighlightKind::Punctuation => "birei-code-editor__token--punctuation",
        HighlightKind::Keyword | HighlightKind::Type | HighlightKind::Variant => {
            "birei-code-editor__token--tag"
        }
        HighlightKind::Function
        | HighlightKind::Method
        | HighlightKind::Field
        | HighlightKind::Binding
        | HighlightKind::Variable => "birei-code-editor__token--attribute",
    }
}

/// Converts detailed ExS categories into Birei's current display categories.
fn map_kind(kind: CompletionKind) -> CodeCompletionKind {
    match kind {
        CompletionKind::Snippet => CodeCompletionKind::Snippet,
        CompletionKind::Keyword | CompletionKind::HostMember => CodeCompletionKind::Keyword,
        CompletionKind::Function
        | CompletionKind::Variable
        | CompletionKind::Type
        | CompletionKind::Enum
        | CompletionKind::Trait
        | CompletionKind::Variant => CodeCompletionKind::Attribute,
    }
}

/// Builds the source edit used for one ExS newline action.
fn newline_indent(text: &str, selection: birei::code_editor::CodeSelection) -> TextEdit {
    let start = clamp_byte_offset(text, selection.start);
    let end = clamp_byte_offset(text, selection.end.max(start));
    let line_start = text[..start].rfind('\n').map_or(0, |offset| offset + 1);
    let line = &text[line_start..start];
    let base_indent = line
        .chars()
        .take_while(|character| matches!(character, ' ' | '\t'))
        .collect::<String>();
    let opens_block = line.trim_end().ends_with('{');
    let next_is_closing_brace = text[end..].trim_start().starts_with('}');
    let adjacent_closing_brace = text[end..].starts_with('}');
    let closing_indent = adjacent_closing_brace
        .then(|| closing_brace_indent(text, end))
        .flatten()
        .unwrap_or_else(|| base_indent.clone());
    let mut indent = base_indent.clone();
    if opens_block {
        indent.push_str("    ");
    }
    let replacement = if (opens_block && next_is_closing_brace) || adjacent_closing_brace {
        format!("\n{indent}\n{closing_indent}")
    } else {
        format!("\n{indent}")
    };
    let cursor = start + 1 + indent.len();
    TextEdit {
        range: start..end,
        replacement,
        cursor: Some(cursor),
    }
}

/// Returns the indentation of the opening brace matching the closer at `closing_offset`.
fn closing_brace_indent(text: &str, closing_offset: usize) -> Option<String> {
    let tokens = tokenize(text);
    let mut openings = Vec::new();
    for token in tokens {
        match token.text.as_str() {
            "{" => openings.push(token),
            "}" if token.start == closing_offset => {
                let opening = openings.pop()?;
                let line_start = text[..opening.start]
                    .rfind('\n')
                    .map_or(0, |offset| offset + 1);
                return Some(
                    text[line_start..opening.start]
                        .chars()
                        .take_while(|character| matches!(character, ' ' | '\t'))
                        .collect(),
                );
            }
            "}" => {
                let _opening = openings.pop();
            }
            _ => {}
        }
    }
    None
}

/// Clamps a browser-provided offset onto a safe UTF-8 boundary.
fn clamp_byte_offset(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Verifies Birei newline edits follow existing ExS indentation rules.
#[cfg(test)]
mod tests {
    use birei::code_editor::CodeSelection;

    use super::{ExsBireiLanguageService, newline_indent};

    /// Preserves the active line's existing indentation when adding a statement line.
    #[test]
    fn preserves_current_line_indentation() {
        let text = "fn main() {\n    value";
        let edit = newline_indent(
            text,
            CodeSelection {
                start: text.len(),
                end: text.len(),
            },
        );
        assert_eq!(edit.replacement, "\n    ");
        assert_eq!(edit.cursor, Some(text.len() + 5));
    }

    /// Creates an indented blank line before an adjacent closing brace.
    #[test]
    fn expands_an_empty_block() {
        let text = "{}";
        let edit = newline_indent(text, CodeSelection { start: 1, end: 1 });
        assert_eq!(edit.replacement, "\n    \n");
        assert_eq!(edit.cursor, Some(6));
    }

    /// Aligns an adjacent closing brace with its matching opening brace.
    #[test]
    fn aligns_an_adjacent_closing_brace_with_its_opening_block() {
        let text = "fn main() {\n    work();}";
        let cursor = text.find('}').unwrap_or_default();
        let edit = newline_indent(
            text,
            CodeSelection {
                start: cursor,
                end: cursor,
            },
        );
        assert_eq!(edit.replacement, "\n    \n");
        assert_eq!(edit.cursor, Some(cursor + 5));
    }

    /// Returns no Birei completion items while writing a line comment.
    #[test]
    fn suppresses_comment_completions_in_the_birei_adapter() {
        let service = ExsBireiLanguageService::default();
        for source in ["//a", "// a", "///a"] {
            let response = service.completion_response(source, source.len(), true);
            assert!(response.items.is_empty());
            assert_eq!(response.replace, None);
        }
    }
}
