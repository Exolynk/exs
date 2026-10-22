//! Shared resolver-owned source graph loading for compiler consumers.

use std::collections::HashMap;
use std::path::Path;

use crate::ast::{ImportDeclaration, Module};
use crate::lexer::{FormattedStringPart, Token, TokenKind};
use crate::{CompileLimits, ModuleResolver, ResolvedSource, SourceInput};

/// A fully resolved project source unit.
pub(crate) struct LoadedSource {
    /// Resolver-provided canonical module identity.
    pub(crate) source_id: String,
    /// Source-defined path displayed by documentation.
    pub(crate) display_path: String,
    /// Complete UTF-8 source text.
    pub(crate) text: String,
    /// The path of the library module this source provides, such as `exo::db`, or `None`
    /// for a source of the project.
    pub(crate) library: Option<String>,
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
    /// Deepest nesting the parser accepts.
    max_nesting: usize,
}

impl LoadedProject {
    /// Loads every relative import reachable from `source` and every library module the
    /// loaded sources name, within the size and module limits of `limits`.
    ///
    /// A library module is loaded when a source names a path the resolver provides as a
    /// library, such as `log` in `log::info` or `exo::db` in `use exo::db::query;`, so
    /// unused libraries cost nothing. Importing a library by its path is accepted and only
    /// loads it.
    pub(crate) fn load<R: ModuleResolver>(
        source: SourceInput<'_>,
        resolver: &mut R,
        limits: CompileLimits,
    ) -> Result<Self, String> {
        let mut loader = Loader::new(limits);
        loader.add(
            ResolvedSource {
                source_id: source.source_id.to_owned(),
                text: source.text.to_owned(),
            },
            root_display_path(source.source_id),
            None,
        )?;
        loader.load_reachable(resolver)
    }

