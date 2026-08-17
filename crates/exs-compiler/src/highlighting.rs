//! Tolerant source tokens and parser-aware highlighting for editor integrations.

use std::collections::BTreeMap;

use crate::SourceInput;
use crate::ast::{
    AssignmentTarget, Block, ElseBranch, Expression, FunctionDeclaration, Identifier, MatchArmBody,
    MatchPattern, Module, ObjectProperty, Parameter, Statement, TypeAnnotation,
};
use crate::diagnostic::SourceSpan;
use crate::lexer::{Token, TokenKind};

/// One stable category of source text suitable for an editor highlighter.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum HighlightKind {
    /// A language keyword or built-in literal keyword.
    Keyword,
    /// A quoted, raw, or dedented string literal.
    String,
    /// A numeric literal.
    Number,
    /// An operator token.
    Operator,
    /// Delimiter or separator punctuation.
    Punctuation,
    /// A declared or referenced nominal type or trait.
    Type,
    /// A function name or direct function call.
    Function,
    /// A method name.
    Method,
    /// A named object field or property.
    Field,
    /// An enum variant name.
    Variant,
    /// A local binding declaration.
    Binding,
    /// A local variable reference.
    Variable,
    /// A line or block comment.
    Comment,
}

/// One byte range and its parser-aware highlighting category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HighlightSpan {
    /// Inclusive UTF-8 byte offset in the supplied source text.
    pub start: usize,
    /// Exclusive UTF-8 byte offset in the supplied source text.
    pub end: usize,
    /// Semantic display category.
    pub kind: HighlightKind,
}

/// A tolerant source token returned for editor features that work on incomplete input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceToken {
    /// Source spelling of this token.
    pub text: String,
    /// Inclusive UTF-8 byte offset in the supplied source text.
    pub start: usize,
    /// Exclusive UTF-8 byte offset in the supplied source text.
    pub end: usize,
    /// Broad lexical category of this token.
    pub kind: SourceTokenKind,
}

/// One source comment retained as trivia for editor features.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceComment {
    /// Inclusive UTF-8 byte offset in the supplied source text.
    pub start: usize,
    /// Exclusive UTF-8 byte offset in the supplied source text.
    pub end: usize,
    /// Whether a cursor exactly at `end` is still inside this comment.
    pub includes_end: bool,
}

/// Tolerant lexical analysis shared by editor integrations.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SourceLex {
    /// Tokens recognized after malformed fragments were skipped during recovery.
    pub tokens: Vec<SourceToken>,
    /// Comments skipped by the token stream.
    pub comments: Vec<SourceComment>,
    source_len: usize,
}

impl SourceLex {
    /// Returns whether one byte offset lies inside a comment.
    #[must_use]
    pub fn is_comment_position(&self, offset: usize) -> bool {
        let cursor = offset.min(self.source_len);
        self.comments.iter().any(|comment| {
            comment.start < cursor
                && (cursor < comment.end || (comment.includes_end && cursor == comment.end))
        })
    }
}

/// Broad source-token categories that remain stable across parser implementation details.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceTokenKind {
    /// A user-defined identifier.
    Identifier,
    /// A reserved language keyword.
    Keyword,
    /// A numeric literal.
    Number,
    /// A quoted, raw, or dedented string literal.
    String,
    /// An operator token.
    Operator,
    /// Delimiter or separator punctuation.
    Punctuation,
}

/// Returns tolerant lexical tokens for one source document.
///
/// Malformed fragments are omitted after lexer recovery so editor features can continue to work
/// while the document is being edited.
#[must_use]
pub fn tokens(source: SourceInput<'_>) -> Vec<SourceToken> {
    source_lex(source).tokens
}

/// Returns tolerant tokens and comment trivia for one source document.
#[must_use]
pub fn source_lex(source: SourceInput<'_>) -> SourceLex {
    let lexed = crate::lexer::lex(source);
    SourceLex {
        tokens: lexed
            .tokens
            .into_iter()
            .filter_map(|token| source_token(source.text, token))
            .collect(),
        comments: lexed
            .comments
            .into_iter()
            .filter_map(|span| source_comment(source.text, span))
            .collect(),
        source_len: source.text.len(),
    }
}

