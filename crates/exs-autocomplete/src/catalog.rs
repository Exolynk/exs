use exs_compiler::{
    standard_library_enums, standard_library_functions, standard_library_namespace,
    standard_library_namespaces, standard_library_traits, standard_library_types,
};

use crate::{CompletionItem, CompletionKind};

/// Adds one catalog item when its label starts with the active source prefix.
pub(crate) fn push_if_matching(
    items: &mut Vec<CompletionItem>,
    prefix: &str,
    label: &str,
    detail: Option<&str>,
    insert_text: &str,
    cursor: Option<usize>,
    kind: CompletionKind,
) {
    if label.starts_with(prefix) {
        items.push(CompletionItem {
            label: label.to_owned(),
            detail: detail.map(str::to_owned),
            insert_text: insert_text.to_owned(),
            cursor,
            kind,
        });
    }
}

/// Appends ExS control-flow and declaration keywords with their required trailing syntax.
pub(crate) fn append_keywords(items: &mut Vec<CompletionItem>, prefix: &str) {
    for (label, detail, insert_text) in [
        ("as", "ExS keyword", "as "),
        ("break", "ExS keyword", "break"),
        ("continue", "ExS keyword", "continue"),
        ("else", "ExS keyword", "else "),
        ("enum", "Enum declaration", "enum "),
        ("fn", "Function declaration", "fn "),
        ("for", "Loop over an iterable", "for "),
        ("if", "Conditional block", "if "),
        ("impl", "Implementation block", "impl "),
        ("import", "Module import", "import "),
        ("in", "ExS keyword", "in "),
        ("let", "Local binding declaration", "let "),
        ("match", "Exhaustive match", "match "),
        ("par", "Parallel task expression", "par "),
        ("ret", "Return statement", "ret "),
        ("trait", "Trait declaration", "trait "),
        ("type", "Type declaration", "type "),
        ("use", "Imported declaration", "use "),
        ("while", "Loop block", "while "),
    ] {
        push_if_matching(
            items,
            prefix,
            label,
            Some(detail),
            insert_text,
            None,
            CompletionKind::Keyword,
        );
    }
}

/// Appends the standard ExS type, trait, and enum names.
pub(crate) fn append_standard_symbols(items: &mut Vec<CompletionItem>, prefix: &str) {
    for type_info in standard_library_types() {
        push_if_matching(
            items,
            prefix,
            type_info.name,
            Some("Built-in type"),
            type_info.name,
            None,
            CompletionKind::Type,
        );
    }
    for trait_info in standard_library_traits() {
        push_if_matching(
            items,
            prefix,
            trait_info.name,
            Some("Standard trait"),
            trait_info.name,
            None,
            CompletionKind::Trait,
        );
    }
    for enum_info in standard_library_enums() {
        push_if_matching(
            items,
            prefix,
            enum_info.name,
            Some("Standard enum"),
            enum_info.name,
            None,
            CompletionKind::Enum,
        );
    }
}

/// Appends global standard namespaces valid in expression contexts.
pub(crate) fn append_standard_namespaces(items: &mut Vec<CompletionItem>, prefix: &str) {
    for namespace in standard_library_namespaces()
        .iter()
        .filter(|namespace| namespace.name != "Duration")
    {
        push_if_matching(
            items,
            prefix,
            namespace.name,
            Some(namespace.description),
            namespace.name,
            None,
            CompletionKind::Type,
        );
    }
}

/// Appends the static operations documented for one standard namespace.
pub(crate) fn append_standard_namespace_functions(
    items: &mut Vec<CompletionItem>,
    prefix: &str,
    namespace_name: &str,
) {
    let Some(namespace) = standard_library_namespace(namespace_name) else {
        return;
    };
    for function in namespace.functions {
        let insert_text = format!("{}()", function.name);
        let cursor = function
            .signature
            .split_once('(')
            .is_some_and(|(_, arguments)| !arguments.starts_with(')'))
            .then_some(function.name.len() + 1);
        push_if_matching(
            items,
            prefix,
            function.name,
            Some(function.signature),
            &insert_text,
            cursor,
            CompletionKind::Function,
        );
    }
}

/// Appends globally callable standard-library functions in expression contexts.
pub(crate) fn append_standard_functions(items: &mut Vec<CompletionItem>, prefix: &str) {
    for function in standard_library_functions()
        .iter()
        .filter(|function| !function.name.contains('.'))
    {
        let insert_text = format!("{}()", function.name);
        let has_arguments = function
            .signature
            .split_once('(')
            .is_some_and(|(_, arguments)| !arguments.starts_with(')'));
        let cursor = has_arguments.then_some(function.name.len() + 1);
        push_if_matching(
            items,
            prefix,
            function.name,
            Some(function.signature),
            &insert_text,
            cursor,
            CompletionKind::Function,
        );
    }
}
