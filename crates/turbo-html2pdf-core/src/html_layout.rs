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
use crate::node::{Node, Tag};
use crate::style::{build_cascade, style_tree, TokenSet};
use crate::text::FontRegistry;
use crate::{Diagnostics, RenderError};

/// Parse an HTML document/fragment string into the resolved node tree, skipping
/// the Jinja pass. This is the Stage-1 html5ever parse exposed on its own for
/// callers that already have final HTML (see the module docs).
pub fn parse_html(html: &str) -> Result<Vec<Node>, RenderError> {
    crate::template::markup::parse(html)
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
    let nodes = parse_html(html)?;
    let mut author_css = collect_style_css(&nodes);
    author_css.push_str(extra_css);
    let cascade = build_cascade(&author_css, "", TokenSet::default());
    let styled = style_tree(&nodes, &cascade);
    Ok(crate::layout(&styled, cb_width, fonts, diags))
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
