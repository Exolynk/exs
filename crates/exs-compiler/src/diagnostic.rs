//! Source locations and compiler diagnostics.

use std::fmt;

/// A byte span in a named source input.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SourceSpan<'a> {
    /// The source identity supplied to the compiler.
    pub source_id: &'a str,
    /// The inclusive byte offset of the span.
    pub start_byte: u32,
    /// The exclusive byte offset of the span.
    pub end_byte: u32,
}

impl<'a> SourceSpan<'a> {
    /// Creates an empty span at the start of a source input.
    #[must_use]
    pub const fn empty(source_id: &'a str) -> Self {
        Self {
            source_id,
            start_byte: 0,
            end_byte: 0,
        }
    }

    /// Combines this span with a later span in the same source input.
    #[must_use]
    pub const fn through(self, end: Self) -> Self {
        Self {
            source_id: self.source_id,
            start_byte: self.start_byte,
            end_byte: end.end_byte,
        }
    }
}

/// The compiler phase that produced one diagnostic.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CompileDiagnosticCategory {
    /// Tokenization of the source text failed.
    Lexical,
    /// The source text does not satisfy the ExS grammar.
    Syntax,
    /// Valid syntax violates an ExS static rule.
    Semantic,
    /// An internal compiler or linking invariant failed.
    Internal,
}

impl fmt::Display for CompileDiagnosticCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Lexical => "lexical",
            Self::Syntax => "syntax",
            Self::Semantic => "semantic",
            Self::Internal => "internal",
        };
        formatter.write_str(name)
    }
}

/// One supplementary source location associated with a compiler diagnostic.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RelatedSpan<'a> {
    /// Supplementary source location.
    pub span: SourceSpan<'a>,
    /// Relationship of this location to the primary diagnostic.
    pub message: String,
}

impl<'a> RelatedSpan<'a> {
    /// Creates one supplementary diagnostic location.
    #[must_use]
    pub fn new(span: SourceSpan<'a>, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}

/// One stable source-spanned compiler error.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CompileDiagnostic<'a> {
    /// Stable machine-readable diagnostic code.
    pub code: &'static str,
    /// Compiler phase that produced the diagnostic.
    pub category: CompileDiagnosticCategory,
    /// Primary source span.
    pub span: SourceSpan<'a>,
    /// Concise diagnostic message.
    pub message: String,
    /// Supplementary source locations, such as the original duplicate declaration.
    pub related: Vec<RelatedSpan<'a>>,
}

impl<'a> CompileDiagnostic<'a> {
    /// Creates a compiler diagnostic.
    #[must_use]
    pub fn new(code: &'static str, span: SourceSpan<'a>, message: impl Into<String>) -> Self {
        Self {
            code,
            category: category_for(code),
            span,
            message: message.into(),
            related: Vec::new(),
        }
    }

    /// Attaches one supplementary source location to this diagnostic.
    #[must_use]
    pub fn with_related(mut self, span: SourceSpan<'a>, message: impl Into<String>) -> Self {
        self.related.push(RelatedSpan::new(span, message));
        self
    }
}

impl fmt::Display for CompileDiagnostic<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}-{}: {}: {}",
            self.span.source_id, self.span.start_byte, self.span.end_byte, self.code, self.message
        )
    }
}

/// Diagnostics emitted by one failed compilation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CompileDiagnostics<'a> {
    /// Collected diagnostics in source order.
    pub diagnostics: Vec<CompileDiagnostic<'a>>,
}

impl<'a> From<CompileDiagnostic<'a>> for CompileDiagnostics<'a> {
    fn from(diagnostic: CompileDiagnostic<'a>) -> Self {
        Self {
            diagnostics: vec![diagnostic],
        }
    }
}

