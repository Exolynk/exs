//! The sources a compilation works on.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

use crate::{ModuleResolver, SourceInput};

/// One ExS source: its identity, used in diagnostics, and its text.
///
/// Both may be borrowed or owned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source<'a> {
    /// The human-readable source identity used in diagnostics.
    pub id: Cow<'a, str>,
    /// UTF-8 ExS source text.
    pub text: Cow<'a, str>,
}

impl<'a> Source<'a> {
    /// Returns the source `text` identified by `id`.
    #[must_use]
    pub fn new(id: impl Into<Cow<'a, str>>, text: impl Into<Cow<'a, str>>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
        }
    }

    /// Returns the borrowed form the compiler passes work on.
    pub(crate) fn input(&self) -> SourceInput<'_> {
        SourceInput {
            source_id: &self.id,
            text: &self.text,
        }
    }
}

/// A resolver over sources known up front: files to import and library modules.
///
/// ```
/// use exs_compiler::{CompileOptions, Source, Sources, compile};
///
/// let mut sources = Sources::new();
/// sources.add("./math.exs", "fn answer() -> Int { ret 42; }");
/// sources.add_library("log", "fn info(value) -> Any { ret value; }");
/// let source = Source::new(
///     "app.exs",
///     r#"import "./math.exs" as math; fn main() { ret log::info(math::answer()); }"#,
/// );
/// assert!(compile(source, CompileOptions::default().resolver(&sources)).is_ok());
/// ```
#[derive(Debug, Clone, Default)]
pub struct Sources {
    /// Text of the importable files by normalized path.
    files: HashMap<String, String>,
    /// Text of the library modules by path.
    libraries: BTreeMap<String, String>,
}

impl Sources {
    /// Creates an empty collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the importable file at `path`, relative to the entry source, replacing an earlier
    /// file at that path.
    pub fn add(&mut self, path: impl AsRef<str>, text: impl Into<String>) -> &mut Self {
        self.files.insert(normalize("", path.as_ref()), text.into());
        self
    }

    /// Adds the library module at `path`, such as `log` or `app::db`, replacing an earlier
    /// module at that path.
    pub fn add_library(&mut self, path: impl Into<String>, text: impl Into<String>) -> &mut Self {
        self.libraries.insert(path.into(), text.into());
        self
    }
}

impl ModuleResolver for Sources {
    fn resolve(&self, importer: &str, path: &str) -> Result<Source<'_>, String> {
        let directory = importer
            .rsplit_once('/')
            .map_or("", |(directory, _)| directory);
        let file = normalize(directory, path);
        match self.files.get_key_value(&file) {
            Some((id, text)) => Ok(Source::new(id.as_str(), text.as_str())),
            None => Err(format!("no source at `{file}`")),
        }
    }

    fn library(&self, path: &str) -> Option<Source<'_>> {
        let (id, text) = self.libraries.get_key_value(path)?;
        Some(Source::new(id.as_str(), text.as_str()))
    }

    fn libraries(&self) -> Vec<String> {
        self.libraries.keys().cloned().collect()
    }
}

/// Returns `path` relative to `directory` without `.` and resolved `..` segments.
fn normalize(directory: &str, path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for segment in directory.split('/').chain(path.split('/')) {
        match segment {
            "" | "." => {}
            ".." if segments.last().is_some_and(|last| *last != "..") => {
                segments.pop();
            }
            segment => segments.push(segment),
        }
    }
    segments.join("/")
}
