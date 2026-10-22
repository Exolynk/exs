//! Renders the documentation model as Markdown pages.

use super::model::{
    ApiDocumentation, ItemDocs, ItemKind, ItemRef, MemberDocs, ModuleDocs, SectionDocs,
};
use super::shared::slug;
use crate::{Documentation, DocumentationPage};

/// How declarations are spread over pages.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum PageLayout {
    /// One page per module that documents every declaration of the module.
    #[default]
    PerModule,
    /// One page per module that documents its functions and links to one page for each of
    /// its types, enums, traits, and namespaces.
    PerItem,
}

/// What a generated page documents.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PageKind {
    /// The overview of every module.
    Index,
    /// One module.
    Module,
    /// One declaration of the given kind.
    Item(ItemKind),
}

/// What a front-matter callback learns about one page.
#[derive(Debug, Clone, Copy)]
pub struct PageInfo<'a> {
    /// Path of the page within the generated output.
    pub path: &'a str,
    /// Plain title: the module name, the declaration name, or `ExS API Documentation`.
    pub title: &'a str,
    /// What the page documents.
    pub kind: PageKind,
    /// The module the page belongs to; `None` for the index.
    pub module: Option<&'a ModuleDocs>,
    /// The declaration the page documents, for item pages.
    pub item: Option<&'a ItemDocs>,
}

/// Options of the Markdown renderer.
#[derive(Clone, Copy)]
pub struct MarkdownOptions<'a> {
    /// How declarations are spread over pages.
    pub layout: PageLayout,
    /// File name of the index page of the output and of every module directory, such as
    /// `index.md`, or `_index.md` for Zola sections.
    pub index_file: &'a str,
    /// Prefix of absolute links, such as `@/exs/` for Zola internal links; links are relative
    /// to the linking page when `None`.
    pub link_prefix: Option<&'a str>,
    /// Whether a page starts with a `#` heading naming its subject; front matter usually
    /// carries the title instead.
    pub title_heading: bool,
    /// Returns the front matter written before each page, including its delimiters.
    pub front_matter: Option<&'a dyn Fn(&PageInfo<'_>) -> String>,
}

impl Default for MarkdownOptions<'_> {
    fn default() -> Self {
        Self {
            layout: PageLayout::PerModule,
            index_file: "index.md",
            link_prefix: None,
            title_heading: true,
            front_matter: None,
        }
    }
}

/// Title of the index page.
const INDEX_TITLE: &str = "ExS API Documentation";

/// Renders `documentation` as Markdown pages.
pub(crate) fn render(
    documentation: &ApiDocumentation,
    options: &MarkdownOptions<'_>,
) -> Documentation {
    let renderer = Renderer {
        documentation,
        options,
    };
    let mut pages = vec![renderer.index()];
    for module in &documentation.modules {
        pages.push(renderer.module(module));
        if options.layout == PageLayout::PerItem {
            for item in module.items.iter().filter(|item| has_own_page(item)) {
                pages.push(renderer.item_page(module, item));
            }
        }
    }
    Documentation { pages }
}

/// Reports whether `item` gets its own page in the per-item layout.
fn has_own_page(item: &ItemDocs) -> bool {
    item.kind != ItemKind::Function
}

/// Returns the first sentence of `description`.
fn summary(description: &str) -> &str {
    let line = description.lines().next().unwrap_or_default();
    line.find(". ").map_or(line, |end| &line[..=end])
}

/// Returns the anchor Markdown renderers such as GitHub and Zola derive from a heading.
fn anchor(heading: &str) -> String {
    let mut anchor = String::new();
    for character in heading.chars() {
        if character.is_alphanumeric() || character == '_' {
            anchor.extend(character.to_lowercase());
        } else if (character == ' ' || character == '-') && !anchor.ends_with('-') {
            anchor.push('-');
        }
    }
    anchor.trim_matches('-').to_owned()
}

/// Returns the heading of `item` on a module page.
fn item_heading(item: &ItemDocs) -> String {
    if item.kind == ItemKind::Function {
        format!("`{}`", item.name)
    } else {
        format!("{} `{}`", item.kind.label(), item.name)
    }
}

/// Renders the documentation of one model with fixed options.
struct Renderer<'d, 'o> {
    /// The documentation to render.
    documentation: &'d ApiDocumentation,
    /// The rendering options.
    options: &'d MarkdownOptions<'o>,
}

impl Renderer<'_, '_> {
    /// Returns the path of the page of `module`.
    fn module_path(&self, module: &ModuleDocs) -> String {
        format!("{}/{}", module.segments.join("/"), self.options.index_file)
    }

