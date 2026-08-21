use std::collections::BTreeSet;

use exs_compiler::{
    SourceInput, source_lex, standard_library_enums, standard_library_namespace,
    standard_library_types,
};

use crate::catalog::{
    append_keywords, append_standard_functions, append_standard_namespace_functions,
    append_standard_namespaces, append_standard_symbols, push_if_matching,
};
use crate::syntax::{
    FunctionHeaderContext, FunctionParameter, FunctionSignature, SymbolKind, call_argument_context,
    document_symbols, function_header_context, identifier_prefix,
    is_function_name_declaration_context, is_host_member_context, is_type_context, member_receiver,
    namespace_receiver, visible_binding_types, visible_bindings,
};
use crate::{CompletionItem, CompletionKind, CompletionRequest, CompletionResponse};

/// Stateless completion engine for one ExS source document.
#[derive(Clone, Copy, Debug, Default)]
pub struct CompletionEngine;

impl CompletionEngine {
    /// Produces context-sensitive ExS completions for one document position.
    #[must_use]
    pub fn complete(&self, request: CompletionRequest<'_>) -> CompletionResponse {
        let lexed = source_lex(SourceInput {
            source_id: "<autocomplete>",
            text: request.source,
        });
        if lexed.is_comment_position(request.cursor) {
            return CompletionResponse::default();
        }
        let (prefix_start, prefix) = identifier_prefix(request.source, request.cursor);
        let tokens = lexed.tokens;
        let symbols = document_symbols(&tokens);
        if is_function_name_declaration_context(request.source, prefix_start) {
            return CompletionResponse::default();
        }
        match function_header_context(&tokens, prefix_start) {
            Some(FunctionHeaderContext::ParameterName) => return CompletionResponse::default(),
            Some(FunctionHeaderContext::ParameterType) => {
                let mut items = Vec::new();
                append_standard_symbols(&mut items, prefix);
                append_document_types(&mut items, prefix, &symbols);
                return unique_response(items, prefix_start, request.cursor);
            }
            None => {}
        }
        if let Some((signature, argument_index)) =
            call_argument_context(&tokens, request.cursor, &symbols)
        {
            let bindings = tokens
                .iter()
                .filter(|token| token.end <= request.cursor)
                .cloned()
                .collect::<Vec<_>>();
            return response_for_call_argument(
                prefix_start,
                request.cursor,
                prefix,
                signature,
                argument_index,
                &visible_bindings(&bindings),
            );
        }
        if prefix.is_empty() {
            return CompletionResponse::default();
        }
        let tokens_before_cursor = tokens
            .iter()
            .filter(|token| token.end <= prefix_start)
            .cloned()
            .collect::<Vec<_>>();

        if is_host_member_context(request.source, prefix_start) {
            return response_for_host_member(prefix, prefix_start, request.cursor);
        }
        if let Some(receiver) = member_receiver(request.source, prefix_start) {
            return response_for_member(
                prefix,
                prefix_start,
                request.cursor,
                receiver,
                &visible_binding_types(&tokens_before_cursor),
            );
        }
        if let Some(receiver) = namespace_receiver(request.source, prefix_start) {
            return response_for_namespace(prefix, prefix_start, request.cursor, &tokens, receiver);
        }

        let mut items = Vec::new();
        if is_type_context(request.source, prefix_start) {
            append_standard_symbols(&mut items, prefix);
            append_document_types(&mut items, prefix, &symbols);
        } else {
            append_keywords(&mut items, prefix);
            append_standard_functions(&mut items, prefix);
            append_standard_symbols(&mut items, prefix);
            append_standard_namespaces(&mut items, prefix);
            append_document_symbols(&mut items, prefix, &symbols);
            append_visible_bindings(&mut items, prefix, &visible_bindings(&tokens_before_cursor));
        }
        unique_response(items, prefix_start, request.cursor)
    }
}