/// Produces lexical spans and parser-aware identifier spans for one source document.
///
/// Lexical spans are returned even when the source does not parse. Semantic identifier categories
/// replace their lexical counterparts only after a successful parse.
#[must_use]
pub fn highlight(source: SourceInput<'_>) -> Vec<HighlightSpan> {
    let lexed = crate::lexer::lex(source);
    let mut spans = BTreeMap::new();
    for token in &lexed.tokens {
        if let Some(kind) = lexical_highlight_kind(&token.kind) {
            insert_span(&mut spans, token.span, kind, source.text.len());
        }
    }
    for span in &lexed.comments {
        insert_span(&mut spans, *span, HighlightKind::Comment, source.text.len());
    }
    if let Ok(module) = crate::parser::parse(source.source_id, lexed.tokens, false) {
        highlight_module(&module, &mut spans, source.text.len());
    }
    spans
        .into_iter()
        .map(|((start, end), kind)| HighlightSpan { start, end, kind })
        .collect()
}

/// Converts one internal lexer token into the public, tolerant token representation.
fn source_token(source: &str, token: Token<'_>) -> Option<SourceToken> {
    let kind = source_token_kind(&token.kind)?;
    let start = usize::try_from(token.span.start_byte).ok()?;
    let end = usize::try_from(token.span.end_byte).ok()?;
    Some(SourceToken {
        text: source.get(start..end)?.to_owned(),
        start,
        end,
        kind,
    })
}

/// Converts one compiler comment span into editor trivia with precise cursor-boundary behavior.
fn source_comment(source: &str, span: SourceSpan<'_>) -> Option<SourceComment> {
    let start = usize::try_from(span.start_byte).ok()?;
    let end = usize::try_from(span.end_byte).ok()?;
    let text = source.get(start..end)?;
    let is_line_comment = text.starts_with("//");
    let includes_end =
        (is_line_comment && end == source.len()) || (!is_line_comment && !text.ends_with("*/"));
    Some(SourceComment {
        start,
        end,
        includes_end,
    })
}

/// Maps one lexer token to the coarse public category used by editor integrations.
fn source_token_kind(kind: &TokenKind) -> Option<SourceTokenKind> {
    match kind {
        TokenKind::Eof => None,
        TokenKind::Identifier(_) => Some(SourceTokenKind::Identifier),
        TokenKind::Integer(_) | TokenKind::Float(_) => Some(SourceTokenKind::Number),
        TokenKind::String(_) | TokenKind::FormattedString(_) => Some(SourceTokenKind::String),
        kind if is_keyword(kind) => Some(SourceTokenKind::Keyword),
        kind if is_operator(kind) => Some(SourceTokenKind::Operator),
        _ => Some(SourceTokenKind::Punctuation),
    }
}

/// Maps one lexer token to its purely lexical highlighting category.
fn lexical_highlight_kind(kind: &TokenKind) -> Option<HighlightKind> {
    match source_token_kind(kind) {
        Some(SourceTokenKind::Keyword) => Some(HighlightKind::Keyword),
        Some(SourceTokenKind::Number) => Some(HighlightKind::Number),
        Some(SourceTokenKind::String) => Some(HighlightKind::String),
        Some(SourceTokenKind::Operator) => Some(HighlightKind::Operator),
        Some(SourceTokenKind::Punctuation) => Some(HighlightKind::Punctuation),
        Some(SourceTokenKind::Identifier) | None => None,
    }
}

/// Returns whether a token is one of ExS's reserved keywords.
fn is_keyword(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Fn
            | TokenKind::Import
            | TokenKind::Use
            | TokenKind::As
            | TokenKind::Type
            | TokenKind::Enum
            | TokenKind::Match
            | TokenKind::Trait
            | TokenKind::Impl
            | TokenKind::Let
            | TokenKind::Ret
            | TokenKind::If
            | TokenKind::Else
            | TokenKind::While
            | TokenKind::For
            | TokenKind::In
            | TokenKind::Break
            | TokenKind::Continue
            | TokenKind::None
            | TokenKind::Is
            | TokenKind::Error
            | TokenKind::Host
            | TokenKind::Par
            | TokenKind::True
            | TokenKind::False
    )
}

