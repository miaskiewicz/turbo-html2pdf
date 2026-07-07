//! Drive a raw HTML string straight into a positioned [`Fragment`] galley,
//! **without** the Jinja templating pass (§1 Stage 1 on its own).
//!
//! The normal entry (`compile` → `Program::render_nodes`) runs the minijinja
//! layer first, which interprets `{{ … }}` / `{% … %}`. That is correct for
//! templates, but wrong for callers that already hold *final* HTML — e.g. a
//! hydrated DOM snapshot from a crawler — where such sequences are page content
//! (inline scripts, JSON, CSS) and must not be evaluated. These helpers skip
//! Jinja and go html5ever-parse → cascade → layout directly.
//!
//! `layout_html` is fully self-contained: it collects the page's own `<style>`
//! blocks as author CSS (the base pipeline applies only inline `style=` +
//! UA defaults), then cascades and lays out at the caller's content width.

use crate::layout::fragment::Fragment;
use crate::layout::ImageCtx;
use crate::node::{Element, Node, Tag};
use crate::style::{build_cascade_with_width, style_tree_with_roots, TokenSet};
use crate::text::FontRegistry;
use crate::{Diagnostics, RenderError};

/// Parse an HTML document/fragment string into the resolved node tree, skipping
/// the Jinja pass. This is the Stage-1 html5ever parse exposed on its own for
/// callers that already have final HTML (see the module docs).
pub fn parse_html(html: &str) -> Result<Vec<Node>, RenderError> {
    crate::template::markup::parse(html)
}

/// Parse `html`, returning the body flow nodes plus the `<html>`/`<body>` ancestor
/// shells the cascade seeds for selector matching (see [`parse_with_roots`]).
pub fn parse_html_with_roots(html: &str) -> Result<(Vec<Node>, Vec<Element>), RenderError> {
    crate::template::markup::parse_with_roots(html)
}

/// Elements whose text content is *not* visible page content and must not be laid
/// out as text (their bodies are CSS/JS/metadata, collected separately or dropped).
fn is_non_visual(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "style" | "script" | "head" | "title" | "meta" | "link" | "noscript" | "template"
    )
}

/// Drop non-visual element subtrees (`<style>`/`<script>`/…) so their bodies don't
/// render as visible text. Author CSS is collected *before* this, so styles still
/// apply; only their raw text is removed from the flow.
fn strip_non_visual(nodes: Vec<Node>) -> Vec<Node> {
    nodes
        .into_iter()
        .filter_map(|node| match node {
            Node::Element(mut el) => {
                if matches!(&el.tag, Tag::Html(name) if is_non_visual(name)) {
                    return None;
                }
                el.children = strip_non_visual(el.children);
                Some(Node::Element(el))
            }
            other => Some(other),
        })
        .collect()
}

/// Concatenate the text of every `<style>` element in a node forest, in
/// document order — the page's author stylesheet. Inline `style=` attributes are
/// applied separately by the cascade, so they are not collected here.
pub fn collect_style_css(nodes: &[Node]) -> String {
    let mut css = String::new();
    collect_style_into(nodes, &mut css);
    css
}

fn collect_style_into(nodes: &[Node], css: &mut String) {
    for node in nodes {
        let Some(el) = node.as_element() else {
            continue;
        };
        if matches!(&el.tag, Tag::Html(name) if name == "style") {
            for child in &el.children {
                if let Some(text) = child.as_text() {
                    css.push_str(text);
                    css.push('\n');
                }
            }
        }
        collect_style_into(&el.children, css);
    }
}

/// Lay a raw HTML string out into a [`Fragment`] galley at content width
/// `cb_width` px, Jinja-free. The page's own `<style>` blocks are collected as
/// author CSS and `extra_css` (UA overrides, a caller reset, etc.) is appended
/// after them so it wins ties. Inline `style=` and the built-in UA defaults
/// apply as in the normal pipeline. `fonts` supplies the faces (use
/// [`FontRegistry::new`] for the bundled set).
pub fn layout_html(
    html: &str,
    extra_css: &str,
    cb_width: f32,
    fonts: &FontRegistry,
    diags: &mut Diagnostics,
) -> Result<Fragment, RenderError> {
    let (nodes, roots) = parse_html_with_roots(html)?;
    let mut author_css = collect_style_css(&nodes);
    author_css.push_str(extra_css);
    let cascade = build_cascade_with_width(&author_css, "", TokenSet::default(), cb_width);
    let styled = style_tree_with_roots(&strip_non_visual(nodes), &cascade, &roots);
    Ok(crate::layout(&styled, cb_width, fonts, diags))
}

/// Like [`layout_html`] but sizes `<img>`/`background-image` boxes against the
/// caller-supplied `images` resolver (see [`crate::layout_with_images`]). For a
/// caller (e.g. turbo-surf's screenshots) that holds final HTML *and* the fetched
/// image bytes: an image is probed for its intrinsic size and laid out as an
/// `Image` fragment the caller then paints. Images the resolver can't supply fall
/// back to the image-free box, exactly as [`layout_html`].
pub fn layout_html_with_images(
    html: &str,
    extra_css: &str,
    cb_width: f32,
    fonts: &FontRegistry,
    images: &ImageCtx,
    diags: &mut Diagnostics,
) -> Result<Fragment, RenderError> {
    let (nodes, roots) = parse_html_with_roots(html)?;
    let mut author_css = collect_style_css(&nodes);
    author_css.push_str(extra_css);
    let cascade = build_cascade_with_width(&author_css, "", TokenSet::default(), cb_width);
    let styled = style_tree_with_roots(&strip_non_visual(nodes), &cascade, &roots);
    Ok(crate::layout_with_images(
        &styled, cb_width, fonts, images, diags,
    ))
}

#[cfg(test)]
mod tests {
    use super::{collect_style_css, parse_html};

    #[test]
    fn parse_html_keeps_body_and_skips_jinja() {
        // Braces are page content, not template syntax — they survive verbatim.
        let nodes = parse_html("<body><p>a {{ x }} b</p></body>").expect("parse");
        assert!(!nodes.is_empty());
        // `collect_style_css` finds a body `<style>` (head is dropped by html5ever).
        let css =
            collect_style_css(&parse_html("<body><style>.a{color:red}</style></body>").unwrap());
        assert!(css.contains(".a{color:red}"));
    }

    #[test]
    fn collect_style_css_empty_without_styles() {
        let nodes = parse_html("<body><div>plain</div></body>").expect("parse");
        assert_eq!(collect_style_css(&nodes), "");
    }
}
