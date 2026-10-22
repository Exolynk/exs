use std::collections::{BTreeMap, BTreeSet};

use exs_compiler::{
    SourceInput, source_lex, standard_library_enums, standard_library_namespace,
    standard_library_types,
};

use crate::catalog::{
    CompletionPresentation, append_keywords, append_standard_functions,
    append_standard_namespace_functions, append_standard_namespaces, append_standard_symbols,
    push_documented_if_matching, push_if_matching,
};
use crate::syntax::{
    DocumentSymbols, FunctionHeaderContext, FunctionParameter, FunctionSignature, SymbolKind,
    UseBinding, call_argument_context, document_symbols, function_header_context,
    identifier_prefix, is_function_name_declaration_context, is_host_member_context,
    is_type_context, library_symbols, member_receiver, namespace_receiver, use_bindings,
    visible_binding_types, visible_bindings,
};
use crate::{CompletionItem, CompletionKind, CompletionRequest, CompletionResponse};

/// Completion engine for one ExS source document and the library modules the embedder
/// provides.
#[derive(Clone, Debug, Default)]
pub struct CompletionEngine {
    /// Public declarations of every library module, keyed by its path such as `exo::db`.
    modules: BTreeMap<String, DocumentSymbols>,
}

impl CompletionEngine {
    /// Adds the public declarations of the library module at `path`, such as `log` or
    /// `exo::db`, which documents may name without importing it.
    #[must_use]
    pub fn with_library(mut self, path: &str, source: &str) -> Self {
        self.modules
            .insert(path.to_owned(), library_symbols(source));
        self
    }

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
        let symbols = document_symbols(request.source, &tokens);
        let uses = use_bindings(&tokens);
        if is_function_name_declaration_context(request.source, prefix_start) {
            return CompletionResponse::default();
        }
        match function_header_context(&tokens, prefix_start) {
            Some(FunctionHeaderContext::ParameterName) => return CompletionResponse::default(),
            Some(FunctionHeaderContext::ParameterType) => {
                let mut items = Vec::new();
                append_standard_symbols(&mut items, prefix);
                append_document_types(&mut items, prefix, &symbols);
                append_used_items(&mut items, prefix, &uses, &self.modules, true);
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
            return response_for_namespace(
                prefix,
                prefix_start,
                request.cursor,
                &symbols,
                &expand_use(receiver, &uses),
                &self.modules,
            );
        }
        if prefix.is_empty() {
            return CompletionResponse::default();
        }

        let mut items = Vec::new();
        if is_type_context(request.source, prefix_start) {
            append_standard_symbols(&mut items, prefix);
            append_document_types(&mut items, prefix, &symbols);
            append_used_items(&mut items, prefix, &uses, &self.modules, true);
        } else {
            append_keywords(&mut items, prefix);
            append_standard_functions(&mut items, prefix);
            append_standard_symbols(&mut items, prefix);
            append_standard_namespaces(&mut items, prefix);
            append_module_namespaces(&mut items, prefix, &self.modules);
            append_used_items(&mut items, prefix, &uses, &self.modules, false);
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
        append_method_completion(
            &mut items,
            prefix,
            method.signature,
            Some(method.description),
        );
    }
    let clone_signature = format!("clone() -> {} | Error", type_info.name);
    append_method_completion(&mut items, prefix, &clone_signature, None);
    unique_response(items, prefix_start, cursor)
}

/// Appends one documented method call with parentheses and a suitable caret position.
fn append_method_completion(
    items: &mut Vec<CompletionItem>,
    prefix: &str,
    signature: &str,
    documentation: Option<&str>,
) {
    let Some((name, arguments)) = signature.split_once('(') else {
        return;
    };
    let insert_text = format!("{name}()");
    let cursor = (!arguments.starts_with(')')).then_some(name.len() + 1);
    push_documented_if_matching(
        items,
        prefix,
        name,
        CompletionPresentation::new(Some(signature), documentation),
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
        push_documented_if_matching(
            &mut items,
            prefix,
            function.name,
            CompletionPresentation::new(Some(function.signature), Some(function.description)),
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
    symbols: &DocumentSymbols,
    receiver: &str,
    modules: &BTreeMap<String, DocumentSymbols>,
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
    if let Some(variants) = symbols.variants.get(receiver) {
        append_variants(&mut items, prefix, variants.iter().map(String::as_str));
    }
    append_module_members(&mut items, prefix, receiver, modules);
    unique_response(items, prefix_start, cursor)
}

/// Returns `receiver` with a leading name a `use` declaration brought into scope replaced by
/// the path it stands for.
fn expand_use(receiver: &str, uses: &[UseBinding]) -> String {
    let (first, rest) = receiver
        .split_once("::")
        .map_or((receiver, None), |(first, rest)| (first, Some(rest)));
    match uses.iter().rev().find(|binding| binding.alias == first) {
        Some(binding) => rest.map_or_else(
            || binding.target.clone(),
            |rest| format!("{}::{rest}", binding.target),
        ),
        None => receiver.to_owned(),
    }
}

/// Splits `path` into the longest library path it starts with and the rest.
fn split_library<'p>(
    path: &'p str,
    modules: &BTreeMap<String, DocumentSymbols>,
) -> Option<(&'p str, &'p str)> {
    let mut end = Some(path.len());
    while let Some(index) = end {
        let library = &path[..index];
        if modules.contains_key(library) {
            let rest = path[index..].strip_prefix("::").unwrap_or_default();
            return Some((library, rest));
        }
        end = library.rfind("::");
    }
    None
}

/// Appends the first segments of the library paths as expression completions.
fn append_module_namespaces(
    items: &mut Vec<CompletionItem>,
    prefix: &str,
    modules: &BTreeMap<String, DocumentSymbols>,
) {
    let roots = modules
        .keys()
        .map(|path| path.split("::").next().unwrap_or(path))
        .collect::<BTreeSet<_>>();
    for root in roots {
        push_if_matching(
            items,
            prefix,
            root,
            Some("Library"),
            &format!("{root}::"),
            Some(root.len() + 2),
            CompletionKind::Module,
        );
    }
}

/// Appends the library declarations and modules `use` declarations brought into scope; only
/// types, enums, and traits when `types_only`.
fn append_used_items(
    items: &mut Vec<CompletionItem>,
    prefix: &str,
    uses: &[UseBinding],
    modules: &BTreeMap<String, DocumentSymbols>,
    types_only: bool,
) {
    for binding in uses {
        if modules.contains_key(&binding.target) {
            if !types_only {
                push_if_matching(
                    items,
                    prefix,
                    &binding.alias,
                    Some("Library"),
                    &format!("{}::", binding.alias),
                    Some(binding.alias.len() + 2),
                    CompletionKind::Module,
                );
            }
            continue;
        }
        let Some((library, name)) = binding.target.rsplit_once("::") else {
            continue;
        };
        let Some(symbols) = modules.get(library) else {
            continue;
        };
        let documentation = symbols.documentation.get(name).map(String::as_str);
        let (detail, kind) = match symbols.declarations.get(name) {
            Some(SymbolKind::Function) => {
                if let Some(signature) = symbols.functions.get(name).filter(|_| !types_only) {
                    append_function_completion(
                        items,
                        prefix,
                        &binding.alias,
                        signature,
                        documentation,
                    );
                }
                continue;
            }
            Some(SymbolKind::Type) => ("Library type", CompletionKind::Type),
            Some(SymbolKind::Enum) => ("Library enum", CompletionKind::Enum),
            Some(SymbolKind::Trait) => ("Library trait", CompletionKind::Trait),
            None => continue,
        };
        push_documented_if_matching(
            items,
            prefix,
            &binding.alias,
            CompletionPresentation::new(Some(detail), documentation),
            &binding.alias,
            None,
            kind,
        );
    }
}

/// Appends the declarations of the library module `receiver` names, or the static methods
/// and variants of the library type or enum it names, and its child library modules.
fn append_module_members(
    items: &mut Vec<CompletionItem>,
    prefix: &str,
    receiver: &str,
    modules: &BTreeMap<String, DocumentSymbols>,
) {
    let Some((library, path)) = split_library(receiver, modules) else {
        return;
    };
    let Some(symbols) = modules.get(library) else {
        return;
    };
    if path.is_empty() {
        let children = format!("{library}::");
        for child in modules
            .keys()
            .filter_map(|key| key.strip_prefix(&children))
            .filter(|child| !child.contains("::"))
        {
            push_if_matching(
                items,
                prefix,
                child,
                Some("Library"),
                &format!("{child}::"),
                Some(child.len() + 2),
                CompletionKind::Module,
            );
        }
    }
    let namespace_prefix = if path.is_empty() {
        String::new()
    } else {
        format!("{path}::")
    };
    if let Some(variants) = symbols.variants.get(path) {
        append_variants(items, prefix, variants.iter().map(String::as_str));
    }
    let mut child_namespaces = BTreeSet::new();
    for (full_name, signature) in &symbols.functions {
        let Some(name) = full_name.strip_prefix(&namespace_prefix) else {
            continue;
        };
        if let Some((child, _)) = name.split_once("::") {
            if !symbols.declarations.contains_key(child) {
                child_namespaces.insert(child);
            }
            continue;
        }
        append_function_completion(
            items,
            prefix,
            name,
            signature,
            symbols.documentation.get(full_name).map(String::as_str),
        );
    }
    for (full_name, kind) in &symbols.declarations {
        if *kind == SymbolKind::Function {
            continue;
        }
        let Some(name) = full_name.strip_prefix(&namespace_prefix) else {
            continue;
        };
        if let Some((child, _)) = name.split_once("::") {
            child_namespaces.insert(child);
            continue;
        }
        let (detail, completion_kind) = match kind {
            SymbolKind::Type => ("Library type", CompletionKind::Type),
            SymbolKind::Enum => ("Library enum", CompletionKind::Enum),
            SymbolKind::Trait => ("Library trait", CompletionKind::Trait),
            SymbolKind::Function => continue,
        };
        push_documented_if_matching(
            items,
            prefix,
            name,
            CompletionPresentation::new(
                Some(detail),
                symbols.documentation.get(full_name).map(String::as_str),
            ),
            name,
            None,
            completion_kind,
        );
    }
    for namespace in child_namespaces {
        push_if_matching(
            items,
            prefix,
            namespace,
            Some("Module namespace"),
            &format!("{namespace}::"),
            Some(namespace.len() + 2),
            CompletionKind::Module,
        );
    }
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
        push_documented_if_matching(
            items,
            prefix,
            name,
            CompletionPresentation::new(
                Some(detail),
                symbols.documentation.get(name).map(String::as_str),
            ),
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
                append_function_completion(
                    items,
                    prefix,
                    name,
                    signature,
                    symbols.documentation.get(name).map(String::as_str),
                );
                continue;
            }
            SymbolKind::Type => ("Source type", CompletionKind::Type),
            SymbolKind::Enum => ("Source enum", CompletionKind::Enum),
            SymbolKind::Trait => ("Source trait", CompletionKind::Trait),
        };
        push_documented_if_matching(
            items,
            prefix,
            name,
            CompletionPresentation::new(
                Some(detail),
                symbols.documentation.get(name).map(String::as_str),
            ),
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
    documentation: Option<&str>,
) {
    let insert_text = format!("{name}()");
    let cursor = if signature.parameters.is_empty() {
        Some(insert_text.len())
    } else {
        Some(name.len() + 1)
    };
    push_documented_if_matching(
        items,
        prefix,
        name,
        CompletionPresentation::new(Some(&function_detail(name, signature)), documentation),
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
        let response = CompletionEngine::default().complete(CompletionRequest {
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
        let response = CompletionEngine::default().complete(CompletionRequest {
            source,
            cursor: source.len() - 2,
        });
        assert_eq!(item(&response, "First").kind, CompletionKind::Variant);
    }

    /// Completes one available Host boundary operation after `Host::`.
    #[test]
    fn completes_host_call() {
        let source = "fn main() { Host::ca }";
        let response = CompletionEngine::default().complete(CompletionRequest {
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
        let response = CompletionEngine::default().complete(CompletionRequest {
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
        let response = CompletionEngine::default().complete(CompletionRequest {
            source,
            cursor: source.len() - 2,
        });
        let completion = item(&response, "sleep");
        assert_eq!(completion.insert_text, "sleep()");
    }

    /// Completes the built-in Host wall-clock operation after a namespace separator.
    #[test]
    fn completes_host_now() {
        let source = "fn main() { Host::no }";
        let response = CompletionEngine::default().complete(CompletionRequest {
            source,
            cursor: source.len() - 2,
        });
        let completion = item(&response, "now");
        assert_eq!(completion.insert_text, "now()");
    }

    /// Completes the built-in Host namespace in expression context.
    #[test]
    fn completes_host_namespace() {
        let source = "fn main() { Hos }";
        let response = CompletionEngine::default().complete(CompletionRequest {
            source,
            cursor: source.len() - 2,
        });
        assert_eq!(item(&response, "Host").kind, CompletionKind::Module);
    }

    /// Inserts required whitespace after accepting a local binding declaration keyword.
    #[test]
    fn completes_let_with_trailing_whitespace() {
        let source = "fn main() { le }";
        let response = CompletionEngine::default().complete(CompletionRequest {
            source,
            cursor: source.len() - 2,
        });
        assert_eq!(item(&response, "let").insert_text, "let ");
    }

    /// Inserts parentheses and places the caret inside a function call with parameters.
    #[test]
    fn completes_parameterized_function_calls() {
        let source = "fn greet(name: String) {}\nfn main() { gre }";
        let response = CompletionEngine::default().complete(CompletionRequest {
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
        let response = CompletionEngine::default().complete(CompletionRequest { source, cursor });
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
        let response = CompletionEngine::default().complete(CompletionRequest { source, cursor });
        assert!(response.items.is_empty());
    }

    /// Avoids turning a function declaration name into an invocation completion.
    #[test]
    fn suppresses_function_call_completion_while_declaring_a_function() {
        let source = "fn gre";
        let response = CompletionEngine::default().complete(CompletionRequest {
            source,
            cursor: source.len(),
        });
        assert!(response.items.is_empty());
    }

    /// Avoids offering expression names while a function parameter name is declared.
    #[test]
    fn suppresses_completions_while_declaring_a_parameter_name() {
        let source = "fn greet(na";
        let response = CompletionEngine::default().complete(CompletionRequest {
            source,
            cursor: source.len(),
        });
        assert!(response.items.is_empty());
    }

    /// Offers standard types, but not expression symbols, in function parameter annotations.
    #[test]
    fn completes_types_while_declaring_a_parameter_annotation() {
        let source = "fn greet(name: Str";
        let response = CompletionEngine::default().complete(CompletionRequest {
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
        let response = CompletionEngine::default().complete(CompletionRequest {
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
        let response = CompletionEngine::default().complete(CompletionRequest {
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
        let response = CompletionEngine::default().complete(CompletionRequest {
            source,
            cursor: source.len() - 2,
        });
        assert_eq!(item(&response, "String").kind, CompletionKind::Type);
    }

    /// Sends standard-library explanations separately from the compact list detail.
    #[test]
    fn completes_standard_namespace_with_documentation() {
        let source = "Mat";
        let response = CompletionEngine::default().complete(CompletionRequest {
            source,
            cursor: source.len(),
        });
        let math = item(&response, "Math");
        assert_eq!(math.detail.as_deref(), Some("Built-in type"));
        assert!(
            math.documentation
                .as_deref()
                .is_some_and(|documentation| documentation.contains("static namespace"))
        );
    }

    /// Completes static imported type methods without exposing instance or private methods.
    #[test]
    fn completes_configured_type_static_methods() {
        let engine = CompletionEngine::default().with_library(
            "camt",
            "type Transaction {} impl Transaction {\n/// Parses a document.\nfn parse(content: String) -> Transaction {} fn status(self) {} fn __internal() {} }",
        );
        let source = "camt::Transaction::";
        let response = engine.complete(CompletionRequest {
            source,
            cursor: source.len(),
        });
        let parse = item(&response, "parse");
        assert_eq!(parse.insert_text, "parse()");
        assert_eq!(parse.documentation.as_deref(), Some("Parses a document."));
        assert!(
            !response
                .items
                .iter()
                .any(|item| matches!(item.label.as_str(), "status" | "__internal"))
        );
        let source = "camt::Trans";
        let response = engine.complete(CompletionRequest {
            source,
            cursor: source.len(),
        });
        assert_eq!(item(&response, "Transaction").kind, CompletionKind::Type);
        assert_eq!(
            response
                .items
                .iter()
                .filter(|item| item.label == "Transaction")
                .count(),
            1
        );
    }

    /// Completes functions, types, and child modules of library modules.
    #[test]
    fn completes_configured_virtual_module_members() {
        let engine = CompletionEngine::default()
            .with_library(
                "exo",
                "/// Localized text.\ntype Language {}\n/// Returns the active environment.\nfn get_environment() {}\nprivate fn __exo_hydrate_environment() {}",
            )
            .with_library(
                "exo::ui",
                "/// Reloads the browser page.\nfn reload() {}",
            );

        let module_source = "ex";
        let module_response = engine.complete(CompletionRequest {
            source: module_source,
            cursor: module_source.len(),
        });
        assert_eq!(item(&module_response, "exo").insert_text, "exo::");

        let root_source = "exo::get";
        let root_response = engine.complete(CompletionRequest {
            source: root_source,
            cursor: root_source.len(),
        });
        assert_eq!(
            item(&root_response, "get_environment").insert_text,
            "get_environment()"
        );

        let type_source = "exo::Lan";
        let type_response = engine.complete(CompletionRequest {
            source: type_source,
            cursor: type_source.len(),
        });
        let language = item(&type_response, "Language");
        assert_eq!(language.kind, CompletionKind::Type);
        assert_eq!(language.documentation.as_deref(), Some("Localized text."));

        let namespace_source = "exo::u";
        let namespace_response = engine.complete(CompletionRequest {
            source: namespace_source,
            cursor: namespace_source.len(),
        });
        let namespace = item(&namespace_response, "ui");
        assert_eq!(namespace.kind, CompletionKind::Module);
        assert_eq!(namespace.insert_text, "ui::");

        let nested_source = "exo::ui::re";
        let nested_response = engine.complete(CompletionRequest {
            source: nested_source,
            cursor: nested_source.len(),
        });
        let reload = item(&nested_response, "reload");
        assert_eq!(reload.insert_text, "reload()");
        assert_eq!(
            reload.documentation.as_deref(),
            Some("Reloads the browser page.")
        );

        let private_source = "exo::__exo";
        let private_response = engine.complete(CompletionRequest {
            source: private_source,
            cursor: private_source.len(),
        });
        assert!(private_response.items.is_empty());
    }

    /// Hides the private declarations of a library and keeps the document's own.
    #[test]
    fn hides_private_library_declarations() {
        let engine = CompletionEngine::default().with_library(
            "log",
            "fn info(message: String) {}\nprivate fn format(message: String) {}\nprivate type Entry {}\ntype Level {} impl Level { private fn parse(text: String) {} fn named(text: String) {} }",
        );
        let response = engine.complete(CompletionRequest {
            source: "log::",
            cursor: 5,
        });
        assert!(response.items.iter().any(|item| item.label == "info"));
        assert!(
            !response
                .items
                .iter()
                .any(|item| matches!(item.label.as_str(), "format" | "Entry"))
        );
        let response = engine.complete(CompletionRequest {
            source: "log::Level::",
            cursor: 12,
        });
        assert!(response.items.iter().any(|item| item.label == "named"));
        assert!(!response.items.iter().any(|item| item.label == "parse"));
        let source = "private fn helper() {}\nfn main() { hel }";
        let response = engine.complete(CompletionRequest {
            source,
            cursor: source.len() - 2,
        });
        assert_eq!(item(&response, "helper").kind, CompletionKind::Function);
    }

    /// Completes names and modules that `use` declarations bring into scope.
    #[test]
    fn completes_used_library_items() {
        let engine = CompletionEngine::default()
            .with_library("exo", "/// A stored record.\ntype Record {}")
            .with_library(
                "exo::ui",
                "/// Reloads the browser page.\nfn reload() {}\n/// Opens a popup.\nfn popup(title: String) {}",
            );
        let source =
            "use exo::ui::{reload, popup as open};\nuse exo::ui;\nuse exo::Record;\nfn main() { ";
        let complete = |text: &str| {
            let source = format!("{source}{text}");
            engine.complete(CompletionRequest {
                source: &source,
                cursor: source.len(),
            })
        };
        let response = complete("rel");
        assert_eq!(item(&response, "reload").insert_text, "reload()");
        let response = complete("op");
        let open = item(&response, "open");
        assert_eq!(open.insert_text, "open()");
        assert_eq!(open.documentation.as_deref(), Some("Opens a popup."));
        let response = complete("u");
        assert_eq!(item(&response, "ui").insert_text, "ui::");
        let response = complete("ui::po");
        assert_eq!(item(&response, "popup").insert_text, "popup()");
        let response = complete("Rec");
        assert_eq!(item(&response, "Record").kind, CompletionKind::Type);
    }

    /// Avoids opening an unsolicited completion list on an empty source position.
    #[test]
    fn suppresses_empty_prefix_completions() {
        let source = "fn main() {\n    \n}";
        let response = CompletionEngine::default().complete(CompletionRequest {
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
            let response = CompletionEngine::default().complete(CompletionRequest {
                source,
                cursor: source.len(),
            });
            assert!(response.items.is_empty());
            assert_eq!(response.replace, None);
        }
    }
}