/// Returns whether a token is displayed as an operator rather than punctuation.
fn is_operator(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Bang
            | TokenKind::Equal
            | TokenKind::EqualEqual
            | TokenKind::BangEqual
            | TokenKind::Less
            | TokenKind::LessEqual
            | TokenKind::Greater
            | TokenKind::GreaterEqual
            | TokenKind::AndAnd
            | TokenKind::OrOr
            | TokenKind::Pipe
            | TokenKind::Question
            | TokenKind::Arrow
            | TokenKind::FatArrow
    )
}

/// Adds semantic spans for every parsed declaration and expression in one module.
fn highlight_module(
    module: &Module<'_>,
    spans: &mut BTreeMap<(usize, usize), HighlightKind>,
    len: usize,
) {
    for import in &module.imports {
        if let Some(alias) = &import.alias {
            highlight_identifier(alias, HighlightKind::Binding, spans, len);
        }
    }
    for use_declaration in &module.uses {
        highlight_identifier(&use_declaration.namespace, HighlightKind::Type, spans, len);
        for item in &use_declaration.items {
            highlight_identifier(&item.name, HighlightKind::Type, spans, len);
            if let Some(alias) = &item.alias {
                highlight_identifier(alias, HighlightKind::Binding, spans, len);
            }
        }
    }
    for declaration in &module.types {
        highlight_identifier(&declaration.name, HighlightKind::Type, spans, len);
        for field in &declaration.fields {
            highlight_identifier(&field.name, HighlightKind::Field, spans, len);
            highlight_type_annotation(field.type_annotation.as_ref(), spans, len);
        }
    }
    for declaration in &module.enums {
        highlight_identifier(&declaration.name, HighlightKind::Type, spans, len);
        for variant in &declaration.variants {
            highlight_identifier(&variant.name, HighlightKind::Variant, spans, len);
            for field in &variant.fields {
                highlight_identifier(&field.name, HighlightKind::Field, spans, len);
                highlight_type_annotation(field.type_annotation.as_ref(), spans, len);
            }
        }
    }
    for declaration in &module.traits {
        highlight_identifier(&declaration.name, HighlightKind::Type, spans, len);
        for method in &declaration.methods {
            highlight_identifier(&method.name, HighlightKind::Method, spans, len);
            for parameter in &method.parameters {
                highlight_parameter(parameter, spans, len);
            }
            highlight_type_annotation(method.return_type.as_ref(), spans, len);
            if let Some(body) = &method.body {
                highlight_block(body, spans, len);
            }
        }
    }
    for implementation in &module.implementations {
        if let Some(trait_name) = &implementation.trait_name {
            highlight_identifier(trait_name, HighlightKind::Type, spans, len);
        }
        highlight_identifier(&implementation.type_name, HighlightKind::Type, spans, len);
        for method in &implementation.methods {
            highlight_function(method, HighlightKind::Method, spans, len);
        }
    }
    for function in &module.functions {
        highlight_function(function, HighlightKind::Function, spans, len);
    }
}

/// Adds semantic spans for one function or method declaration.
fn highlight_function(
    function: &FunctionDeclaration<'_>,
    kind: HighlightKind,
    spans: &mut BTreeMap<(usize, usize), HighlightKind>,
    len: usize,
) {
    highlight_identifier(&function.name, kind, spans, len);
    for parameter in &function.parameters {
        highlight_parameter(parameter, spans, len);
    }
    highlight_type_annotation(function.return_type.as_ref(), spans, len);
    highlight_block(&function.body, spans, len);
}

/// Adds semantic spans for one function parameter.
fn highlight_parameter(
    parameter: &Parameter<'_>,
    spans: &mut BTreeMap<(usize, usize), HighlightKind>,
    len: usize,
) {
    highlight_identifier(&parameter.name, HighlightKind::Binding, spans, len);
    highlight_type_annotation(parameter.type_annotation.as_ref(), spans, len);
}

