use std::ops::Range;

/// One request for completions at a byte offset in UTF-8 ExS source text.
#[derive(Clone, Copy, Debug)]
pub struct CompletionRequest<'a> {
    /// Complete document contents at the time of the request.
    pub source: &'a str,
    /// Byte offset of the caret within `source`.
    pub cursor: usize,
}

/// Broad semantic category for an ExS completion item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionKind {
    /// A reserved ExS language word.
    Keyword,
    /// A multi-character source template with a preferred caret position.
    Snippet,
    /// A user-declared top-level function.
    Function,
    /// A visible lexical binding.
    Variable,
    /// A built-in or user-defined source type.
    Type,
    /// A built-in or user-defined enum.
    Enum,
    /// A built-in or user-defined trait.
    Trait,
    /// A member selected from an enum through `::`.
    Variant,
    /// A reserved member of the `host` boundary.
    HostMember,
}

/// One source replacement offered to the user.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionItem {
    /// Text displayed in the completion list.
    pub label: String,
    /// Optional concise signature or category shown beside `label`.
    pub detail: Option<String>,
    /// Text that replaces the response's selected source range.
    pub insert_text: String,
    /// Optional caret byte offset relative to the inserted text.
    pub cursor: Option<usize>,
    /// Semantic category of this item.
    pub kind: CompletionKind,
}

/// Completion items together with the source range each item replaces.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CompletionResponse {
    /// Candidate items in deterministic relevance order.
    pub items: Vec<CompletionItem>,
    /// Identifier prefix replaced when accepting an item.
    pub replace: Option<Range<usize>>,
}
