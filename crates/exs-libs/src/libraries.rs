//! The enabled library modules and their ExS sources.

/// One library module: its path and its ExS source.
struct Library {
    /// Path scripts name the library by, such as `json`.
    path: &'static str,
    /// ExS source of the library module.
    source: &'static str,
}

/// The `json` library, when its feature is enabled.
#[cfg(feature = "json")]
const JSON: Option<Library> = Some(Library {
    path: "json",
    source: crate::json::SOURCE,
});
/// The `json` library, when its feature is enabled.
#[cfg(not(feature = "json"))]
const JSON: Option<Library> = None;

/// The `url` library, when its feature is enabled.
#[cfg(feature = "url")]
const URL: Option<Library> = Some(Library {
    path: "url",
    source: crate::url::SOURCE,
});
/// The `url` library, when its feature is enabled.
#[cfg(not(feature = "url"))]
const URL: Option<Library> = None;

/// The `rand` library, when its feature is enabled.
#[cfg(feature = "rand")]
const RAND: Option<Library> = Some(Library {
    path: "rand",
    source: crate::rand::SOURCE,
});
/// The `rand` library, when its feature is enabled.
#[cfg(not(feature = "rand"))]
const RAND: Option<Library> = None;

/// The `http` library, when its feature is enabled.
#[cfg(feature = "http")]
const HTTP: Option<Library> = Some(Library {
    path: "http",
    source: crate::http::SOURCE,
});
/// The `http` library, when its feature is enabled.
#[cfg(not(feature = "http"))]
const HTTP: Option<Library> = None;

/// Returns every library module enabled by Cargo features.
fn enabled() -> Vec<Library> {
    [JSON, URL, RAND, HTTP].into_iter().flatten().collect()
}

/// Returns the path of every enabled library, for `ModuleResolver::libraries`.
#[must_use]
pub fn libraries() -> Vec<String> {
    enabled()
        .into_iter()
        .map(|library| library.path.to_owned())
        .collect()
}

/// Returns the ExS source of the enabled library at `path`, or `None` when no enabled library
/// has that path.
#[must_use]
pub fn source(path: &str) -> Option<&'static str> {
    enabled()
        .into_iter()
        .find(|library| library.path == path)
        .map(|library| library.source)
}

/// Returns the enabled library at `path` for `ModuleResolver::library`, or `None` when no
/// enabled library has that path.
#[cfg(feature = "compiler")]
#[must_use]
pub fn library(path: &str) -> Option<exs_compiler::ResolvedSource> {
    source(path).map(|text| exs_compiler::ResolvedSource {
        source_id: path.to_owned(),
        text: text.to_owned(),
    })
}