/// Adds semantic spans for a union type annotation when present.
fn highlight_type_annotation(
    annotation: Option<&TypeAnnotation<'_>>,
    spans: &mut BTreeMap<(usize, usize), HighlightKind>,
    len: usize,
) {
    if let Some(annotation) = annotation {
        for member in &annotation.members {
            insert_span(spans, member.span, HighlightKind::Type, len);
        }
    }
}

/// Adds semantic spans for every statement in one block.
fn highlight_block(
    block: &Block<'_>,
    spans: &mut BTreeMap<(usize, usize), HighlightKind>,
    len: usize,
) {
    for statement in &block.statements {
        match statement {
            Statement::Let { name, value, .. } => {
                highlight_identifier(name, HighlightKind::Binding, spans, len);
                highlight_expression(value, spans, len);
            }
            Statement::Assign { target, value, .. } => {
                highlight_assignment_target(target, spans, len);
                highlight_expression(value, spans, len);
            }
            Statement::Return { value, .. } => {
                if let Some(value) = value {
                    highlight_expression(value, spans, len);
                }
            }
            Statement::Block { block, .. } => highlight_block(block, spans, len),
            Statement::If {
                condition,
                then_block,
                else_branch,
                ..
            } => {
                highlight_expression(condition, spans, len);
                highlight_block(then_block, spans, len);
                if let Some(else_branch) = else_branch {
                    match else_branch {
                        ElseBranch::Block(block) => highlight_block(block, spans, len),
                        ElseBranch::If(statement) => highlight_if_statement(statement, spans, len),
                    }
                }
            }
            Statement::While {
                condition, body, ..
            } => {
                highlight_expression(condition, spans, len);
                highlight_block(body, spans, len);
            }
            Statement::For {
                binding,
                iterable,
                body,
                ..
            } => {
                highlight_identifier(binding, HighlightKind::Binding, spans, len);
                highlight_expression(iterable, spans, len);
                highlight_block(body, spans, len);
            }
            Statement::Break { .. } | Statement::Continue { .. } => {}
            Statement::Expression { expression, .. } => {
                highlight_expression(expression, spans, len)
            }
        }
    }
}

/// Adds semantic spans for a nested conditional branch without cloning its AST node.
fn highlight_if_statement(
    statement: &Statement<'_>,
    spans: &mut BTreeMap<(usize, usize), HighlightKind>,
    len: usize,
) {
    let Statement::If {
        condition,
        then_block,
        else_branch,
        ..
    } = statement
    else {
        return;
    };
    highlight_expression(condition, spans, len);
    highlight_block(then_block, spans, len);
    if let Some(else_branch) = else_branch {
        match else_branch {
            ElseBranch::Block(block) => highlight_block(block, spans, len),
            ElseBranch::If(statement) => highlight_if_statement(statement, spans, len),
        }
    }
}

/// Adds semantic spans for one assignment target.
fn highlight_assignment_target(
    target: &AssignmentTarget<'_>,
    spans: &mut BTreeMap<(usize, usize), HighlightKind>,
    len: usize,
) {
    match target {
        AssignmentTarget::Variable(name) => {
            highlight_identifier(name, HighlightKind::Variable, spans, len)
        }
        AssignmentTarget::Index {
            receiver, index, ..
        } => {
            highlight_expression(receiver, spans, len);
            highlight_expression(index, spans, len);
        }
        AssignmentTarget::Property {
            receiver, property, ..
        } => {
            highlight_expression(receiver, spans, len);
            highlight_identifier(property, HighlightKind::Field, spans, len);
        }
    }
}

