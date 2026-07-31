//! Lexer for the Phase-1 `ExS` grammar.

use crate::SourceInput;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// A token plus its source span.
#[derive(Debug, Clone)]
pub struct Token<'a> {
    /// Token kind.
    pub kind: TokenKind,
    /// Source span.
    pub span: SourceSpan<'a>,
}

/// Tokens recognized by the Phase-1 lexer.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// An identifier spelling.
    Identifier(String),
    /// A parsed decimal integer.
    Integer(i64),
    /// A parsed binary64 floating-point literal.
    Float(f64),
    /// A decoded UTF-8 string literal.
    String(String),
    /// The `fn` keyword.
    Fn,
    /// The `type` keyword.
    Type,
    /// The `trait` keyword.
    Trait,
    /// The `impl` keyword.
    Impl,
    /// The `let` keyword.
    Let,
    /// The `ret` keyword.
    Ret,
    /// The `if` keyword.
    If,
    /// The `else` keyword.
    Else,
    /// The while keyword.
    While,
    /// The for keyword.
    For,
    /// The in keyword.
    In,
    /// The break keyword.
    Break,
    /// The continue keyword.
    Continue,
    /// The None keyword.
    None,
    /// The is keyword.
    Is,
    /// The Error keyword.
    Error,
    /// The reserved host boundary keyword.
    Host,
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
    /// `[`.
    LeftBracket,
    /// `]`.
    RightBracket,
    /// `.`.
    Dot,
    /// `:`.
    Colon,
    /// `::`.
    DoubleColon,
    /// `,`.
    Comma,
    /// `;`.
    Semicolon,
    /// `+`.
    Plus,
    /// `-`.
    Minus,
    /// `->`.
    Arrow,
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
    /// `|`.
    Pipe,
    /// `?`.
    Question,
    /// End of source.
    Eof,
}

/// The best-effort output of tokenizing one ExS source input.
pub struct Lexed<'a> {
    /// Tokens recognized after malformed source fragments were skipped.
    pub tokens: Vec<Token<'a>>,
    /// All lexical diagnostics encountered while tokenizing the source.
    pub diagnostics: CompileDiagnostics<'a>,
}

/// Lexes a UTF-8 source input while recovering after malformed token fragments.
pub fn lex<'a>(source: SourceInput<'a>) -> Lexed<'a> {
    let bytes = source.text.as_bytes();
    let mut tokens = Vec::new();
    let mut diagnostics = CompileDiagnostics::new();
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
                diagnostics.push(diagnostic(
                    source,
                    start,
                    bytes.len(),
                    "E0002",
                    "unterminated block comment",
                ));
                break;
            }
            index += 2;
            continue;
        }

        let start = index;
        let token = if byte.is_ascii_digit() {
            let integer_start = index;
            index = consume_digits(bytes, index);
            if !valid_digit_segment(&source.text[integer_start..index]) {
                diagnostics.push(diagnostic(
                    source,
                    start,
                    index,
                    "E0003",
                    "invalid numeric separator",
                ));
                continue;
            }
            let mut is_float = false;
            if bytes.get(index) == Some(&b'.') {
                is_float = true;
                index += 1;
                let fraction_start = index;
                index = consume_digits(bytes, index);
                if !valid_digit_segment(&source.text[fraction_start..index]) {
                    diagnostics.push(diagnostic(
                        source,
                        start,
                        index,
                        "E0003",
                        "invalid floating-point fraction",
                    ));
                    continue;
                }
            }
            if matches!(bytes.get(index), Some(b'e' | b'E')) {
                is_float = true;
                index += 1;
                if matches!(bytes.get(index), Some(b'+' | b'-')) {
                    index += 1;
                }
                let exponent_start = index;
                index = consume_digits(bytes, index);
                if !valid_digit_segment(&source.text[exponent_start..index]) {
                    diagnostics.push(diagnostic(
                        source,
                        start,
                        index,
                        "E0003",
                        "invalid floating-point exponent",
                    ));
                    continue;
                }
            }
            let numeric = source.text[start..index].replace('_', "");
            if is_float {
                let value = match numeric.parse::<f64>() {
                    Ok(value) => value,
                    Err(_) => {
                        diagnostics.push(diagnostic(
                            source,
                            start,
                            index,
                            "E0005",
                            "invalid floating-point literal",
                        ));
                        continue;
                    }
                };
                TokenKind::Float(value)
            } else {
                let value = match numeric.parse::<i64>() {
                    Ok(value) => value,
                    Err(_) => {
                        diagnostics.push(diagnostic(
                            source,
                            start,
                            index,
                            "E0004",
                            "integer literal is outside i64 range",
                        ));
                        continue;
                    }
                };
                TokenKind::Integer(value)
            }
        } else if byte == b'"' {
            match string_literal(source, &mut index, start) {
                Ok(token) => token,
                Err(error) => {
                    diagnostics.push(error);
                    recover_string(bytes, &mut index);
                    continue;
                }
            }
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
                    b'[' => TokenKind::LeftBracket,
                    b']' => TokenKind::RightBracket,
                    b'.' => TokenKind::Dot,
                    b':' if bytes.get(index) == Some(&b':') => {
                        index += 1;
                        TokenKind::DoubleColon
                    }
                    b':' => TokenKind::Colon,
                    b',' => TokenKind::Comma,
                    b';' => TokenKind::Semicolon,
                    b'+' => TokenKind::Plus,
                    b'-' if bytes.get(index) == Some(&b'>') => {
                        index += 1;
                        TokenKind::Arrow
                    }
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
                    b'|' => TokenKind::Pipe,
                    b'?' => TokenKind::Question,
                    _ => {
                        diagnostics.push(diagnostic(
                            source,
                            start,
                            index,
                            "E0001",
                            format!("unexpected character `{character}`"),
                        ));
                        continue;
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
    Lexed {
        tokens,
        diagnostics,
    }
}

/// Skips a malformed string remainder without consuming the following source line.
fn recover_string(bytes: &[u8], index: &mut usize) {
    while *index < bytes.len() && !matches!(bytes[*index], b'"' | b'\n' | b'\r') {
        *index += 1;
    }
    if bytes.get(*index) == Some(&b'"') {
        *index += 1;
    }
}

/// Reads one double-quoted string literal and decodes its supported escapes.
fn string_literal<'a>(
    source: SourceInput<'a>,
    index: &mut usize,
    start: usize,
) -> Result<TokenKind, CompileDiagnostic<'a>> {
    let bytes = source.text.as_bytes();
    *index += 1;
    let mut value = String::new();
    while *index < bytes.len() {
        match bytes[*index] {
            b'"' => {
                *index += 1;
                return Ok(TokenKind::String(value));
            }
            b'\\' => {
                *index += 1;
                let Some(escape) = bytes.get(*index).copied() else {
                    break;
                };
                *index += 1;
                match escape {
                    b'"' => value.push('"'),
                    b'\\' => value.push('\\'),
                    b'n' => value.push('\n'),
                    b'r' => value.push('\r'),
                    b't' => value.push('\t'),
                    b'0' => value.push('\0'),
                    b'u' => decode_unicode_escape(source, index, start, &mut value)?,
                    _ => {
                        return Err(diagnostic(
                            source,
                            start,
                            *index,
                            "E0006",
                            "invalid string escape",
                        ));
                    }
                }
            }
            b'\n' | b'\r' => {
                return Err(diagnostic(
                    source,
                    start,
                    *index,
                    "E0007",
                    "unterminated string literal",
                ));
            }
            _ => {
                let Some(character) = source.text[*index..].chars().next() else {
                    break;
                };
                value.push(character);
                *index += character.len_utf8();
            }
        }
    }
    Err(diagnostic(
        source,
        start,
        bytes.len(),
        "E0007",
        "unterminated string literal",
    ))
}

