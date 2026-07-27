//! Lexer for the Phase-1 `ExS` grammar.

use crate::SourceInput;
use crate::diagnostic::{CompileDiagnostic, SourceSpan};

/// A token plus its source span.
#[derive(Debug, Clone)]
pub struct Token<'a> {
    /// Token kind.
    pub kind: TokenKind,
    /// Source span.
    pub span: SourceSpan<'a>,
}

/// Tokens recognized by the Phase-1 lexer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    /// An identifier spelling.
    Identifier(String),
    /// A parsed decimal integer.
    Integer(i64),
    /// The `fn` keyword.
    Fn,
    /// The `let` keyword.
    Let,
    /// The `ret` keyword.
    Ret,
    /// The `if` keyword.
    If,
    /// The `else` keyword.
    Else,
    /// The `true` keyword.
    True,
    /// The `false` keyword.
    False,
    /// `(`.
    LeftParen,
    /// `)`.
    RightParen,
    /// `{`.
    LeftBrace,
    /// `}`.
    RightBrace,
    /// `,`.
    Comma,
    /// `;`.
    Semicolon,
    /// `+`.
    Plus,
    /// `-`.
    Minus,
    /// `*`.
    Star,
    /// `!`.
    Bang,
    /// `=`.
    Equal,
    /// `==`.
    EqualEqual,
    /// `!=`.
    BangEqual,
    /// `<`.
    Less,
    /// `<=`.
    LessEqual,
    /// `>`.
    Greater,
    /// `>=`.
    GreaterEqual,
    /// `&&`.
    AndAnd,
    /// `||`.
    OrOr,
    /// End of source.
    Eof,
}

/// Lexes a UTF-8 source input into Phase-1 tokens.
pub fn lex<'a>(source: SourceInput<'a>) -> Result<Vec<Token<'a>>, CompileDiagnostic<'a>> {
    let bytes = source.text.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_whitespace() {
            index += 1;
            continue;
        }
        if bytes[index..].starts_with(b"//") {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if bytes[index..].starts_with(b"/*") {
            let start = index;
            index += 2;
            while index + 1 < bytes.len() && !bytes[index..].starts_with(b"*/") {
                index += 1;
            }
            if index + 1 == bytes.len() {
                return Err(diagnostic(
                    source,
                    start,
                    bytes.len(),
                    "E0002",
                    "unterminated block comment",
                ));
            }
            index += 2;
            continue;
        }

        let start = index;
        let token = if byte.is_ascii_digit() {
            index += 1;
            while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'_') {
                index += 1;
            }
            let literal = &source.text[start..index];
            if literal.starts_with('_') || literal.ends_with('_') || literal.contains("__") {
                return Err(diagnostic(
                    source,
                    start,
                    index,
                    "E0003",
                    "invalid integer separator",
                ));
            }
            let numeric = literal.replace('_', "");
            let value = numeric.parse::<i64>().map_err(|_| {
                diagnostic(
                    source,
                    start,
                    index,
                    "E0004",
                    "integer literal is outside i64 range",
                )
            })?;
            TokenKind::Integer(value)
        } else if let Some(character) = source.text[index..].chars().next() {
            if character == '_' || character.is_alphabetic() {
                index += character.len_utf8();
                while index < bytes.len() {
                    let Some(next) = source.text[index..].chars().next() else {
                        break;
                    };
                    if next == '_' || next.is_alphanumeric() {
                        index += next.len_utf8();
                    } else {
                        break;
                    }
                }
                keyword_or_identifier(&source.text[start..index])
            } else {
                index += 1;
                match byte {
                    b'(' => TokenKind::LeftParen,
                    b')' => TokenKind::RightParen,
                    b'{' => TokenKind::LeftBrace,
                    b'}' => TokenKind::RightBrace,
                    b',' => TokenKind::Comma,
                    b';' => TokenKind::Semicolon,
                    b'+' => TokenKind::Plus,
                    b'-' => TokenKind::Minus,
                    b'*' => TokenKind::Star,
                    b'!' if bytes.get(index) == Some(&b'=') => {
                        index += 1;
                        TokenKind::BangEqual
                    }
                    b'!' => TokenKind::Bang,
                    b'=' if bytes.get(index) == Some(&b'=') => {
                        index += 1;
                        TokenKind::EqualEqual
                    }
                    b'=' => TokenKind::Equal,
                    b'<' if bytes.get(index) == Some(&b'=') => {
                        index += 1;
                        TokenKind::LessEqual
                    }
                    b'<' => TokenKind::Less,
                    b'>' if bytes.get(index) == Some(&b'=') => {
                        index += 1;
                        TokenKind::GreaterEqual
                    }
                    b'>' => TokenKind::Greater,
                    b'&' if bytes.get(index) == Some(&b'&') => {
                        index += 1;
                        TokenKind::AndAnd
                    }
                    b'|' if bytes.get(index) == Some(&b'|') => {
                        index += 1;
                        TokenKind::OrOr
                    }
                    _ => {
                        return Err(diagnostic(
                            source,
                            start,
                            index,
                            "E0001",
                            format!("unexpected character `{character}`"),
                        ));
                    }
                }
            }
        } else {
            break;
        };
        tokens.push(Token {
            kind: token,
            span: span(source, start, index),
        });
    }

    tokens.push(Token {
        kind: TokenKind::Eof,
        span: span(source, bytes.len(), bytes.len()),
    });
    Ok(tokens)
}

fn keyword_or_identifier(value: &str) -> TokenKind {
    match value {
        "fn" => TokenKind::Fn,
        "let" => TokenKind::Let,
        "ret" => TokenKind::Ret,
        "if" => TokenKind::If,
        "else" => TokenKind::Else,
        "true" => TokenKind::True,
        "false" => TokenKind::False,
        _ => TokenKind::Identifier(value.to_owned()),
    }
}

fn diagnostic<'a>(
    source: SourceInput<'a>,
    start: usize,
    end: usize,
    code: &'static str,
    message: impl Into<String>,
) -> CompileDiagnostic<'a> {
    CompileDiagnostic::new(code, span(source, start, end), message)
}

fn span(source: SourceInput<'_>, start: usize, end: usize) -> SourceSpan<'_> {
    SourceSpan {
        source_id: source.source_id,
        start_byte: start as u32,
        end_byte: end as u32,
    }
}