/// Adds semantic spans for one expression and its descendants.
fn highlight_expression(
    expression: &Expression<'_>,
    spans: &mut BTreeMap<(usize, usize), HighlightKind>,
    len: usize,
) {
    match expression {
        Expression::Integer(..)
        | Expression::Float(..)
        | Expression::String(..)
        | Expression::Bool(..)
        | Expression::None(..) => {}
        Expression::FormattedString { parts, .. } => {
            for part in parts {
                if let crate::ast::FormattedStringPart::Expression(expression) = part {
                    highlight_expression(expression, spans, len);
                }
            }
        }
        Expression::IsError { value, .. }
        | Expression::Propagate { value, .. }
        | Expression::Unary { operand: value, .. } => highlight_expression(value, spans, len),
        Expression::List { elements, .. } => {
            for element in elements {
                highlight_expression(element, spans, len);
            }
        }
        Expression::Object { properties, .. } => highlight_properties(properties, spans, len),
        Expression::TypedObject {
            type_name,
            properties,
            ..
        } => {
            highlight_identifier(type_name, HighlightKind::Type, spans, len);
            highlight_properties(properties, spans, len);
        }
        Expression::Match { value, arms, .. } => {
            highlight_expression(value, spans, len);
            for arm in arms {
                match &arm.pattern {
                    MatchPattern::Variant {
                        type_name,
                        variant,
                        bindings,
                        ..
                    } => {
                        highlight_identifier(type_name, HighlightKind::Type, spans, len);
                        highlight_identifier(variant, HighlightKind::Variant, spans, len);
                        for binding in bindings {
                            highlight_identifier(binding, HighlightKind::Binding, spans, len);
                        }
                    }
                    MatchPattern::Wildcard(..) => {}
                }
                match &arm.body {
                    MatchArmBody::Expression(expression) => {
                        highlight_expression(expression, spans, len)
                    }
                    MatchArmBody::Block(block) => highlight_block(block, spans, len),
                }
            }
        }
        Expression::Variable(identifier) => {
            highlight_identifier(identifier, HighlightKind::Variable, spans, len)
        }
        Expression::Closure {
            parameters, body, ..
        } => {
            for parameter in parameters {
                highlight_parameter(parameter, spans, len);
            }
            highlight_block(body, spans, len);
        }
        Expression::ParallelStatic { tasks, .. } => {
            for task in tasks {
                highlight_expression(task, spans, len);
            }
        }
        Expression::ParallelDynamic { functions, .. } => {
            highlight_expression(functions, spans, len)
        }
        Expression::Binary { left, right, .. } => {
            highlight_expression(left, spans, len);
            highlight_expression(right, spans, len);
        }
        Expression::Call {
            callee, arguments, ..
        } => {
            highlight_identifier(callee, HighlightKind::Function, spans, len);
            for argument in arguments {
                highlight_expression(argument, spans, len);
            }
        }
        Expression::HostCall {
            name, arguments, ..
        } => {
            highlight_expression(name, spans, len);
            for argument in arguments {
                highlight_expression(argument, spans, len);
            }
        }
        Expression::MethodCall {
            receiver,
            method,
            arguments,
            ..
        } => {
            highlight_expression(receiver, spans, len);
            highlight_identifier(method, HighlightKind::Method, spans, len);
            for argument in arguments {
                highlight_expression(argument, spans, len);
            }
        }
        Expression::StaticMethodCall {
            type_name,
            method,
            arguments,
            ..
        } => {
            highlight_identifier(type_name, HighlightKind::Type, spans, len);
            highlight_identifier(method, HighlightKind::Method, spans, len);
            for argument in arguments {
                highlight_expression(argument, spans, len);
            }
        }
        Expression::Index {
            receiver, index, ..
        } => {
            highlight_expression(receiver, spans, len);
            highlight_expression(index, spans, len);
        }
        Expression::Property {
            receiver, property, ..
        } => {
            highlight_expression(receiver, spans, len);
            highlight_identifier(property, HighlightKind::Field, spans, len);
        }
    }
}