    /// Returns the path of the page of `item` in the per-item layout.
    fn item_path(module: &ModuleDocs, item: &ItemDocs) -> String {
        format!("{}/{}.md", module.segments.join("/"), slug(&item.name))
    }

    /// Returns the link from the page at `from` to the target `to`, which may carry an
    /// anchor.
    fn link(&self, from: &str, to: &str) -> String {
        if let Some(prefix) = self.options.link_prefix {
            return format!("{prefix}{to}");
        }
        let (target, fragment) = to
            .split_once('#')
            .map_or((to, ""), |(path, anchor)| (path, anchor));
        if target == from {
            return format!("#{fragment}");
        }
        let from = from.split('/').collect::<Vec<_>>();
        let target = target.split('/').collect::<Vec<_>>();
        let directories = &from[..from.len().saturating_sub(1)];
        let common = directories
            .iter()
            .zip(&target)
            .take_while(|(left, right)| left == right)
            .count();
        let mut parts = vec![".."; directories.len() - common];
        parts.extend(&target[common..]);
        let mut link = parts.join("/");
        if !fragment.is_empty() {
            link.push('#');
            link.push_str(fragment);
        }
        link
    }

    /// Returns the target of a link to `reference`, if the documentation contains it.
    fn target(&self, reference: &ItemRef) -> Option<String> {
        let module = self.documentation.module(&reference.module)?;
        let item = module.item(&reference.name)?;
        if self.options.layout == PageLayout::PerItem && has_own_page(item) {
            Some(Self::item_path(module, item))
        } else {
            Some(format!(
                "{}#{}",
                self.module_path(module),
                anchor(&item_heading(item))
            ))
        }
    }

    /// Returns `text` with the first code span naming the referenced item linked to it.
    fn linked(&self, from: &str, text: &str, reference: Option<&ItemRef>) -> String {
        let Some(reference) = reference else {
            return text.to_owned();
        };
        let Some(target) = self.target(reference) else {
            return text.to_owned();
        };
        let span = format!("`{}`", reference.name);
        text.replacen(&span, &format!("[{span}]({})", self.link(from, &target)), 1)
    }

    /// Returns the page at `path` with its front matter and optional title heading.
    fn page(&self, info: PageInfo<'_>, heading: &str, body: String) -> DocumentationPage {
        let mut markdown = self
            .options
            .front_matter
            .map(|front_matter| front_matter(&info))
            .unwrap_or_default();
        if self.options.title_heading {
            markdown.push_str(&format!("# {heading}\n\n"));
        }
        markdown.push_str(body.trim_start());
        if !markdown.ends_with('\n') {
            markdown.push('\n');
        }
        DocumentationPage {
            path: info.path.to_owned(),
            markdown,
        }
    }

    /// Renders the index page of every module.
    fn index(&self) -> DocumentationPage {
        let path = self.options.index_file.to_owned();
        let mut body = String::from(
            "ExS is a dynamically typed scripting language compiled to WebAssembly. This reference documents the standard library, the library modules every script may name without importing them, and project modules. Adjacent `///` comments describe declarations and leading `//!` comments describe modules.\n\n## Language\n\nRoot modules declare `fn main(...)`; other modules provide functions, nominal types, enums, traits, and implementations. Library items are named by their path, such as `log::info(...)`, or brought into scope with `use log::info;`. `Host::call(name, args...)` invokes a runner-provided host function and may suspend.\n\n## Modules\n\n",
        );
        for module in &self.documentation.modules {
            let summary = summary(&module.description);
            body.push_str(&format!(
                "- [`{}`]({})",
                module.name,
                self.link(&path, &self.module_path(module))
            ));
            if !summary.is_empty() {
                body.push_str(&format!(" - {summary}"));
            }
            body.push('\n');
        }
        self.page(
            PageInfo {
                path: &path,
                title: INDEX_TITLE,
                kind: PageKind::Index,
                module: None,
                item: None,
            },
            INDEX_TITLE,
            body,
        )
    }