/// Returns documented runtime methods for one statically annotated lexical receiver.
fn response_for_member(
    prefix: &str,
    prefix_start: usize,
    cursor: usize,
    receiver: &str,
    binding_types: &std::collections::BTreeMap<String, String>,
) -> CompletionResponse {
    let Some(type_name) = binding_types.get(receiver) else {
        return CompletionResponse::default();
    };
    let Some(type_info) = standard_library_types()
        .into_iter()
        .find(|type_info| type_info.name == type_name)
    else {
        return CompletionResponse::default();
    };
    let mut items = Vec::new();
    for method in type_info.methods {
        append_method_completion(&mut items, prefix, method.signature);
    }
    let clone_signature = format!("clone() -> {} | Error", type_info.name);
    append_method_completion(&mut items, prefix, &clone_signature);
    unique_response(items, prefix_start, cursor)
}

/// Appends one documented method call with parentheses and a suitable caret position.
fn append_method_completion(items: &mut Vec<CompletionItem>, prefix: &str, signature: &str) {
    let Some((name, arguments)) = signature.split_once('(') else {
        return;
    };
    let insert_text = format!("{name}()");
    let cursor = (!arguments.starts_with(')')).then_some(name.len() + 1);
    push_if_matching(
        items,
        prefix,
        name,
        Some(signature),
        &insert_text,
        cursor,
        CompletionKind::Function,
    );
}

/// Returns visible source bindings for one known function argument position.
fn response_for_call_argument(
    prefix_start: usize,
    cursor: usize,
    prefix: &str,
    signature: FunctionSignature,
    argument_index: usize,
    bindings: &[String],
) -> CompletionResponse {
    let parameter = &signature.parameters[argument_index];
    let mut items = Vec::new();
    for binding in bindings.iter().rev() {
        if binding == prefix {
            continue;
        }
        push_if_matching(
            &mut items,
            prefix,
            binding,
            Some(&argument_detail(parameter)),
            binding,
            None,
            CompletionKind::Variable,
        );
    }
    unique_response(items, prefix_start, cursor)
}

/// Returns a completion response for Host namespace operations.
fn response_for_host_member(
    prefix: &str,
    prefix_start: usize,
    cursor: usize,
) -> CompletionResponse {
    let mut items = Vec::new();
    let Some(namespace) = standard_library_namespace("Host") else {
        return CompletionResponse::default();
    };
    for function in namespace.functions {
        let (insert_text, cursor_offset) = if function.name == "call" {
            ("call(\"name\")".to_owned(), Some(6))
        } else {
            (
                format!("{}()", function.name),
                Some(function.name.len() + 1),
            )
        };
        push_if_matching(
            &mut items,
            prefix,
            function.name,
            Some(function.signature),
            &insert_text,
            cursor_offset,
            CompletionKind::HostMember,
        );
    }
    unique_response(items, prefix_start, cursor)
}

/// Returns enum variants available after a source namespace separator.
fn response_for_namespace(
    prefix: &str,
    prefix_start: usize,
    cursor: usize,
    tokens: &[crate::syntax::Token],
    receiver: &str,
) -> CompletionResponse {
    let mut items = Vec::new();
    if receiver == "std" {
        append_standard_symbols(&mut items, prefix);
    }
    append_standard_namespace_functions(&mut items, prefix, receiver);
    if let Some(enum_info) = standard_library_enums()
        .into_iter()
        .find(|enum_info| enum_info.name == receiver)
    {
        append_variants(&mut items, prefix, enum_info.variants.iter().copied());
    }
    if let Some(variants) = document_symbols(tokens).variants.get(receiver) {
        append_variants(&mut items, prefix, variants.iter().map(String::as_str));
    }
    unique_response(items, prefix_start, cursor)
}

/// Appends source declarations that are valid in a type annotation context.
fn append_document_types(
    items: &mut Vec<CompletionItem>,
    prefix: &str,
    symbols: &crate::syntax::DocumentSymbols,
) {
    for (name, kind) in &symbols.declarations {
        let (detail, completion_kind) = match kind {
            SymbolKind::Type => ("Source type", CompletionKind::Type),
            SymbolKind::Enum => ("Source enum", CompletionKind::Enum),
            SymbolKind::Trait => ("Source trait", CompletionKind::Trait),
            SymbolKind::Function => continue,
        };
        push_if_matching(
            items,
            prefix,
            name,
            Some(detail),
            name,
            None,
            completion_kind,
        );
    }
}

