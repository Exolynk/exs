//! Shared resolver-owned source graph loading for compiler consumers.

use std::collections::HashMap;
use std::path::Path;

use crate::ast::{ImportDeclaration, Module};
use crate::lexer::{FormattedStringPart, Token, TokenKind};
use crate::{CompileError, CompileLimits, ModuleResolver, Source, SourceInput, SourceSpan};

/// A fully resolved project source unit.
pub(crate) struct LoadedSource {
    /// Resolver-provided canonical module identity.
    pub(crate) source_id: String,
    /// Source-defined path displayed by documentation.
    pub(crate) display_path: String,
    /// Complete UTF-8 source text.
    pub(crate) text: String,
    /// The path of the library module this source provides, such as `app::db`, or `None`
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
    /// library, such as `log` in `log::info` or `app::db` in `use app::db::query;`, so
    /// unused libraries cost nothing. Importing a library by its path is an error.
    pub(crate) fn load(
        source: SourceInput<'_>,
        resolver: &dyn ModuleResolver,
        limits: CompileLimits,
    ) -> Result<Self, CompileError> {
        let mut loader = Loader::new(limits);
        loader.add(
            Source::new(source.source_id, source.text),
            root_display_path(source.source_id),
            None,
        )?;
        loader.load_reachable(resolver)
    }

    /// Loads the library modules at `paths`, which must exist, and everything they reach.
    pub(crate) fn load_libraries(
        paths: &[String],
        resolver: &dyn ModuleResolver,
        limits: CompileLimits,
    ) -> Result<Self, CompileError> {
        let mut loader = Loader::new(limits);
        for path in paths {
            if loader.library(resolver, path)?.is_none() {
                return Err(format!(
                    "library `{path}` is listed but the resolver does not provide it"
                )
                .into());
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

    /// Parses every loaded source for one consumer-specific workflow; `require_main` asks the
    /// root source to declare a function.
    pub(crate) fn parse_modules(
        &self,
        require_main: bool,
    ) -> Result<Vec<Module<'_>>, CompileError> {
        self.files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                parse_source(
                    &file.source_id,
                    &file.text,
                    require_main && index == 0,
                    self.max_nesting,
                )
            })
            .collect()
    }

