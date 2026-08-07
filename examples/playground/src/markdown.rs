//! Safe Markdown rendering for generated playground documentation.

use exs_compiler::DocumentationPage;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::documentation::resolve_documentation_link;

/// Renders generated Markdown into a restricted, escaped HTML subset for the playground.
pub(crate) fn render_documentation_markdown(
    markdown: &str,
    current_page: &str,
    pages: &[DocumentationPage],
) -> String {
    let parser = Parser::new_ext(
        markdown,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS,
    );
    let mut output = String::new();
    let mut link_ends = Vec::new();
    let mut image_ends = Vec::new();

    for event in parser {
        match event {
            Event::Start(tag) => render_start_tag(
                tag,
                current_page,
                pages,
                &mut output,
                &mut link_ends,
                &mut image_ends,
            ),
            Event::End(tag) => render_end_tag(tag, &mut output, &mut link_ends, &mut image_ends),
            Event::Text(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text)
            | Event::Html(text)
            | Event::InlineHtml(text) => output.push_str(&escape_html(&text)),
            Event::Code(text) => {
                output.push_str("<code>");
                output.push_str(&escape_html(&text));
                output.push_str("</code>");
            }
            Event::FootnoteReference(label) => {
                output.push_str("[^{}");
                output.push_str(&escape_html(&label));
                output.push(']');
            }
            Event::SoftBreak => output.push('\n'),
            Event::HardBreak => output.push_str("<br />"),
            Event::Rule => output.push_str("<hr />"),
            Event::TaskListMarker(checked) => {
                output.push_str(if checked { "[x] " } else { "[ ] " });
            }
        }
    }
    output
}

/// Emits the opening markup for one Markdown tag.
fn render_start_tag(
    tag: Tag<'_>,
    current_page: &str,
    pages: &[DocumentationPage],
    output: &mut String,
    link_ends: &mut Vec<&'static str>,
    image_ends: &mut Vec<&'static str>,
) {
    match tag {
        Tag::Paragraph => output.push_str("<p>"),
        Tag::Heading { level, .. } => output.push_str(heading_start(level)),
        Tag::BlockQuote(_) => output.push_str("<blockquote>"),
        Tag::CodeBlock(CodeBlockKind::Fenced(_)) | Tag::CodeBlock(CodeBlockKind::Indented) => {
            output.push_str("<pre><code>")
        }
        Tag::HtmlBlock => output.push_str("<pre class=\"documentation-raw\">"),
        Tag::List(Some(start)) => output.push_str(&format!("<ol start=\"{start}\">")),
        Tag::List(None) => output.push_str("<ul>"),
        Tag::Item => output.push_str("<li>"),
        Tag::FootnoteDefinition(_) => output.push_str("<aside class=\"documentation-footnote\">"),
        Tag::DefinitionList => output.push_str("<dl>"),
        Tag::DefinitionListTitle => output.push_str("<dt>"),
        Tag::DefinitionListDefinition => output.push_str("<dd>"),
        Tag::Table(_) => output.push_str("<table>"),
        Tag::TableHead => output.push_str("<thead>"),
        Tag::TableRow => output.push_str("<tr>"),
        Tag::TableCell => output.push_str("<td>"),
        Tag::Emphasis => output.push_str("<em>"),
        Tag::Strong => output.push_str("<strong>"),
        Tag::Strikethrough => output.push_str("<s>"),
        Tag::Superscript => output.push_str("<sup>"),
        Tag::Subscript => output.push_str("<sub>"),
        Tag::Link { dest_url, .. } => {
            if let Some(page) = resolve_documentation_link(current_page, &dest_url, pages) {
                output.push_str("<a href=\"#\" data-documentation-page=\"");
                output.push_str(&escape_attribute(&page));
                output.push_str("\">");
                link_ends.push("</a>");
            } else if is_safe_external_link(&dest_url) {
                output.push_str("<a href=\"");
                output.push_str(&escape_attribute(&dest_url));
                output.push_str("\" target=\"_blank\" rel=\"noopener noreferrer\">");
                link_ends.push("</a>");
            } else {
                output.push_str("<span class=\"documentation-link--unresolved\">");
                link_ends.push("</span>");
            }
        }
        Tag::Image { .. } => {
            output.push_str("<span class=\"documentation-image\">");
            image_ends.push("</span>");
        }
        Tag::MetadataBlock(_) => output.push_str("<pre class=\"documentation-metadata\">"),
    }
}

