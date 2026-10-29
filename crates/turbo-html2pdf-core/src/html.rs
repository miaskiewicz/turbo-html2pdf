//! The HTML drive in html2pdf mode: raw HTML (no templating) straight to a
//! galley or to `data-cid` boxes, with the `t:` directives classified exactly as
//! in a template render — `<t:…/>` expanded, an unknown `t:` name rejected — and
//! this build's `xref`/`print-color` switches. It is turbo-dom-layout's HTML drive
//! (`turbo_dom_layout::html_layout`, whose string entries know no directives) over
//! the parsed document.

use turbo_dom_layout::{CidBox, Fragment, HtmlOptions};

use crate::error::{Diagnostics, RenderError};
use crate::flow::Flow;
use crate::layout::ImageCtx;
use crate::template::markup;
use crate::text::FontRegistry;

/// This build's HTML-drive options: `t:` markers, plus `xref` / `print-color`.
fn options() -> HtmlOptions<'static> {
    HtmlOptions {
        markers: Some(&crate::markers::classify),
        atomic_internal_links: cfg!(feature = "xref"),
        accept_cmyk: cfg!(feature = "print-color"),
        linked_css: "",
    }
}

/// Parse an HTML document/fragment string into a [`Flow`] (the parsed turbo-dom
/// document and its `<body>` content, `t:` directives classified as markers),
/// skipping the Jinja pass.
pub fn parse_html(html: &str) -> Result<Flow, RenderError> {
    markup::parse(html)
}

/// Lay a raw HTML string out into a [`Fragment`] galley at content width
/// `cb_width` px, Jinja-free (see [`turbo_dom_layout::layout_html`]: the whole
/// document is styled — head and body `<style>` blocks are author CSS, `extra_css` is
/// appended after them — with `<html>`/`<body>` as real boxes).
pub fn layout_html(
    html: &str,
    extra_css: &str,
    cb_width: f32,
    fonts: &FontRegistry,
    diags: &mut Diagnostics,
) -> Result<Fragment, RenderError> {
    let images = ImageCtx {
        resolver: &crate::NoImages,
        body_height: None,
    };
    layout_html_with_images(html, extra_css, cb_width, fonts, &images, diags)
}

/// Like [`layout_html`] but sizes `<img>`/`background-image` boxes against the
/// caller-supplied `images` resolver.
pub fn layout_html_with_images(
    html: &str,
    extra_css: &str,
    cb_width: f32,
    fonts: &FontRegistry,
    images: &ImageCtx,
    diags: &mut Diagnostics,
) -> Result<Fragment, RenderError> {
    let tree = markup::parse_tree(html)?;
    Ok(turbo_dom_layout::layout_html_tree(
        &tree,
        extra_css,
        cb_width,
        fonts,
        images,
        &options(),
        diags,
    ))
}

/// Lay `html` out (Jinja-free, exactly as [`layout_html`]) and return the placed
/// border-box geometry of every box whose source element carries `data-cid` (see
/// [`turbo_dom_layout::layout_boxes`]) — the conformance seam.
pub fn layout_boxes(
    html: &str,
    extra_css: &str,
    cb_width: f32,
    cb_height: f32,
    fonts: &FontRegistry,
    diags: &mut Diagnostics,
) -> Result<Vec<CidBox>, RenderError> {
    let tree = markup::parse_tree(html)?;
    Ok(turbo_dom_layout::layout_boxes_tree(
        &tree,
        extra_css,
        cb_width,
        cb_height,
        fonts,
        &options(),
        diags,
    ))
}