    /// Loads the library modules at `paths`, which must exist, and everything they reach.
    pub(crate) fn load_libraries<R: ModuleResolver>(
        paths: &[String],
        resolver: &mut R,
        limits: CompileLimits,
    ) -> Result<Self, String> {
        let mut loader = Loader::new(limits);
        for path in paths {
            if loader.library(resolver, path)?.is_none() {
                return Err(format!(
                    "library `{path}` is listed but the resolver does not provide it"
                ));
            }
        }
        loader.load_reachable(resolver)
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
            .map(|file| parse_source(&file.source_id, &file.text, self.max_nesting))
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

/// The sources a loader has gathered so far.
struct Loader {
    /// Loaded sources in discovery order.
    files: Vec<LoadedSource>,
    /// Import edges indexed by importing source.
    edges: Vec<Vec<ImportEdge>>,
    /// Index of every loaded source by its identity.
    indices: HashMap<String, usize>,
    /// Every path asked of the resolver as a library, with the index of its source if the
    /// resolver provides one.
    libraries: HashMap<String, Option<usize>>,
    /// Bytes of all loaded sources.
    total_bytes: usize,
    /// Limits of the compilation.
    limits: CompileLimits,
}

impl Loader {
    /// Returns a loader without sources.
    fn new(limits: CompileLimits) -> Self {
        Self {
            files: Vec::new(),
            edges: Vec::new(),
            indices: HashMap::new(),
            libraries: HashMap::new(),
            total_bytes: 0,
            limits,
        }
    }

    /// Loads every relative import and library module the loaded sources reach.
    fn load_reachable<R: ModuleResolver>(
        mut self,
        resolver: &mut R,
    ) -> Result<LoadedProject, String> {
        let mut index = 0;
        while index < self.files.len() {
            let source_id = self.files[index].source_id.clone();
            let text = self.files[index].text.clone();
            let scanned = scan_source(&source_id, &text)?;
            for import in scanned.imports {
                if let Some(target) = self.library(resolver, &import.path)? {
                    // A library is visible everywhere; only a different alias needs an edge.
                    if let Some(alias) = import.alias.filter(|alias| alias.name != import.path) {
                        self.edges[index].push(ImportEdge {
                            namespace: alias.name,
                            start_byte: import.span.start_byte,
                            end_byte: import.span.end_byte,
                            target,
                        });
                    }
                    continue;
                }
                let resolved = resolver
                    .resolve(&source_id, &import.path)
                    .map_err(|error| {
                        format!(
                            "{}:{}-{}: could not resolve import `{}`: {error}",
                            source_id, import.span.start_byte, import.span.end_byte, import.path
                        )
                    })?;
                let target = self.add(resolved, import.path.clone(), None)?;
                self.edges[index].push(ImportEdge {
                    namespace: import
                        .alias
                        .map_or_else(|| default_namespace(&import.path), |alias| alias.name),
                    start_byte: import.span.start_byte,
                    end_byte: import.span.end_byte,
                    target,
                });
            }
            for path in scanned.paths {
                let mut prefix = String::new();
                for segment in path.split("::") {
                    if !prefix.is_empty() {
                        prefix.push_str("::");
                    }
                    prefix.push_str(segment);
                    self.library(resolver, &prefix)?;
                }
            }
            index += 1;
        }
        Ok(LoadedProject {
            files: self.files,
            edges: self.edges,
            max_nesting: self.limits.max_nesting,
        })
    }

    /// Adds `source` unless a source of its identity is loaded already, and returns its index.
    fn add(
        &mut self,
        source: ResolvedSource,
        display_path: String,
        library: Option<String>,
    ) -> Result<usize, String> {
        if let Some(index) = self.indices.get(&source.source_id) {
            return Ok(*index);
        }
        if self.files.len() >= self.limits.max_modules {
            return Err(format!(
                "{}: the project exceeds the limit of {} modules",
                self.files
                    .first()
                    .map_or(source.source_id.as_str(), |root| root.source_id.as_str()),
                self.limits.max_modules
            ));
        }
        self.total_bytes = check_size(
            &source.source_id,
            source.text.len(),
            self.total_bytes,
            self.limits,
        )?;
        let index = self.files.len();
        self.indices.insert(source.source_id.clone(), index);
        self.files.push(LoadedSource {
            source_id: source.source_id,
            display_path,
            text: source.text,
            library,
        });
        self.edges.push(Vec::new());
        Ok(index)
    }

    /// Loads the library module at `path` if the resolver provides one, asking it once per
    /// path, and returns its index.
    fn library<R: ModuleResolver>(
        &mut self,
        resolver: &mut R,
        path: &str,
    ) -> Result<Option<usize>, String> {
        if let Some(index) = self.libraries.get(path) {
            return Ok(*index);
        }
        let index = match resolver.library(path) {
            Some(source) => Some(self.add(source, path.to_owned(), Some(path.to_owned()))?),
            None => None,
        };
        self.libraries.insert(path.to_owned(), index);
        Ok(index)
    }
}

/// What loading needs from one source before it is parsed.
struct ScannedSource<'a> {
    /// Its import declarations.
    imports: Vec<ImportDeclaration<'a>>,
    /// Every qualified path it spells whose first segment starts with a lowercase letter,
    /// such as `exo::db::query`; library names are lowercase.
    paths: Vec<String>,
}

/// Scans one source for its imports and the qualified paths it names, and renders its
/// lexical diagnostics or the syntax diagnostics of its imports; the rest of the source is
/// parsed once, later.
fn scan_source<'a>(source_id: &'a str, text: &'a str) -> Result<ScannedSource<'a>, String> {
    let lexed = crate::lexer::lex(SourceInput { source_id, text });
    if !lexed.diagnostics.is_empty() {
        return Err(lexed.diagnostics.render(text));
    }
    let paths = qualified_paths(&lexed.tokens);
    let imports = crate::parser::parse_imports(lexed.tokens).map_err(|error| error.render(text))?;
    Ok(ScannedSource { imports, paths })
}

/// Returns every distinct `name::name...` path of `tokens` whose first name starts with a
/// lowercase letter.
fn qualified_paths(tokens: &[Token<'_>]) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    collect_paths(tokens, &mut paths);
    paths
}

/// Adds the paths of `tokens`, including those inside formatted-string interpolations, to
/// `paths`.
fn collect_paths(tokens: &[Token<'_>], paths: &mut Vec<String>) {
    let mut index = 0;
    while index < tokens.len() {
        if let TokenKind::FormattedString(formatted) = &tokens[index].kind {
            for part in &formatted.parts {
                if let FormattedStringPart::Expression {
                    source,
                    start_byte,
                    merged,
                } = part
                {
                    let fragment = crate::lexer::lex_fragment("", source, *start_byte, merged);
                    collect_paths(&fragment.tokens, paths);
                }
            }
        }
        let TokenKind::Identifier(first) = &tokens[index].kind else {
            index += 1;
            continue;
        };
        let mut path = first.clone();
        let mut next = index + 1;
        while let (Some(TokenKind::DoubleColon), Some(TokenKind::Identifier(segment))) = (
            tokens.get(next).map(|token| &token.kind),
            tokens.get(next + 1).map(|token| &token.kind),
        ) {
            path.push_str("::");
            path.push_str(segment);
            next += 2;
        }
        if next > index + 1
            && first.starts_with(|character: char| character.is_ascii_lowercase())
            && !paths.contains(&path)
        {
            paths.push(path);
        }
        index = next;
    }
}

/// Parses one non-root source module, nesting at most `max_nesting` levels deep, and renders
/// lexical or syntax diagnostics.
pub(crate) fn parse_source<'a>(
    source_id: &'a str,
    text: &'a str,
    max_nesting: usize,
) -> Result<Module<'a>, String> {
    let lexed = crate::lexer::lex(SourceInput { source_id, text });
    if !lexed.diagnostics.is_empty() {
        return Err(lexed.diagnostics.render(text));
    }
    crate::parser::parse_with_nesting(source_id, lexed.tokens, false, max_nesting)
        .map_err(|error| error.render(text))
}

/// Checks the `bytes` of source `source_id` against the limits, given the `total` bytes of the
/// sources loaded before it, and returns the new total.
fn check_size(
    source_id: &str,
    bytes: usize,
    total: usize,
    limits: CompileLimits,
) -> Result<usize, String> {
    if bytes > limits.max_source_bytes {
        return Err(format!(
            "{source_id}: source exceeds the limit of {} bytes",
            limits.max_source_bytes
        ));
    }
    let total = total.saturating_add(bytes);
    if total > limits.max_total_source_bytes {
        return Err(format!(
            "{source_id}: the project's sources exceed the limit of {} bytes",
            limits.max_total_source_bytes
        ));
    }
    Ok(total)
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