/// Appends all top-level source declarations in deterministic source-name order.
fn append_document_symbols(
    items: &mut Vec<CompletionItem>,
    prefix: &str,
    symbols: &crate::syntax::DocumentSymbols,
) {
    for (name, kind) in &symbols.declarations {
        let (detail, completion_kind) = match kind {
            SymbolKind::Function => {
                let Some(signature) = symbols.functions.get(name) else {
                    continue;
                };
                append_function_completion(items, prefix, name, signature);
                continue;
            }
            SymbolKind::Type => ("Source type", CompletionKind::Type),
            SymbolKind::Enum => ("Source enum", CompletionKind::Enum),
            SymbolKind::Trait => ("Source trait", CompletionKind::Trait),
        };
        push_if_matching(
            items,
            prefix,
            name,
            Some(detail),
            name,
            None,
            completion_kind,
        );
    }
}

/// Appends one function call edit with parentheses and an appropriate caret position.
fn append_function_completion(
    items: &mut Vec<CompletionItem>,
    prefix: &str,
    name: &str,
    signature: &FunctionSignature,
) {
    let insert_text = format!("{name}()");
    let cursor = if signature.parameters.is_empty() {
        Some(insert_text.len())
    } else {
        Some(name.len() + 1)
    };
    push_if_matching(
        items,
        prefix,
        name,
        Some(&function_detail(name, signature)),
        &insert_text,
        cursor,
        CompletionKind::Function,
    );
}

