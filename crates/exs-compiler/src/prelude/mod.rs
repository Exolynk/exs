//! Compiler-linked ExS source declarations available to every module.

use std::sync::OnceLock;

use crate::SourceInput;
use crate::ast::{Module, Visibility};
use crate::diagnostic::CompileDiagnostics;

/// One named ExS source unit bundled into the global standard prelude.
struct PreludeSource {
    /// Stable identity retained in diagnostics and embedded source metadata.
    source_id: &'static str,
    /// Complete ExS source text compiled from this unit.
    text: &'static str,
    /// Nominal type names made available before user source is parsed.
    type_names: &'static [&'static str],
}

/// Prelude units linked in deterministic dependency order.
const SOURCES: &[PreludeSource] = &[
    PreludeSource {
        source_id: "<std>/equality.exs",
        text: include_str!("equality.exs"),
        type_names: &[],
    },
    PreludeSource {
        source_id: "<std>/render.exs",
        text: include_str!("render.exs"),
        type_names: &[],
    },
    PreludeSource {
        source_id: "<std>/collections.exs",
        text: include_str!("collections.exs"),
        type_names: &[],
    },
    PreludeSource {
        source_id: "<std>/datetime.exs",
        text: include_str!("datetime.exs"),
        type_names: &["DateTime"],
    },
    PreludeSource {
        source_id: "<std>/duration.exs",
        text: include_str!("duration.exs"),
        type_names: &["Duration"],
    },
    PreludeSource {
        source_id: "<std>/iterator.exs",
        text: include_str!("iterator.exs"),
        type_names: &["IteratorStep", "HostStream"],
    },
    PreludeSource {
        source_id: "<std>/range.exs",
        text: include_str!("range.exs"),
        type_names: &["Range"],
    },
];

/// Returns all named prelude source units in linking order.
const fn sources() -> &'static [PreludeSource] {
    SOURCES
}

/// Returns prelude type names recognized while parsing user and prelude source.
pub(crate) fn type_names() -> impl Iterator<Item = &'static str> {
    sources()
        .iter()
        .flat_map(|source| source.type_names.iter().copied())
        .chain(["Bytes", "Int", "Float", "Object", "Math"])
}

/// The private prelude function that compares two composites of one kind part by part for
/// the built-in `==`.
pub(crate) const EQUALITY_HELPER: &str = "__exs_equal";

/// The private prelude function that renders a composite in ExS literal syntax.
pub(crate) const RENDER_HELPER: &str = "__exs_render";

/// Type mask of the values the prelude renders: composites, built part by part, and Bytes,
/// whose literal spelling the core escapes.
pub(crate) const RENDERED: u32 = exs_layout::TYPE_LIST
    | exs_layout::TYPE_OBJECT
    | exs_layout::TYPE_ERROR
    | exs_layout::TYPE_BYTES;

/// A private prelude function implementing a built-in method for some receivers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MethodHelper {
    /// The helper's name.
    pub(crate) key: &'static str,
    /// Type mask of the receivers the helper serves; other receivers use the built-in method.
    pub(crate) receivers: u32,
}