    /// Rejects the loaded graph when any relative-import cycle exists.
    pub(crate) fn validate_no_cycles(&self) -> Result<(), CompileError> {
        let Some(cycle) = find_cycle(&self.edges) else {
            return Ok(());
        };
        let chain = cycle
            .iter()
            .map(|index| self.files[*index].source_id.as_str())
            .collect::<Vec<_>>()
            .join(" -> ");
        let root = &self.files[0];
        Err(CompileError::at(
            "E0233",
            SourceSpan::empty(&root.source_id),
            &root.text,
            format!("import cycle: {chain}"),
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
    fn load_reachable(
        mut self,
        resolver: &dyn ModuleResolver,
    ) -> Result<LoadedProject, CompileError> {
        let mut index = 0;
        while index < self.files.len() {
            let source_id = self.files[index].source_id.clone();
            let text = self.files[index].text.clone();
            let scanned = scan_source(&source_id, &text, self.limits.max_nesting)?;
            for import in scanned.imports {
                if self.library(resolver, &import.path)?.is_some() {
                    return Err(CompileError::at(
                        "E0233",
                        import.span,
                        &text,
                        format!(
                            "`{}` is a library and needs no import; remove the import and name it as `{}::...` or bring items into scope with `use`",
                            import.path, import.path
                        ),
                    ));
                }
                let resolved = resolver
                    .resolve(&source_id, &import.path)
                    .map_err(|error| {
                        CompileError::at(
                            "E0233",
                            import.span,
                            &text,
                            format!("could not resolve import `{}`: {error}", import.path),
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
        source: Source<'_>,
        display_path: String,
        library: Option<String>,
    ) -> Result<usize, CompileError> {
        if let Some(index) = self.indices.get(source.id.as_ref()) {
            return Ok(*index);
        }
        if self.files.len() >= self.limits.max_modules {
            let (source_id, text) = self
                .files
                .first()
                .map_or((source.id.as_ref(), source.text.as_ref()), |root| {
                    (root.source_id.as_str(), root.text.as_str())
                });
            return Err(CompileError::at(
                "E0010",
                SourceSpan::empty(source_id),
                text,
                format!(
                    "the project exceeds the limit of {} modules",
                    self.limits.max_modules
                ),
            ));
        }
        self.total_bytes = check_size(&source.id, &source.text, self.total_bytes, self.limits)?;
        let index = self.files.len();
        self.indices.insert(source.id.clone().into_owned(), index);
        self.files.push(LoadedSource {
            source_id: source.id.into_owned(),
            display_path,
            text: source.text.into_owned(),
            library,
        });
        self.edges.push(Vec::new());
        Ok(index)
    }

    /// Loads the library module at `path` if the resolver provides one, asking it once per
    /// path, and returns its index.
    fn library(
        &mut self,
        resolver: &dyn ModuleResolver,
        path: &str,
    ) -> Result<Option<usize>, CompileError> {
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
    /// such as `app::db::query`; library names are lowercase.
    paths: Vec<String>,
}

/// Scans one source for its imports and the qualified paths it names. A source with lexical
/// errors fails with them and its syntax errors, one with invalid imports with their syntax
/// errors; the rest of a valid source is parsed once, later.
fn scan_source<'a>(
    source_id: &'a str,
    text: &'a str,
    max_nesting: usize,
) -> Result<ScannedSource<'a>, CompileError> {
    let lexed = crate::lexer::lex(SourceInput { source_id, text });
    if !lexed.diagnostics.is_empty() {
        let lexical = CompileError::from_diagnostics(&lexed.diagnostics, text);
        return Err(parse_tokens(source_id, text, lexed, false, max_nesting)
            .err()
            .unwrap_or(lexical));
    }
    let paths = qualified_paths(&lexed.tokens);
    let imports = crate::parser::parse_imports(lexed.tokens)
        .map_err(|error| CompileError::from_diagnostics(&error, text))?;
    Ok(ScannedSource { imports, paths })
}

/// Returns every distinct `name::name...` path of `tokens` whose first name starts with a
/// lowercase letter, and the single names of `use name;` and `use name::{...};`.
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
        // `use app::{Record};` and `use log;` name a library by a single segment.
        let names_group = matches!(
            (
                tokens.get(next).map(|token| &token.kind),
                tokens.get(next + 1).map(|token| &token.kind),
            ),
            (Some(TokenKind::DoubleColon), Some(TokenKind::LeftBrace))
        );
        let used = index > 0 && tokens[index - 1].kind == TokenKind::Use;
        if (next > index + 1 || names_group || used)
            && first.starts_with(|character: char| character.is_ascii_lowercase())
            && !paths.contains(&path)
        {
            paths.push(path);
        }
        index = next;
    }
}

/// Parses one source module, nesting at most `max_nesting` levels deep; `require_main` asks it
/// to declare a function.
pub(crate) fn parse_source<'a>(
    source_id: &'a str,
    text: &'a str,
    require_main: bool,
    max_nesting: usize,
) -> Result<Module<'a>, CompileError> {
    let lexed = crate::lexer::lex(SourceInput { source_id, text });
    parse_tokens(source_id, text, lexed, require_main, max_nesting)
}

/// Parses the tokens of `lexed`, failing with its lexical errors and the syntax errors in
/// source order.
fn parse_tokens<'a>(
    source_id: &'a str,
    text: &'a str,
    lexed: crate::lexer::Lexed<'a>,
    require_main: bool,
    max_nesting: usize,
) -> Result<Module<'a>, CompileError> {
    let mut diagnostics = lexed.diagnostics;
    let parsed =
        crate::parser::parse_with_nesting(source_id, lexed.tokens, require_main, max_nesting);
    let module = match parsed {
        Ok(module) => Some(module),
        Err(syntax) => {
            diagnostics.extend(syntax);
            None
        }
    };
    match module {
        Some(module) if diagnostics.is_empty() => Ok(module),
        _ => {
            diagnostics.sort_by_span();
            Err(CompileError::from_diagnostics(&diagnostics, text))
        }
    }
}

/// Checks the source `text` of `source_id` against the limits, given the `total` bytes of the
/// sources loaded before it, and returns the new total.
fn check_size(
    source_id: &str,
    text: &str,
    total: usize,
    limits: CompileLimits,
) -> Result<usize, CompileError> {
    if text.len() > limits.max_source_bytes {
        return Err(CompileError::at(
            "E0010",
            SourceSpan::empty(source_id),
            text,
            format!(
                "source exceeds the limit of {} bytes",
                limits.max_source_bytes
            ),
        ));
    }
    let total = total.saturating_add(text.len());
    if total > limits.max_total_source_bytes {
        return Err(CompileError::at(
            "E0010",
            SourceSpan::empty(source_id),
            text,
            format!(
                "the project's sources exceed the limit of {} bytes",
                limits.max_total_source_bytes
            ),
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