impl<'a> CompileDiagnostics<'a> {
    /// Creates an empty diagnostic collection.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            diagnostics: Vec::new(),
        }
    }

    /// Appends one compiler diagnostic.
    pub fn push(&mut self, diagnostic: CompileDiagnostic<'a>) {
        self.diagnostics.push(diagnostic);
    }

    /// Appends all diagnostics from another compilation stage.
    pub fn extend(&mut self, diagnostics: Self) {
        self.diagnostics.extend(diagnostics.diagnostics);
    }

    /// Orders diagnostics by their primary source position for deterministic presentation.
    pub fn sort_by_span(&mut self) {
        self.diagnostics.sort_by_key(|diagnostic| {
            (
                diagnostic.span.source_id,
                diagnostic.span.start_byte,
                diagnostic.span.end_byte,
            )
        });
    }

    /// Returns whether this collection contains no diagnostics.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// Renders compiler diagnostics with source excerpts for terminal output.
    #[must_use]
    pub fn render(&self, source: &str) -> String {
        let mut output = String::new();
        for (index, diagnostic) in self.diagnostics.iter().enumerate() {
            if index > 0 {
                output.push('\n');
            }
            render_diagnostic(&mut output, diagnostic, source, "");
        }
        output
    }
}

impl<'a> Default for CompileDiagnostics<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for CompileDiagnostics<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, diagnostic) in self.diagnostics.iter().enumerate() {
            if index > 0 {
                writeln!(formatter)?;
            }
            write!(formatter, "{diagnostic}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CompileDiagnostics<'_> {}

/// Classifies stable diagnostic codes without requiring every construction site to repeat it.
fn category_for(code: &str) -> CompileDiagnosticCategory {
    if code.starts_with("E00") {
        CompileDiagnosticCategory::Lexical
    } else if code.starts_with("E01") {
        CompileDiagnosticCategory::Syntax
    } else if code.starts_with("E09") || code.starts_with("E1") {
        CompileDiagnosticCategory::Internal
    } else {
        CompileDiagnosticCategory::Semantic
    }
}

/// Appends one compiler diagnostic in the CLI report shape.
fn render_diagnostic(
    output: &mut String,
    diagnostic: &CompileDiagnostic<'_>,
    source: &str,
    indent: &str,
) {
    output.push_str(indent);
    output.push_str("error: ");
    output.push_str(diagnostic.code);
    output.push_str(" (compile ");
    output.push_str(&diagnostic.category.to_string());
    output.push_str(")\n");
    output.push_str(indent);
    output.push_str("  message: ");
    output.push_str(&diagnostic.message);
    output.push('\n');
    output.push_str(indent);
    output.push_str("  origin: ");
    output.push_str(diagnostic.span.source_id);
    output.push(':');
    let (line, column) = line_and_column(source, diagnostic.span.start_byte);
    output.push_str(&line.to_string());
    output.push(':');
    output.push_str(&column.to_string());
    output.push('\n');
    append_source_excerpt(output, source, diagnostic.span, indent);
    for related in &diagnostic.related {
        output.push_str(indent);
        output.push_str("  related: ");
        output.push_str(&related.message);
        output.push('\n');
        append_source_excerpt(output, source, related.span, indent);
    }
}

/// Appends one source line and a caret for a compiler diagnostic location.
fn append_source_excerpt(output: &mut String, source: &str, span: SourceSpan<'_>, indent: &str) {
    let (line, column) = line_and_column(source, span.start_byte);
    let Some(source_line) = source.lines().nth(line.saturating_sub(1)) else {
        return;
    };
    output.push_str(indent);
    output.push_str("    ");
    output.push_str(&line.to_string());
    output.push_str(" | ");
    output.push_str(source_line);
    output.push('\n');
    output.push_str(indent);
    output.push_str("      | ");
    output.push_str(&" ".repeat(column.saturating_sub(1)));
    output.push('^');
    output.push('\n');
}

/// Converts one UTF-8 byte offset into a one-based line and Unicode-scalar column number.
fn line_and_column(source: &str, offset: u32) -> (usize, usize) {
    let offset = usize::try_from(offset)
        .unwrap_or(source.len())
        .min(source.len());
    let prefix = source.get(..offset).unwrap_or(source);
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .map_or(1, |line| line.chars().count() + 1);
    (line, column)
}
