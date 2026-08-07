//! Generated documentation selection for the browser playground.

use exs_compiler::{
    DocumentationPage, ModuleResolver, ResolvedSource, SourceInput, document_with_resolver,
};

/// Stable source identifier used for editor diagnostics and generated documentation paths.
pub(crate) const PLAYGROUND_SOURCE_ID: &str = "playground.exs";

/// Resolves no imports because browser playground sources have no filesystem access.
struct PlaygroundDocumentationResolver;

impl ModuleResolver for PlaygroundDocumentationResolver {
    /// Reports that relative imports cannot be loaded in the browser playground.
    fn resolve(&mut self, _importer: &str, _path: &str) -> Result<ResolvedSource, String> {
        Err(String::from(
            "imports are unavailable in the browser playground documentation view",
        ))
    }
}

/// Generates standard-library and active-source documentation pages for the documentation pane.
pub(crate) fn documentation_pages(source: &str) -> Result<Vec<DocumentationPage>, String> {
    let mut resolver = PlaygroundDocumentationResolver;
    let mut pages = document_with_resolver(
        SourceInput {
            source_id: PLAYGROUND_SOURCE_ID,
            text: source,
        },
        &mut resolver,
    )?
    .pages;
    pages.sort_by(|left, right| {
        let left_is_standard = left.path.starts_with("modules/std/");
        let right_is_standard = right.path.starts_with("modules/std/");
        right_is_standard
            .cmp(&left_is_standard)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(pages)
}

/// Resolves one generated Markdown link to another generated documentation page.
pub(crate) fn resolve_documentation_link(
    current_page: &str,
    destination: &str,
    pages: &[DocumentationPage],
) -> Option<String> {
    let destination = destination.split('#').next().unwrap_or_default();
    if destination.is_empty() || destination.contains("://") || destination.starts_with("mailto:") {
        return None;
    }
    let mut components = if destination.starts_with("modules/") {
        Vec::new()
    } else {
        current_page
            .split('/')
            .filter(|component| !component.is_empty())
            .collect::<Vec<_>>()
    };
    if !destination.starts_with("modules/") {
        let _file_name = components.pop();
    }
    for component in destination.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                let _parent = components.pop();
            }
            component => components.push(component),
        }
    }
    let candidate = components.join("/");
    pages
        .iter()
        .find(|page| page.path == candidate)
        .map(|page| page.path.clone())
}

/// Verifies that generated playground documentation includes runtime and user declarations.
#[cfg(test)]
mod tests {
    use exs_compiler::DocumentationPage;

    use super::{documentation_pages, resolve_documentation_link};

    /// Includes standard-library pages and declarations from the active editor source.
    #[test]
    fn documents_standard_library_and_current_source() {
        let pages = match documentation_pages(
            "/// Returns a greeting.\nfn greet(name: String) -> String { ret name; }",
        ) {
            Ok(pages) => pages,
            Err(error) => panic!("documentation generation failed: {error}"),
        };
        assert!(pages.iter().any(|page| page.path == "modules/std/index.md"));
        assert!(pages.iter().any(|page| page.path.ends_with("/fn/greet.md")));
    }

    /// Resolves generated documentation links relative to the current page.
    #[test]
    fn resolves_relative_documentation_links() {
        let pages = vec![DocumentationPage {
            path: "modules/std/types/int.md".to_owned(),
            markdown: String::new(),
        }];
        assert_eq!(
            resolve_documentation_link("modules/std/index.md", "types/int.md", &pages),
            Some(String::from("modules/std/types/int.md"))
        );
    }
}
