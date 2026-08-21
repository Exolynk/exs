use std::collections::BTreeMap;

use exs_compiler::{SourceInput, SourceToken, source_lex};

/// One compiler-produced token used by tolerant completion analysis.
pub(crate) type Token = SourceToken;

/// Source declarations and enum variants discoverable without a complete parse.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct DocumentSymbols {
    /// Top-level declarations keyed by their exact source spelling.
    pub(crate) declarations: BTreeMap<String, SymbolKind>,
    /// Function signatures keyed by their declared source names.
    pub(crate) functions: BTreeMap<String, FunctionSignature>,
    /// Variants keyed by their enclosing enum name.
    pub(crate) variants: BTreeMap<String, Vec<String>>,
}

/// One source function signature available to a completion client.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct FunctionSignature {
    /// Parameters in declaration order.
    pub(crate) parameters: Vec<FunctionParameter>,
}

/// One named source parameter with its optional type annotation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FunctionParameter {
    /// Parameter name as written in ExS source.
    pub(crate) name: String,
    /// Optional source type annotation rendered for completion detail.
    pub(crate) type_annotation: Option<String>,
}

/// Kinds of top-level declaration recognized by completion analysis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SymbolKind {
    /// A top-level direct function.
    Function,
    /// A nominal object type.
    Type,
    /// A tagged union declaration.
    Enum,
    /// A trait declaration.
    Trait,
}

/// The editable declaration position inside an incomplete function header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FunctionHeaderContext {
    /// A parameter identifier is being declared.
    ParameterName,
    /// A parameter type annotation is being written.
    ParameterType,
}

/// Returns compiler-produced tokens for tolerant completion analysis.
pub(crate) fn tokenize(source: &str) -> Vec<Token> {
    source_lex(SourceInput {
        source_id: "<autocomplete>",
        text: source,
    })
    .tokens
}

/// Collects declarations from a best-effort token stream, including incomplete documents.
pub(crate) fn document_symbols(tokens: &[Token]) -> DocumentSymbols {
    let mut symbols = DocumentSymbols::default();
    let mut depth = 0usize;
    let mut index = 0usize;
    while index < tokens.len() {
        let token = &tokens[index];
        match token.text.as_str() {
            "{" => depth += 1,
            "}" => depth = depth.saturating_sub(1),
            "fn" | "type" | "enum" | "trait" if depth == 0 => {
                let Some(name) = next_identifier(tokens, index + 1) else {
                    index += 1;
                    continue;
                };
                let kind = match token.text.as_str() {
                    "fn" => SymbolKind::Function,
                    "type" => SymbolKind::Type,
                    "enum" => SymbolKind::Enum,
                    "trait" => SymbolKind::Trait,
                    _ => unreachable!(),
                };
                symbols.declarations.insert(name.text.clone(), kind);
                if kind == SymbolKind::Function
                    && let Some(signature) = function_signature(tokens, index)
                {
                    symbols.functions.insert(name.text.clone(), signature);
                }
                if kind == SymbolKind::Enum {
                    symbols
                        .variants
                        .insert(name.text.clone(), enum_variants(tokens, index));
                }
            }
            _ => {}
        }
        index += 1;
    }
    symbols
}

/// Returns lexical bindings visible at the end of the provided token stream.
pub(crate) fn visible_bindings(tokens: &[Token]) -> Vec<String> {
    let mut scopes = vec![Vec::<String>::new()];
    let mut scope_bindings = BTreeMap::<usize, Vec<String>>::new();
    let mut index = 0usize;
    while index < tokens.len() {
        match tokens[index].text.as_str() {
            "fn" => {
                if let Some((body, parameters)) = function_scope(tokens, index) {
                    scope_bindings.insert(
                        body,
                        parameters
                            .into_iter()
                            .map(|parameter| parameter.name)
                            .collect(),
                    );
                }
            }
            "for" => {
                if let Some((body, binding)) = for_scope(tokens, index) {
                    scope_bindings.insert(body, vec![binding]);
                }
            }
            "let" => {
                if let Some(binding) = next_identifier(tokens, index + 1)
                    && let Some(scope) = scopes.last_mut()
                {
                    scope.push(binding.text.clone());
                }
            }
            "{" => scopes.push(scope_bindings.remove(&index).unwrap_or_default()),
            "}" if scopes.len() > 1 => {
                let _closed_scope = scopes.pop();
            }
            _ => {}
        }
        index += 1;
    }
    scopes.into_iter().flatten().collect()
}

