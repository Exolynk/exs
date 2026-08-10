//! Source comments and empty lines retained by the canonical formatter.

/// One non-semantic source fragment retained while formatting parsed syntax.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Trivia {
    /// Inclusive byte offset of this fragment.
    pub(super) start: usize,
    /// Fragment category and source text where applicable.
    pub(super) kind: TriviaKind,
}

/// Kinds of retained non-semantic source fragments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum TriviaKind {
    /// A line or block comment, including its delimiters.
    Comment(String),
    /// A documentation comment that belongs directly above a declaration.
    DocumentationComment(String),
    /// One line containing no source other than whitespace.
    BlankLine,
}

/// Collects comments and blank source lines while excluding quoted string contents.
pub(super) fn collect(source: &str) -> Vec<Trivia> {
    let bytes = source.as_bytes();
    let mut protected = vec![false; bytes.len()];
    let mut trivia = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if source[index..].starts_with("//") {
            let start = index;
            index = source[index..]
                .find('\n')
                .map_or(source.len(), |offset| index + offset);
            protect(&mut protected, start, index);
            trivia.push(Trivia {
                start,
                kind: if source[start..].starts_with("///") {
                    TriviaKind::DocumentationComment(source[start..index].to_owned())
                } else {
                    TriviaKind::Comment(source[start..index].to_owned())
                },
            });
        } else if source[index..].starts_with("/*") {
            let start = index;
            index = source[index + 2..]
                .find("*/")
                .map_or(source.len(), |offset| index + 2 + offset + 2);
            protect(&mut protected, start, index);
            trivia.push(Trivia {
                start,
                kind: TriviaKind::Comment(source[start..index].to_owned()),
            });
        } else if bytes[index] == b'"' {
            let start = index;
            index = skip_quoted_string(source, index, 0);
            protect(&mut protected, start, index);
        } else if bytes[index] == b'#' {
            let hashes = source[index..]
                .bytes()
                .take_while(|byte| *byte == b'#')
                .count();
            if source[index + hashes..].starts_with('"') {
                let start = index;
                index = skip_quoted_string(source, index + hashes, hashes);
                protect(&mut protected, start, index);
            } else {
                index += 1;
            }
        } else {
            index += 1;
        }
    }
    let mut line_start = 0;
    while line_start < source.len() {
        let line_end = source[line_start..]
            .find('\n')
            .map_or(source.len(), |offset| line_start + offset);
        if source[line_start..line_end]
            .bytes()
            .all(|byte| byte.is_ascii_whitespace())
            && !protected[line_start..line_end]
                .iter()
                .any(|protected| *protected)
        {
            trivia.push(Trivia {
                start: line_start,
                kind: TriviaKind::BlankLine,
            });
        }
        if line_end == source.len() {
            break;
        }
        line_start = line_end + 1;
    }
    trivia.sort_by_key(|item| item.start);
    trivia
}

/// Marks one byte range as source content that cannot contain formatting trivia.
fn protect(protected: &mut [bool], start: usize, end: usize) {
    for byte in &mut protected[start..end] {
        *byte = true;
    }
}

/// Skips one quoted or hash-delimited ExS string without interpreting its contents.
fn skip_quoted_string(source: &str, quote_start: usize, hashes: usize) -> usize {
    let mut index = quote_start + 1;
    let bytes = source.as_bytes();
    while index < bytes.len() {
        if hashes == 0 && bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
        } else if bytes[index] == b'"' && source[index + 1..].starts_with(&"#".repeat(hashes)) {
            return (index + 1 + hashes).min(source.len());
        } else {
            index += 1;
        }
    }
    source.len()
}

/// Verifies comments and empty lines are not discovered inside string literals.
#[cfg(test)]
mod tests {
    use super::{TriviaKind, collect};

    /// Retains source comments and genuine empty lines while skipping string contents.
    #[test]
    fn collects_comments_and_blank_lines_outside_strings() {
        let trivia = collect("// leading\n\nlet value = \"// text\"; /* tail */\n");
        assert!(trivia.iter().any(
            |item| matches!(item.kind, TriviaKind::Comment(ref text) if text == "// leading")
        ));
        assert!(trivia.iter().any(
            |item| matches!(item.kind, TriviaKind::Comment(ref text) if text == "/* tail */")
        ));
        assert_eq!(
            trivia
                .iter()
                .filter(|item| matches!(item.kind, TriviaKind::BlankLine))
                .count(),
            1
        );
    }

    /// Distinguishes declaration documentation from ordinary line comments.
    #[test]
    fn collects_documentation_comments() {
        assert!(matches!(
            collect("/// Documents a function.\nfn main() {}")
                .first()
                .map(|item| &item.kind),
            Some(TriviaKind::DocumentationComment(text)) if text == "/// Documents a function."
        ));
    }

    /// Ignores compact source that contains no comment or empty-line trivia.
    #[test]
    fn ignores_compact_source() {
        assert!(collect("fn main(){ret 1;}").is_empty());
    }
}
