//! Integration tests for the public Phase-1 compiler API.

use std::collections::HashMap;

use exs_compiler::{
    CompileOptions, ModuleResolver, ResolvedSource, SourceInput, compile, compile_with_resolver,
    document_with_resolver, format, read_debug_info,
};
use wasmparser::{Parser, Payload, Validator};

#[path = "compiler/continuations.rs"]
mod continuations;
#[path = "compiler/core.rs"]
mod core;
#[path = "compiler/documentation.rs"]
mod documentation;
#[path = "compiler/formatter.rs"]
mod formatter;
#[path = "compiler/imports.rs"]
mod imports;
#[path = "compiler/language.rs"]
mod language;

/// Resolves compiler-test source files from an in-memory canonical source table.
struct TestResolver {
    /// Sources keyed by their canonical identity.
    sources: HashMap<String, String>,
}

impl ModuleResolver for TestResolver {
    /// Resolves a test import through the preconfigured source table.
    fn resolve(&mut self, _importer: &str, path: &str) -> Result<ResolvedSource, String> {
        let source_id = path.to_owned();
        let text = self
            .sources
            .get(&source_id)
            .cloned()
            .ok_or_else(|| format!("missing test source {path}"))?;
        Ok(ResolvedSource { source_id, text })
    }
}
