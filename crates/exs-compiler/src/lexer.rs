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
    /// A parsed decimal integer magnitude, widened so unary negation can produce `i64::MIN`.
    Integer(i128),
    /// A parsed binary64 floating-point literal.
    Float(f64),
    /// A decoded UTF-8 string literal.
    String(String),
    /// The `fn` keyword.
    Fn,
    /// The `import` keyword.
    Import,
    /// The `use` keyword.
    Use,
    /// The `as` keyword.
    As,
    /// The `type` keyword.
    Type,
    /// The `enum` keyword.
    Enum,
    /// The `match` keyword.
    Match,
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
    /// The parallel task expression keyword.
    Par,
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
    /// `/`.
    Slash,
    /// `!`.
    Bang,
    /// `=`.
    Equal,
    /// `=>`.
    FatArrow,
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
                let value = match numeric.parse::<i128>() {
                    Ok(value) => value,
                    Err(_) => {
                        diagnostics.push(diagnostic(
                            source,
                            start,
                            index,
                            "E0004",
                            "integer literal is outside the supported literal range",
                        ));
                        continue;
                    }
                };
                TokenKind::Integer(value)
            }
        } else if matches!(byte, b'r' | b'd') && bytes.get(index + 1) == Some(&b'#') {
            match prefixed_string_literal(source, &mut index, start, byte == b'd') {
                Ok(token) => token,
                Err(error) => {
                    diagnostics.push(error);
                    continue;
                }
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
                    b'/' => TokenKind::Slash,
                    b'!' if bytes.get(index) == Some(&b'=') => {
                        index += 1;
                        TokenKind::BangEqual
                    }
                    b'!' => TokenKind::Bang,
                    b'=' if bytes.get(index) == Some(&b'>') => {
                        index += 1;
                        TokenKind::FatArrow
                    }
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

/// Reads one hash-delimited raw string and optionally removes its shared content indentation.
fn prefixed_string_literal<'a>(
    source: SourceInput<'a>,
    index: &mut usize,
    start: usize,
    dedent: bool,
) -> Result<TokenKind, CompileDiagnostic<'a>> {
    let bytes = source.text.as_bytes();
    *index += 1;
    let hash_start = *index;
    while bytes.get(*index) == Some(&b'#') {
        *index += 1;
    }
    let hash_count = index.checked_sub(hash_start).unwrap_or(0);
    if hash_count == 0 || bytes.get(*index) != Some(&b'"') {
        return Err(diagnostic(
            source,
            start,
            *index,
            "E0008",
            "invalid raw string delimiter",
        ));
    }
    *index += 1;
    let value_start = *index;
    while *index < bytes.len() {
        if bytes[*index] == b'"'
            && bytes
                .get(*index + 1..*index + 1 + hash_count)
                .is_some_and(|suffix| suffix.iter().all(|byte| *byte == b'#'))
        {
            let value = &source.text[value_start..*index];
            *index += 1 + hash_count;
            return Ok(TokenKind::String(if dedent {
                dedent_string(value)
            } else {
                value.to_owned()
            }));
        }
        *index += 1;
    }
    Err(diagnostic(
        source,
        start,
        bytes.len(),
        "E0007",
        "unterminated raw string literal",
    ))
}

/// Removes delimiter-only outer lines and common indentation from one dedented raw literal.
fn dedent_string(value: &str) -> String {
    let value = remove_outer_delimiter_lines(value);
    let indent = common_indent(value);
    if indent.is_empty() {
        return value.to_owned();
    }
    let mut dedented = String::with_capacity(value.len());
    for line in value.split_inclusive('\n') {
        let (content, ending) = line
            .strip_suffix('\n')
            .map_or((line, ""), |content| (content, "\n"));
        if content.trim_matches([' ', '\t', '\r']).is_empty() {
            dedented.push_str(ending);
        } else {
            dedented.push_str(content.strip_prefix(indent).unwrap_or(content));
            dedented.push_str(ending);
        }
    }
    dedented
}

/// Removes the whitespace-only source lines containing a dedented literal's delimiters.
fn remove_outer_delimiter_lines(value: &str) -> &str {
    let value = value
        .find('\n')
        .filter(|newline| value[..*newline].trim_matches([' ', '\t', '\r']).is_empty())
        .map_or(value, |newline| &value[newline + 1..]);
    value.rfind('\n').map_or(value, |newline| {
        if value[newline + 1..]
            .trim_matches([' ', '\t', '\r'])
            .is_empty()
        {
            &value[..newline]
        } else {
            value
        }
    })
}

/// Finds the shared spaces-and-tabs prefix used by every nonblank dedented content line.
fn common_indent(value: &str) -> &str {
    let mut common = None::<&str>;
    for line in value.split('\n') {
        let content = line.strip_suffix('\r').unwrap_or(line);
        if content.trim_matches([' ', '\t']).is_empty() {
            continue;
        }
        let indent_end = content
            .bytes()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        let indent = &content[..indent_end];
        common = Some(common.map_or(indent, |current| shared_prefix(current, indent)));
    }
    common.unwrap_or("")
}

/// Returns the common byte prefix of two ASCII indentation strings.
fn shared_prefix<'a>(first: &'a str, second: &str) -> &'a str {
    let shared = first
        .bytes()
        .zip(second.bytes())
        .take_while(|(first, second)| first == second)
        .count();
    &first[..shared]
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
        "import" => TokenKind::Import,
        "use" => TokenKind::Use,
        "as" => TokenKind::As,
        "type" => TokenKind::Type,
        "enum" => TokenKind::Enum,
        "match" => TokenKind::Match,
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
        "par" => TokenKind::Par,
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
