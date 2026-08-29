//! Shared resolver-owned source graph loading for compiler consumers.

use std::collections::HashMap;
use std::path::Path;

use crate::ast::Module;
use crate::{ModuleResolver, SourceInput};

/// A fully resolved project source unit.
pub(crate) struct LoadedSource {
    /// Resolver-provided canonical module identity.
    pub(crate) source_id: String,
    /// Source-defined path displayed by documentation.
    pub(crate) display_path: String,
    /// Complete UTF-8 source text.
    pub(crate) text: String,
}

/// One resolved relative import between loaded project sources.
pub(crate) struct ImportEdge {
    /// Namespace exposed by the importing module.
    pub(crate) namespace: String,
    /// Byte offset of the import declaration start.
    pub(crate) start_byte: u32,
    /// Byte offset immediately after the import declaration.
    pub(crate) end_byte: u32,
    /// Target source index in [`LoadedProject::files`].
    pub(crate) target: usize,
}

/// One resolver-loaded project graph shared by compiler consumers.
pub(crate) struct LoadedProject {
    /// Loaded source units in deterministic discovery order.
    files: Vec<LoadedSource>,
    /// Resolved import edges indexed by importing source.
    edges: Vec<Vec<ImportEdge>>,
}

impl LoadedProject {
    /// Loads every relative import reachable from `source` without applying consumer semantics.
    pub(crate) fn load<R: ModuleResolver>(
        source: SourceInput<'_>,
        resolver: &mut R,
    ) -> Result<Self, String> {
        let mut files = vec![LoadedSource {
            source_id: source.source_id.to_owned(),
            display_path: root_display_path(source.source_id),
            text: source.text.to_owned(),
        }];
        let mut edges = vec![Vec::new()];
        let mut indices = HashMap::from([(source.source_id.to_owned(), 0_usize)]);
        let mut index = 0;
        while index < files.len() {
            let source_id = files[index].source_id.clone();
            let text = files[index].text.clone();
            let imports = parse_source(&source_id, &text)?.imports;
            for import in imports {
                let resolved = resolver
                    .resolve(&source_id, &import.path)
                    .map_err(|error| {
                        format!(
                            "{}:{}-{}: could not resolve import `{}`: {error}",
                            source_id, import.span.start_byte, import.span.end_byte, import.path
                        )
                    })?;
                let target = if let Some(target) = indices.get(&resolved.source_id) {
                    *target
                } else {
                    let target = files.len();
                    indices.insert(resolved.source_id.clone(), target);
                    files.push(LoadedSource {
                        source_id: resolved.source_id,
                        display_path: import.path.clone(),
                        text: resolved.text,
                    });
                    edges.push(Vec::new());
                    target
                };
                edges[index].push(ImportEdge {
                    namespace: import
                        .alias
                        .map_or_else(|| default_namespace(&import.path), |alias| alias.name),
                    start_byte: import.span.start_byte,
                    end_byte: import.span.end_byte,
                    target,
                });
            }
            index += 1;
        }
        Ok(Self { files, edges })
    }

    /// Returns the loaded sources in deterministic discovery order.
    pub(crate) fn files(&self) -> &[LoadedSource] {
        &self.files
    }

    /// Returns import edges indexed by importing source.
    pub(crate) fn edges(&self) -> &[Vec<ImportEdge>] {
        &self.edges
    }

    /// Parses every loaded source for one consumer-specific workflow.
    pub(crate) fn parse_modules(&self) -> Result<Vec<Module<'_>>, String> {
        self.files
            .iter()
            .map(|file| parse_source(&file.source_id, &file.text))
            .collect()
    }

    /// Rejects the loaded graph when any relative-import cycle exists.
    pub(crate) fn validate_no_cycles(&self) -> Result<(), String> {
        let Some(cycle) = find_cycle(&self.edges) else {
            return Ok(());
        };
        let chain = cycle
            .iter()
            .map(|index| self.files[*index].source_id.as_str())
            .collect::<Vec<_>>()
            .join(" -> ");
        Err(format!(
            "{}: import cycle: {chain}",
            self.files[0].source_id
        ))
    }
}

/// Parses one non-root source module and renders lexical or syntax diagnostics.
pub(crate) fn parse_source<'a>(source_id: &'a str, text: &'a str) -> Result<Module<'a>, String> {
    let lexed = crate::lexer::lex(SourceInput { source_id, text });
    if !lexed.diagnostics.is_empty() {
        return Err(lexed.diagnostics.render(text));
    }
    crate::parser::parse(source_id, lexed.tokens, false).map_err(|error| error.render(text))
}

/// Derives the default namespace from a relative `.exs` import path.
fn default_namespace(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .strip_suffix(".exs")
        .unwrap_or(path)
        .to_owned()
}

/// Converts an absolute root source identity into a concise documentation label.
fn root_display_path(source_id: &str) -> String {
    let path = Path::new(source_id);
    if path.is_absolute() {
        path.file_name()
            .and_then(|name| name.to_str())
            .map_or_else(|| source_id.to_owned(), |name| format!("./{name}"))
    } else {
        source_id.to_owned()
    }
}

/// Finds one directed cycle in an import graph.
fn find_cycle(edges: &[Vec<ImportEdge>]) -> Option<Vec<usize>> {
    fn visit(
        node: usize,
        edges: &[Vec<ImportEdge>],
        states: &mut [u8],
        stack: &mut Vec<usize>,
    ) -> Option<Vec<usize>> {
        states[node] = 1;
        stack.push(node);
        for edge in &edges[node] {
            if states[edge.target] == 1 {
                let start = stack.iter().position(|item| *item == edge.target)?;
                let mut cycle = stack[start..].to_vec();
                cycle.push(edge.target);
                return Some(cycle);
            }
            if states[edge.target] == 0
                && let Some(cycle) = visit(edge.target, edges, states, stack)
            {
                return Some(cycle);
            }
        }
        stack.pop();
        states[node] = 2;
        None
    }

    let mut states = vec![0; edges.len()];
    let mut stack = Vec::new();
    for node in 0..edges.len() {
        if states[node] == 0
            && let Some(cycle) = visit(node, edges, &mut states, &mut stack)
        {
            return Some(cycle);
        }
    }
    None
}