/// Decodes a `\\u{HEX}` escape into one Unicode scalar value.
fn decode_unicode_escape<'a>(
    source: SourceInput<'a>,
    index: &mut usize,
    start: usize,
    value: &mut String,
) -> Result<(), CompileDiagnostic<'a>> {
    let bytes = source.text.as_bytes();
    if bytes.get(*index) != Some(&b'{') {
        return Err(diagnostic(
            source,
            start,
            *index,
            "E0006",
            "expected `{` after `\\u`",
        ));
    }
    *index += 1;
    let digits_start = *index;
    while bytes.get(*index).is_some_and(u8::is_ascii_hexdigit) {
        *index += 1;
    }
    if digits_start == *index || bytes.get(*index) != Some(&b'}') {
        return Err(diagnostic(
            source,
            start,
            *index,
            "E0006",
            "invalid Unicode string escape",
        ));
    }
    let digits = &source.text[digits_start..*index];
    *index += 1;
    let scalar = u32::from_str_radix(digits, 16).map_err(|_| {
        diagnostic(
            source,
            start,
            *index,
            "E0006",
            "invalid Unicode string escape",
        )
    })?;
    let Some(character) = char::from_u32(scalar) else {
        return Err(diagnostic(
            source,
            start,
            *index,
            "E0006",
            "Unicode string escape is not a scalar value",
        ));
    };
    value.push(character);
    Ok(())
}

/// Consumes one sequence of decimal digits and separators.
fn consume_digits(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'_') {
        index += 1;
    }
    index
}

/// Validates a nonempty decimal digit sequence with optional separators.
fn valid_digit_segment(segment: &str) -> bool {
    !segment.is_empty()
        && !segment.starts_with('_')
        && !segment.ends_with('_')
        && !segment.contains("__")
}

fn keyword_or_identifier(value: &str) -> TokenKind {
    match value {
        "fn" => TokenKind::Fn,
        "type" => TokenKind::Type,
        "trait" => TokenKind::Trait,
        "impl" => TokenKind::Impl,
        "let" => TokenKind::Let,
        "ret" => TokenKind::Ret,
        "if" => TokenKind::If,
        "else" => TokenKind::Else,
        "while" => TokenKind::While,
        "for" => TokenKind::For,
        "in" => TokenKind::In,
        "break" => TokenKind::Break,
        "continue" => TokenKind::Continue,
        "None" => TokenKind::None,
        "is" => TokenKind::Is,
        "Error" => TokenKind::Error,
        "host" => TokenKind::Host,
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