/// Returns known type annotations for lexical bindings visible at the end of the token stream.
pub(crate) fn visible_binding_types(tokens: &[Token]) -> BTreeMap<String, String> {
    let mut scopes = vec![BTreeMap::<String, String>::new()];
    let mut scope_bindings = BTreeMap::<usize, BTreeMap<String, String>>::new();
    let mut index = 0usize;
    while index < tokens.len() {
        match tokens[index].text.as_str() {
            "fn" => {
                if let Some((body, parameters)) = function_scope(tokens, index) {
                    let bindings = parameters
                        .into_iter()
                        .filter_map(|parameter| {
                            parameter
                                .type_annotation
                                .map(|type_annotation| (parameter.name, type_annotation))
                        })
                        .collect();
                    scope_bindings.insert(body, bindings);
                }
            }
            "{" => scopes.push(scope_bindings.remove(&index).unwrap_or_default()),
            "}" if scopes.len() > 1 => {
                let _closed_scope = scopes.pop();
            }
            _ => {}
        }
        index += 1;
    }
    let mut bindings = BTreeMap::new();
    for scope in scopes {
        bindings.extend(scope);
    }
    bindings
}

/// Returns the identifier prefix immediately preceding a valid source offset.
pub(crate) fn identifier_prefix(source: &str, cursor: usize) -> (usize, &str) {
    let cursor = clamp_offset(source, cursor);
    let mut start = cursor;
    while start > 0
        && (source.as_bytes()[start - 1].is_ascii_alphanumeric()
            || source.as_bytes()[start - 1] == b'_')
    {
        start -= 1;
    }
    (start, &source[start..cursor])
}

/// Returns the identifier immediately before a namespace separator.
pub(crate) fn namespace_receiver(source: &str, before_prefix: usize) -> Option<&str> {
    let before = &source[..before_prefix];
    let separator = before.trim_end().strip_suffix("::")?;
    let end = separator.len();
    let (start, receiver) = identifier_prefix(separator, end);
    if start == end || receiver.is_empty() {
        None
    } else {
        Some(receiver)
    }
}

/// Returns whether the source immediately before the prefix selects the Host boundary.
pub(crate) fn is_host_member_context(source: &str, before_prefix: usize) -> bool {
    source[..before_prefix].trim_end().ends_with("Host::")
}

/// Returns the identifier immediately before an instance member separator.
pub(crate) fn member_receiver(source: &str, before_prefix: usize) -> Option<&str> {
    let before = source[..before_prefix].trim_end();
    let receiver = before.strip_suffix('.')?;
    let end = receiver.len();
    let (start, receiver) = identifier_prefix(receiver, end);
    if start == end || receiver.is_empty() {
        None
    } else {
        Some(receiver)
    }
}

/// Returns whether the prefix follows a type annotation delimiter.
pub(crate) fn is_type_context(source: &str, before_prefix: usize) -> bool {
    let before = source[..before_prefix].trim_end();
    before.ends_with(':') || before.ends_with("->") || before.ends_with('|')
}

/// Returns whether the prefix is the name being declared after an ExS `fn` keyword.
pub(crate) fn is_function_name_declaration_context(source: &str, before_prefix: usize) -> bool {
    source[..before_prefix].trim_end().ends_with("fn")
}

/// Returns the active context inside the parameter list of an incomplete function header.
pub(crate) fn function_header_context(
    tokens: &[Token],
    before_prefix: usize,
) -> Option<FunctionHeaderContext> {
    let tokens = tokens
        .iter()
        .take_while(|token| token.end <= before_prefix)
        .collect::<Vec<_>>();
    let mut nested_parentheses = 0usize;
    let mut open = None;
    for (index, token) in tokens.iter().enumerate().rev() {
        match token.text.as_str() {
            ")" => nested_parentheses += 1,
            "(" if nested_parentheses == 0 => {
                open = Some(index);
                break;
            }
            "(" => nested_parentheses -= 1,
            _ => {}
        }
    }
    let open = open?;
    let function_name = tokens.get(open.checked_sub(1)?)?;
    let function_keyword = tokens.get(open.checked_sub(2)?)?;
    if function_keyword.text != "fn" || !token_is_identifier(function_name) {
        return None;
    }
    let last_separator = tokens[open + 1..]
        .iter()
        .rposition(|token| token.text == ",")
        .map_or(open, |offset| open + offset + 1);
    let has_type_delimiter = tokens[last_separator + 1..]
        .iter()
        .any(|token| token.text == ":");
    Some(if has_type_delimiter {
        FunctionHeaderContext::ParameterType
    } else {
        FunctionHeaderContext::ParameterName
    })
}

/// Finds the known function and argument index at the active argument position.
pub(crate) fn call_argument_context(
    tokens: &[Token],
    cursor: usize,
    symbols: &DocumentSymbols,
) -> Option<(FunctionSignature, usize)> {
    let tokens = tokens
        .iter()
        .take_while(|token| token.end <= cursor)
        .collect::<Vec<_>>();
    let mut nested_parentheses = 0usize;
    let mut open = None;
    for (index, token) in tokens.iter().enumerate().rev() {
        match token.text.as_str() {
            ")" => nested_parentheses += 1,
            "(" if nested_parentheses == 0 => {
                open = Some(index);
                break;
            }
            "(" => nested_parentheses -= 1,
            _ => {}
        }
    }
    let open = open?;
    let function = tokens.get(open.checked_sub(1)?)?;
    if !token_is_identifier(function) {
        return None;
    }
    let signature = symbols.functions.get(&function.text)?.clone();
    let mut nested_parentheses = 0usize;
    let mut argument_index = 0usize;
    for token in &tokens[open + 1..] {
        match token.text.as_str() {
            "(" => nested_parentheses += 1,
            ")" => nested_parentheses = nested_parentheses.saturating_sub(1),
            "," if nested_parentheses == 0 => argument_index += 1,
            _ => {}
        }
    }
    (argument_index < signature.parameters.len()).then_some((signature, argument_index))
}