    /// Renders the page of one module.
    fn module(&self, module: &ModuleDocs) -> DocumentationPage {
        let path = self.module_path(module);
        let mut body = String::new();
        if !module.description.is_empty() {
            body.push_str(&module.description);
            body.push_str("\n\n");
        }
        let submodules = self
            .documentation
            .modules
            .iter()
            .filter(|candidate| {
                candidate.segments.len() == module.segments.len() + 1
                    && candidate.segments.starts_with(&module.segments)
            })
            .collect::<Vec<_>>();
        if !submodules.is_empty() {
            body.push_str("## Modules\n\n");
            for submodule in submodules {
                body.push_str(&format!(
                    "- [`{}`]({})\n",
                    submodule.name,
                    self.link(&path, &self.module_path(submodule))
                ));
            }
            body.push('\n');
        }
        match self.options.layout {
            PageLayout::PerItem => {
                for (kind, heading) in [
                    (ItemKind::Type, "Types"),
                    (ItemKind::Enum, "Enums"),
                    (ItemKind::Trait, "Traits"),
                    (ItemKind::Namespace, "Namespaces"),
                ] {
                    let items = module
                        .items
                        .iter()
                        .filter(|item| item.kind == kind)
                        .collect::<Vec<_>>();
                    if items.is_empty() {
                        continue;
                    }
                    body.push_str(&format!("## {heading}\n\n"));
                    for item in items {
                        body.push_str(&format!(
                            "- [`{}`]({})\n",
                            item.name,
                            self.link(&path, &Self::item_path(module, item))
                        ));
                    }
                    body.push('\n');
                }
            }
            PageLayout::PerModule => {
                for item in module
                    .items
                    .iter()
                    .filter(|item| item.kind != ItemKind::Function)
                {
                    body.push_str(&format!("## {}\n\n", item_heading(item)));
                    self.item_body(&mut body, &path, item, 2);
                }
            }
        }
        let functions = module
            .items
            .iter()
            .filter(|item| item.kind == ItemKind::Function)
            .collect::<Vec<_>>();
        if !functions.is_empty() {
            body.push_str("## Functions\n\n");
            for item in functions {
                body.push_str(&format!("### {}\n\n", item_heading(item)));
                self.item_body(&mut body, &path, item, 3);
            }
        }
        if module.items.is_empty() {
            body.push_str("No public declarations.\n");
        }
        self.page(
            PageInfo {
                path: &path,
                title: &module.name,
                kind: PageKind::Module,
                module: Some(module),
                item: None,
            },
            &format!("Module `{}`", module.name),
            body,
        )
    }

    /// Renders the own page of one item in the per-item layout.
    fn item_page(&self, module: &ModuleDocs, item: &ItemDocs) -> DocumentationPage {
        let path = Self::item_path(module, item);
        let mut body = String::new();
        self.item_body(&mut body, &path, item, 1);
        self.page(
            PageInfo {
                path: &path,
                title: &item.name,
                kind: PageKind::Item(item.kind),
                module: Some(module),
                item: Some(item),
            },
            &item_heading(item),
            body,
        )
    }

    /// Appends the documentation of `item`, whose heading has `level`, to `output`.
    fn item_body(&self, output: &mut String, path: &str, item: &ItemDocs, level: usize) {
        if !item.description.is_empty() {
            output.push_str(&item.description);
            output.push_str("\n\n");
        }
        if !item.declaration.is_empty() {
            output.push_str(&format!("```exs\n{}\n```\n\n", item.declaration));
        }
        if let Some(usage) = &item.usage {
            output.push_str(&format!(
                "{} Usage\n\n```exs\n{usage}\n```\n\n",
                "#".repeat(level + 1)
            ));
        }
        let mut group: Option<&str> = None;
        for section in &item.sections {
            let section_level = match section.group.as_deref() {
                Some(name) => {
                    if group != Some(name) {
                        output.push_str(&format!("{} {name}\n\n", "#".repeat(level + 1)));
                    }
                    level + 2
                }
                None => level + 1,
            };
            group = section.group.as_deref();
            self.section(output, path, section, section_level);
        }
    }

    /// Appends one section whose heading has `level` to `output`.
    fn section(&self, output: &mut String, path: &str, section: &SectionDocs, level: usize) {
        output.push_str(&format!(
            "{} {}\n\n",
            "#".repeat(level),
            self.linked(path, &section.title, section.link.as_ref())
        ));
        if !section.description.is_empty() {
            output.push_str(&section.description);
            output.push_str("\n\n");
        }
        for reference in &section.references {
            let name = format!("`{}`", reference.name);
            match self.target(reference) {
                Some(target) => {
                    output.push_str(&format!("- [{name}]({})\n", self.link(path, &target)))
                }
                None => output.push_str(&format!("- {name}\n")),
            }
        }
        if !section.references.is_empty() {
            output.push('\n');
        }
        for member in &section.members {
            Self::member(output, member, level + 1);
        }
    }

    /// Appends one member whose heading has `level` to `output`.
    fn member(output: &mut String, member: &MemberDocs, level: usize) {
        output.push_str(&format!("{} `{}`\n\n", "#".repeat(level), member.name));
        if !member.description.is_empty() {
            output.push_str(&member.description);
            output.push_str("\n\n");
        }
        if !member.declaration.is_empty() && member.declaration != member.name {
            output.push_str(&format!("```exs\n{}\n```\n\n", member.declaration));
        }
        if let Some(example) = &member.example {
            output.push_str(&format!("```exs\n{example}\n```\n\n"));
        }
    }
}