/// Emits the closing markup for one Markdown tag.
fn render_end_tag(
    tag: TagEnd,
    output: &mut String,
    link_ends: &mut Vec<&'static str>,
    image_ends: &mut Vec<&'static str>,
) {
    match tag {
        TagEnd::Paragraph => output.push_str("</p>"),
        TagEnd::Heading(level) => output.push_str(heading_end(level)),
        TagEnd::BlockQuote(_) => output.push_str("</blockquote>"),
        TagEnd::CodeBlock => output.push_str("</code></pre>"),
        TagEnd::HtmlBlock => output.push_str("</pre>"),
        TagEnd::List(true) => output.push_str("</ol>"),
        TagEnd::List(false) => output.push_str("</ul>"),
        TagEnd::Item => output.push_str("</li>"),
        TagEnd::FootnoteDefinition => output.push_str("</aside>"),
        TagEnd::DefinitionList => output.push_str("</dl>"),
        TagEnd::DefinitionListTitle => output.push_str("</dt>"),
        TagEnd::DefinitionListDefinition => output.push_str("</dd>"),
        TagEnd::Table => output.push_str("</table>"),
        TagEnd::TableHead => output.push_str("</thead>"),
        TagEnd::TableRow => output.push_str("</tr>"),
        TagEnd::TableCell => output.push_str("</td>"),
        TagEnd::Emphasis => output.push_str("</em>"),
        TagEnd::Strong => output.push_str("</strong>"),
        TagEnd::Strikethrough => output.push_str("</s>"),
        TagEnd::Superscript => output.push_str("</sup>"),
        TagEnd::Subscript => output.push_str("</sub>"),
        TagEnd::Link => output.push_str(link_ends.pop().unwrap_or("</span>")),
        TagEnd::Image => output.push_str(image_ends.pop().unwrap_or("</span>")),
        TagEnd::MetadataBlock(_) => output.push_str("</pre>"),
    }
}

/// Returns the opening HTML tag for one Markdown heading level.
fn heading_start(level: HeadingLevel) -> &'static str {
    match level {
        HeadingLevel::H1 => "<h1>",
        HeadingLevel::H2 => "<h2>",
        HeadingLevel::H3 => "<h3>",
        HeadingLevel::H4 => "<h4>",
        HeadingLevel::H5 => "<h5>",
        HeadingLevel::H6 => "<h6>",
    }
}

/// Returns the closing HTML tag for one Markdown heading level.
fn heading_end(level: HeadingLevel) -> &'static str {
    match level {
        HeadingLevel::H1 => "</h1>",
        HeadingLevel::H2 => "</h2>",
        HeadingLevel::H3 => "</h3>",
        HeadingLevel::H4 => "</h4>",
        HeadingLevel::H5 => "</h5>",
        HeadingLevel::H6 => "</h6>",
    }
}

/// Returns whether a Markdown link is safe to open outside the playground.
fn is_safe_external_link(destination: &str) -> bool {
    destination.starts_with("https://") || destination.starts_with("http://")
}

/// Escapes text for safe HTML rendering.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escapes one generated attribute value for safe HTML rendering.
fn escape_attribute(value: &str) -> String {
    escape_html(value)
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Verifies that rendering escapes raw HTML and exposes internal document navigation.
#[cfg(test)]
mod tests {
    use exs_compiler::DocumentationPage;

    use super::render_documentation_markdown;

    /// Escapes raw user HTML while rendering internal generated-document links.
    #[test]
    fn renders_safe_internal_documentation_links() {
        let pages = vec![DocumentationPage {
            path: "modules/std/index.md".to_owned(),
            markdown: String::new(),
        }];
        let rendered = render_documentation_markdown(
            "[Standard](modules/std/index.md) <script>alert(1)</script>",
            "modules/00-playground/index.md",
            &pages,
        );
        assert!(rendered.contains("data-documentation-page=\"modules/std/index.md\""));
        assert!(rendered.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }
}