/// Returns whether one byte can begin an ExS identifier.
fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

/// Returns the next identifier token at or after one token index.
fn next_identifier(tokens: &[Token], start: usize) -> Option<&Token> {
    tokens.get(start..)?.iter().find(|token| {
        token
            .text
            .as_bytes()
            .first()
            .is_some_and(|byte| is_identifier_start(*byte))
    })
}

/// Extracts direct enum variants from the body associated with one enum declaration.
fn enum_variants(tokens: &[Token], enum_index: usize) -> Vec<String> {
    let Some(open) = tokens[enum_index..]
        .iter()
        .position(|token| token.text == "{")
        .map(|offset| enum_index + offset)
    else {
        return Vec::new();
    };
    let mut variants = Vec::new();
    let mut depth = 0usize;
    let mut expect_variant = true;
    for token in &tokens[open + 1..] {
        match token.text.as_str() {
            "{" | "(" | "[" => depth += 1,
            "}" if depth == 0 => break,
            "}" | ")" | "]" => depth = depth.saturating_sub(1),
            "," if depth == 0 => expect_variant = true,
            _ if depth == 0 && expect_variant && token_is_identifier(token) => {
                variants.push(token.text.clone());
                expect_variant = false;
            }
            _ => {}
        }
    }
    variants
}

/// Finds a function body and the parameters that become visible inside it.
fn function_scope(
    tokens: &[Token],
    function_index: usize,
) -> Option<(usize, Vec<FunctionParameter>)> {
    let signature = function_signature(tokens, function_index)?;
    let open = tokens[function_index..]
        .iter()
        .position(|token| token.text == "(")
        .map(|offset| function_index + offset)?;
    let mut close = None;
    for (index, token) in tokens.iter().enumerate().skip(open + 1) {
        if token.text == ")" {
            close = Some(index);
            break;
        }
    }
    let close = close?;
    let body = tokens[close + 1..]
        .iter()
        .position(|token| token.text == "{" || token.text == ";")
        .map(|offset| close + 1 + offset)?;
    (tokens[body].text == "{").then_some((body, signature.parameters))
}

/// Extracts one function's parameter names and optional type annotations.
fn function_signature(tokens: &[Token], function_index: usize) -> Option<FunctionSignature> {
    let open = tokens[function_index..]
        .iter()
        .position(|token| token.text == "(")
        .map(|offset| function_index + offset)?;
    let mut parameters = Vec::new();
    let mut index = open + 1;
    while index < tokens.len() && tokens[index].text != ")" {
        if tokens[index].text == "," {
            index += 1;
            continue;
        }
        if !token_is_identifier(&tokens[index]) {
            index += 1;
            continue;
        }
        let name = tokens[index].text.clone();
        index += 1;
        let type_annotation = if tokens.get(index).is_some_and(|token| token.text == ":") {
            index += 1;
            let start = index;
            while index < tokens.len() && tokens[index].text != "," && tokens[index].text != ")" {
                index += 1;
            }
            format_type_annotation(&tokens[start..index])
        } else {
            None
        };
        parameters.push(FunctionParameter {
            name,
            type_annotation,
        });
    }
    Some(FunctionSignature { parameters })
}

/// Formats source tokens as a concise union type annotation.
fn format_type_annotation(tokens: &[Token]) -> Option<String> {
    if tokens.is_empty() {
        return None;
    }
    Some(
        tokens
            .iter()
            .map(|token| token.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Finds a `for` body and its binding name.
fn for_scope(tokens: &[Token], for_index: usize) -> Option<(usize, String)> {
    let binding = next_identifier(tokens, for_index + 1)?.text.clone();
    let body = tokens[for_index + 1..]
        .iter()
        .position(|token| token.text == "{" || token.text == ";")
        .map(|offset| for_index + 1 + offset)?;
    (tokens[body].text == "{").then_some((body, binding))
}

/// Returns whether a token has identifier spelling.
fn token_is_identifier(token: &Token) -> bool {
    token
        .text
        .as_bytes()
        .first()
        .is_some_and(|byte| is_identifier_start(*byte))
}

/// Clamps an arbitrary offset to a valid UTF-8 character boundary.
fn clamp_offset(source: &str, offset: usize) -> usize {
    let mut offset = offset.min(source.len());
    while offset > 0 && !source.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}