/// Adds field spans and recursively highlights their values.
fn highlight_properties(
    properties: &[ObjectProperty<'_>],
    spans: &mut BTreeMap<(usize, usize), HighlightKind>,
    len: usize,
) {
    for property in properties {
        insert_span(spans, property.key_span, HighlightKind::Field, len);
        highlight_expression(&property.value, spans, len);
    }
}

/// Inserts a semantic category for one identifier span.
fn highlight_identifier(
    identifier: &Identifier<'_>,
    kind: HighlightKind,
    spans: &mut BTreeMap<(usize, usize), HighlightKind>,
    len: usize,
) {
    insert_span(spans, identifier.span, kind, len);
}

/// Converts one compiler span into an editor span after validating it against the source text.
fn insert_span(
    spans: &mut BTreeMap<(usize, usize), HighlightKind>,
    span: SourceSpan<'_>,
    kind: HighlightKind,
    len: usize,
) {
    let (Ok(start), Ok(end)) = (
        usize::try_from(span.start_byte),
        usize::try_from(span.end_byte),
    ) else {
        return;
    };
    if start < end && end <= len {
        spans.insert((start, end), kind);
    }
}

#[cfg(test)]
mod tests {
    use super::{HighlightKind, highlight, tokens};
    use crate::SourceInput;

    /// Returns the category assigned to the selected source occurrence.
    fn kind_at(source: &str, needle: &str, occurrence: usize) -> Option<HighlightKind> {
        let start = source.match_indices(needle).nth(occurrence)?.0;
        let end = start + needle.len();
        highlight(SourceInput {
            source_id: "highlight-test.exs",
            text: source,
        })
        .into_iter()
        .find(|span| span.start == start && span.end == end)
        .map(|span| span.kind)
    }

    /// Uses parser structure to distinguish declarations, references, and members.
    #[test]
    fn highlights_semantic_identifier_roles() {
        let source = r#"
            type User { name: String, }
            enum Color { Blue, }
            fn main(value: Int) {
                let user = User { name: "Ada" };
                ret user.name;
            }
        "#;
        assert_eq!(kind_at(source, "User", 0), Some(HighlightKind::Type));
        assert_eq!(kind_at(source, "name", 0), Some(HighlightKind::Field));
        assert_eq!(kind_at(source, "Blue", 0), Some(HighlightKind::Variant));
        assert_eq!(kind_at(source, "main", 0), Some(HighlightKind::Function));
        assert_eq!(kind_at(source, "value", 0), Some(HighlightKind::Binding));
        assert_eq!(kind_at(source, "user", 0), Some(HighlightKind::Binding));
        assert_eq!(kind_at(source, "user", 1), Some(HighlightKind::Variable));
        assert_eq!(kind_at(source, "name", 1), Some(HighlightKind::Field));
    }

    /// Retains lexical colors while a partially typed document cannot be parsed.
    #[test]
    fn highlights_incomplete_source_lexically() {
        let source = "fn main( // incomplete";
        assert_eq!(kind_at(source, "fn", 0), Some(HighlightKind::Keyword));
        assert_eq!(
            kind_at(source, "// incomplete", 0),
            Some(HighlightKind::Comment)
        );
    }

    /// Keeps comment markers inside strings classified as strings rather than comments.
    #[test]
    fn does_not_treat_comment_markers_inside_strings_as_comments() {
        let source = r#"fn main() { ret "// text"; }"#;
        assert_eq!(
            kind_at(source, r#""// text""#, 0),
            Some(HighlightKind::String)
        );
        assert_eq!(kind_at(source, "// text", 0), None);
    }

    /// Preserves an unterminated block comment as editor trivia through the end of the document.
    #[test]
    fn highlights_unterminated_block_comments() {
        let source = "fn main() { /* incomplete";
        assert_eq!(
            kind_at(source, "/* incomplete", 0),
            Some(HighlightKind::Comment)
        );
    }

    /// Returns compiler-lexed tokens for editor features on incomplete source.
    #[test]
    fn returns_tolerant_source_tokens() {
        let source = "fn main(";
        let text = tokens(SourceInput {
            source_id: "tokens-test.exs",
            text: source,
        })
        .into_iter()
        .map(|token| token.text)
        .collect::<Vec<_>>();
        assert_eq!(text, ["fn", "main", "("]);
    }

    /// Retains the comment boundaries needed to suppress completions while editing.
    #[test]
    fn retains_comment_cursor_positions() {
        let source = "// comment\n/* incomplete";
        let lexed = super::source_lex(SourceInput {
            source_id: "comment-test.exs",
            text: source,
        });
        assert!(lexed.is_comment_position(4));
        assert!(!lexed.is_comment_position(10));
        assert!(lexed.is_comment_position(source.len()));
    }
}