/// Returns the private prelude helper implementing the built-in method `method` taking
/// `arguments` arguments: the eager iterable methods for every receiver; the List searches,
/// sorting and aggregates, which compare and add through the operators, for Lists; and the
/// rendering of composites.
pub(crate) fn method_helper(method: &str, arguments: usize) -> Option<MethodHelper> {
    let iterable = |key| {
        Some(MethodHelper {
            key,
            receivers: exs_layout::TYPE_ANY,
        })
    };
    let composite = |key| {
        (arguments == 0).then_some(MethodHelper {
            key,
            receivers: RENDERED,
        })
    };
    let list = |key| {
        Some(MethodHelper {
            key,
            receivers: exs_layout::TYPE_LIST,
        })
    };
    match method {
        "map" => iterable("__exs_list_map"),
        "filter" => iterable("__exs_list_filter"),
        "find" => iterable("__exs_list_find"),
        "any" => iterable("__exs_list_any"),
        "all" => iterable("__exs_list_all"),
        "each" => iterable("__exs_list_each"),
        "reduce" => iterable("__exs_list_reduce"),
        "collect" => iterable("__exs_iterator_collect"),
        "count" => iterable("__exs_iterator_count"),
        "to_list" => iterable("__exs_iterator_to_list"),
        "contains" if arguments == 1 => list("__exs_list_contains"),
        "index_of" if arguments == 1 => list("__exs_list_index_of"),
        "last_index_of" if arguments == 1 => list("__exs_list_last_index_of"),
        "sort" if arguments == 0 => list("__exs_list_sort"),
        "sort" if arguments == 1 => list("__exs_list_sort_by"),
        "sorted" if arguments == 0 => list("__exs_list_sorted"),
        "sorted" if arguments == 1 => list("__exs_list_sorted_by"),
        "sum" if arguments == 0 => list("__exs_list_sum"),
        "min" if arguments == 0 => list("__exs_list_min"),
        "max" if arguments == 0 => list("__exs_list_max"),
        "flatten" if arguments == 0 => list("__exs_list_flatten"),
        "unique" if arguments == 0 => list("__exs_list_unique"),
        "to_string" | "debug" => composite(RENDER_HELPER),
        _ => None,
    }
}

/// Returns source metadata for all prelude units with a caller-selected lifetime.
pub(crate) fn source_inputs<'a>() -> Vec<SourceInput<'a>> {
    sources()
        .iter()
        .map(|source| SourceInput {
            source_id: source.source_id,
            text: source.text,
        })
        .collect()
}

/// The package of the prelude's declarations.
pub(crate) const STD_PACKAGE: &str = "std";

/// Returns the combined bundled prelude units; they are parsed once per process and cloned
/// for every compilation, which modifies its copy.
pub(crate) fn parse() -> Result<Module<'static>, CompileDiagnostics<'static>> {
    parsed().clone()
}

/// Returns the names of the prelude's private functions, which only the prelude may call.
pub(crate) fn private_functions() -> Vec<&'static str> {
    parsed().as_ref().map_or_else(
        |_| Vec::new(),
        |module| {
            module
                .functions
                .iter()
                .filter(|function| function.visibility == Visibility::Private)
                .map(|function| function.name.name.as_str())
                .collect()
        },
    )
}

/// Returns the prelude, parsed on first use.
fn parsed() -> &'static Result<Module<'static>, CompileDiagnostics<'static>> {
    static PRELUDE: OnceLock<Result<Module<'static>, CompileDiagnostics<'static>>> =
        OnceLock::new();
    PRELUDE.get_or_init(parse_sources)
}

/// Parses and combines all bundled prelude units without requiring a root entry point.
fn parse_sources() -> Result<Module<'static>, CompileDiagnostics<'static>> {
    let mut combined = Module {
        imports: Vec::new(),
        uses: Vec::new(),
        types: Vec::new(),
        enums: Vec::new(),
        traits: Vec::new(),
        implementations: Vec::new(),
        functions: Vec::new(),
        constants: Vec::new(),
        tests: Vec::new(),
    };
    let mut diagnostics = CompileDiagnostics::new();
    for source in sources() {
        let lexed = crate::lexer::lex(SourceInput {
            source_id: source.source_id,
            text: source.text,
        });
        diagnostics.extend(lexed.diagnostics);
        match crate::parser::parse(source.source_id, lexed.tokens, false) {
            Ok(mut module) => {
                for function in &mut module.functions {
                    function.entry = false;
                    function.package = STD_PACKAGE.to_owned();
                }
                for method in module
                    .implementations
                    .iter_mut()
                    .flat_map(|implementation| &mut implementation.methods)
                {
                    method.package = STD_PACKAGE.to_owned();
                }
                combined.types.append(&mut module.types);
                combined.enums.append(&mut module.enums);
                combined.traits.append(&mut module.traits);
                combined.implementations.append(&mut module.implementations);
                combined.functions.append(&mut module.functions);
            }
            Err(parser_diagnostics) => diagnostics.extend(parser_diagnostics),
        }
    }
    if diagnostics.is_empty() {
        Ok(combined)
    } else {
        diagnostics.sort_by_span();
        Err(diagnostics)
    }
}
