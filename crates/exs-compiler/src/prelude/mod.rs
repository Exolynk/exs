//! Compiler-linked ExS source declarations available to every module.

use crate::SourceInput;
use crate::ast::Module;
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
const SOURCES: &[PreludeSource] = &[PreludeSource {
    source_id: "<std>/duration.exs",
    text: include_str!("duration.exs"),
    type_names: &["Duration"],
}];

/// Returns all named prelude source units in linking order.
const fn sources() -> &'static [PreludeSource] {
    SOURCES
}

/// Returns prelude type names recognized while parsing user and prelude source.
pub(crate) fn type_names() -> impl Iterator<Item = &'static str> {
    sources()
        .iter()
        .flat_map(|source| source.type_names.iter().copied())
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

/// Parses and combines all bundled prelude units without requiring a root entry point.
pub(crate) fn parse() -> Result<Module<'static>, CompileDiagnostics<'static>> {
    let mut combined = Module {
        imports: Vec::new(),
        uses: Vec::new(),
        types: Vec::new(),
        enums: Vec::new(),
        traits: Vec::new(),
        implementations: Vec::new(),
        functions: Vec::new(),
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