/// Renders a concise function signature for the completion detail field.
fn function_detail(name: &str, signature: &FunctionSignature) -> String {
    let parameters = signature
        .parameters
        .iter()
        .map(|parameter| match &parameter.type_annotation {
            Some(type_annotation) => format!("{}: {type_annotation}", parameter.name),
            None => parameter.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("fn {name}({parameters})")
}

/// Renders the argument currently expected by a known function call.
fn argument_detail(parameter: &FunctionParameter) -> String {
    match &parameter.type_annotation {
        Some(type_annotation) => format!("For parameter {}: {type_annotation}", parameter.name),
        None => format!("For parameter {}", parameter.name),
    }
}

/// Appends visible lexical bindings while preserving nearest-scope precedence.
fn append_visible_bindings(items: &mut Vec<CompletionItem>, prefix: &str, bindings: &[String]) {
    for binding in bindings.iter().rev() {
        push_if_matching(
            items,
            prefix,
            binding,
            Some("Local binding"),
            binding,
            None,
            CompletionKind::Variable,
        );
    }
}

/// Appends enum variants provided by one iterator of source names.
fn append_variants<'a>(
    items: &mut Vec<CompletionItem>,
    prefix: &str,
    variants: impl IntoIterator<Item = &'a str>,
) {
    for variant in variants {
        push_if_matching(
            items,
            prefix,
            variant,
            Some("Enum variant"),
            variant,
            None,
            CompletionKind::Variant,
        );
    }
}

/// Removes duplicate labels and attaches the identifier replacement range.
fn unique_response(
    items: Vec<CompletionItem>,
    prefix_start: usize,
    cursor: usize,
) -> CompletionResponse {
    let mut labels = BTreeSet::new();
    let items = items
        .into_iter()
        .filter(|item| labels.insert(item.label.clone()))
        .take(24)
        .collect();
    CompletionResponse {
        items,
        replace: Some(prefix_start..cursor),
    }
}

/// Verifies context selection, lexical scope collection, and source enum variants.
#[cfg(test)]
mod tests {
    use super::CompletionEngine;
    use crate::{CompletionKind, CompletionRequest};

    /// Returns the completion item with the requested source label.
    fn item<'a>(response: &'a crate::CompletionResponse, label: &str) -> &'a crate::CompletionItem {
        response
            .items
            .iter()
            .find(|item| item.label == label)
            .unwrap_or_else(|| panic!("missing completion item `{label}`"))
    }

    /// Completes a lexical binding introduced in the active function body.
    #[test]
    fn completes_visible_local_bindings() {
        let source = "fn main(value: Int) {\n    let total = value;\n    tot\n}";
        let response = CompletionEngine.complete(CompletionRequest {
            source,
            cursor: source.find("tot\n").unwrap_or_default() + 3,
        });
        assert_eq!(response.replace, Some(49..52));
        assert_eq!(item(&response, "total").kind, CompletionKind::Variable);
    }

    /// Completes variants declared by an enum even when the surrounding script is incomplete.
    #[test]
    fn completes_user_enum_variants() {
        let source = "enum Choice { First, Second }\nfn main() { Choice::Fi }";
        let response = CompletionEngine.complete(CompletionRequest {
            source,
            cursor: source.len() - 2,
        });
        assert_eq!(item(&response, "First").kind, CompletionKind::Variant);
    }

    /// Completes the only available Host boundary operation after `Host::`.
    #[test]
    fn completes_host_call() {
        let source = "fn main() { Host::ca }";
        let response = CompletionEngine.complete(CompletionRequest {
            source,
            cursor: source.len() - 2,
        });
        let completion = item(&response, "call");
        assert_eq!(completion.insert_text, "call(\"name\")");
        assert_eq!(completion.kind, CompletionKind::HostMember);
    }

    /// Completes the standard Duration factory after a namespace separator.
    #[test]
    fn completes_duration_factories() {
        let source = "fn main() { Duration::mil }";
        let response = CompletionEngine.complete(CompletionRequest {
            source,
            cursor: source.len() - 2,
        });
        let completion = item(&response, "milliseconds");
        assert_eq!(completion.insert_text, "milliseconds()");
    }

    /// Completes the built-in Host sleep operation after a namespace separator.
    #[test]
    fn completes_host_sleep() {
        let source = "fn main() { Host::sl }";
        let response = CompletionEngine.complete(CompletionRequest {
            source,
            cursor: source.len() - 2,
        });
        let completion = item(&response, "sleep");
        assert_eq!(completion.insert_text, "sleep()");
    }

    /// Completes the built-in Host namespace in expression context.
    #[test]
    fn completes_host_namespace() {
        let source = "fn main() { Hos }";
        let response = CompletionEngine.complete(CompletionRequest {
            source,
            cursor: source.len() - 2,
        });
        assert_eq!(item(&response, "Host").kind, CompletionKind::Type);
    }

    /// Inserts required whitespace after accepting a local binding declaration keyword.
    #[test]
    fn completes_let_with_trailing_whitespace() {
        let source = "fn main() { le }";
        let response = CompletionEngine.complete(CompletionRequest {
            source,
            cursor: source.len() - 2,
        });
        assert_eq!(item(&response, "let").insert_text, "let ");
    }

    /// Inserts parentheses and places the caret inside a function call with parameters.
    #[test]
    fn completes_parameterized_function_calls() {
        let source = "fn greet(name: String) {}\nfn main() { gre }";
        let response = CompletionEngine.complete(CompletionRequest {
            source,
            cursor: source.len() - 2,
        });
        let completion = item(&response, "greet");
        assert_eq!(completion.insert_text, "greet()");
        assert_eq!(completion.cursor, Some(6));
        assert_eq!(completion.detail.as_deref(), Some("fn greet(name: String)"));
    }

    /// Offers visible bindings for the first parameter immediately inside a known call.
    #[test]
    fn completes_first_function_argument() {
        let source = "fn greet(name: String) {}\nfn main(value: String) { greet() }";
        let cursor = source.rfind("greet()").unwrap_or_default() + 6;
        let response = CompletionEngine.complete(CompletionRequest { source, cursor });
        let completion = item(&response, "value");
        assert_eq!(completion.kind, CompletionKind::Variable);
        assert_eq!(
            completion.detail.as_deref(),
            Some("For parameter name: String")
        );
    }

    /// Closes the call-argument popup after a selected binding exactly fills its prefix.
    #[test]
    fn closes_after_selecting_an_exact_call_argument_binding() {
        let source = "fn greet(name: String) {}\nfn main(value: String) { greet(value) }";
        let cursor = source.rfind("value)").unwrap_or_default() + 5;
        let response = CompletionEngine.complete(CompletionRequest { source, cursor });
        assert!(response.items.is_empty());
    }

    /// Avoids turning a function declaration name into an invocation completion.
    #[test]
    fn suppresses_function_call_completion_while_declaring_a_function() {
        let source = "fn gre";
        let response = CompletionEngine.complete(CompletionRequest {
            source,
            cursor: source.len(),
        });
        assert!(response.items.is_empty());
    }

    /// Avoids offering expression names while a function parameter name is declared.
    #[test]
    fn suppresses_completions_while_declaring_a_parameter_name() {
        let source = "fn greet(na";
        let response = CompletionEngine.complete(CompletionRequest {
            source,
            cursor: source.len(),
        });
        assert!(response.items.is_empty());
    }

    /// Offers standard types, but not expression symbols, in function parameter annotations.
    #[test]
    fn completes_types_while_declaring_a_parameter_annotation() {
        let source = "fn greet(name: Str";
        let response = CompletionEngine.complete(CompletionRequest {
            source,
            cursor: source.len(),
        });
        assert_eq!(item(&response, "String").kind, CompletionKind::Type);
        assert!(
            response
                .items
                .iter()
                .all(|item| item.kind != CompletionKind::Function)
        );
    }

    /// Uses the compiler-owned standard-library catalog for global constructors.
    #[test]
    fn completes_documented_standard_error_constructor() {
        let source = "fn main() { Err }";
        let response = CompletionEngine.complete(CompletionRequest {
            source,
            cursor: source.len() - 2,
        });
        let completion = item(&response, "Error");
        assert_eq!(completion.insert_text, "Error()");
        assert_eq!(completion.cursor, Some(6));
        assert_eq!(
            completion.detail.as_deref(),
            Some("Error(kind, message, data)")
        );
    }

    /// Completes documented runtime methods for an explicitly typed function parameter.
    #[test]
    fn completes_documented_methods_for_typed_parameters() {
        let source = "fn main(text: String) { text.le }";
        let response = CompletionEngine.complete(CompletionRequest {
            source,
            cursor: source.len() - 2,
        });
        let completion = item(&response, "length");
        assert_eq!(completion.insert_text, "length()");
        assert_eq!(completion.detail.as_deref(), Some("length() -> Int"));
    }

    /// Completes compiler-owned standard symbols through the reserved std namespace.
    #[test]
    fn completes_standard_symbols_through_the_std_namespace() {
        let source = "fn main() { std::Str }";
        let response = CompletionEngine.complete(CompletionRequest {
            source,
            cursor: source.len() - 2,
        });
        assert_eq!(item(&response, "String").kind, CompletionKind::Type);
    }

    /// Avoids opening an unsolicited completion list on an empty source position.
    #[test]
    fn suppresses_empty_prefix_completions() {
        let source = "fn main() {\n    \n}";
        let response = CompletionEngine.complete(CompletionRequest {
            source,
            cursor: source.find("    \n").unwrap_or_default() + 4,
        });
        assert!(response.items.is_empty());
        assert_eq!(response.replace, None);
    }

    /// Suppresses suggestions while the caret is inside a source comment.
    #[test]
    fn suppresses_completions_inside_comments() {
        for source in ["// Err", "/* Error"] {
            let response = CompletionEngine.complete(CompletionRequest {
                source,
                cursor: source.len(),
            });
            assert!(response.items.is_empty());
            assert_eq!(response.replace, None);
        }
    }
}
