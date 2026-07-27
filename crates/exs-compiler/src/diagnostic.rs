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

/// One stable source-spanned compiler error.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CompileDiagnostic<'a> {
    /// Stable machine-readable diagnostic code.
    pub code: &'static str,
    /// Primary source span.
    pub span: SourceSpan<'a>,
    /// Concise diagnostic message.
    pub message: String,
}

impl<'a> CompileDiagnostic<'a> {
    /// Creates a compiler diagnostic.
    #[must_use]
    pub fn new(code: &'static str, span: SourceSpan<'a>, message: impl Into<String>) -> Self {
        Self {
            code,
            span,
            message: message.into(),
        }
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
