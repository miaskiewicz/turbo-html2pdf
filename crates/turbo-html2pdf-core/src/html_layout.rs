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

use std::collections::HashMap;

use crate::layout::boxgen::{build_box_tree, BoxKind, InlineItem, LayoutBox};
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

/// One laid-out box paired with the `data-cid` its source element carried
/// ([`layout_boxes`]). Coordinates are absolute px in the galley's top-down
/// space (a fragment's `x`/`y` are already accumulated to page origin), and the
/// size is the border box — the same rectangle a browser's
/// `getBoundingClientRect()` reports — so the two can be compared directly.
#[derive(Debug, Clone, PartialEq)]
pub struct CidBox {
    /// The `data-cid` attribute value of the source element.
    pub cid: String,
    /// Left edge, px from the page origin.
    pub x: f32,
    /// Top edge, px from the page origin.
    pub y: f32,
    /// Border-box width, px.
    pub width: f32,
    /// Border-box height, px.
    pub height: f32,
}

/// Lay `html` out (Jinja-free, exactly as [`layout_html`]) and return the placed
/// geometry of every box whose SOURCE element carries a `data-cid="..."`
/// attribute — a debug/conformance seam, **not** part of the render path.
///
/// The box tree stamps each box a pre-order [`NodeId`](crate::NodeId) alongside
/// the source element's `attrs`, so we build the box tree once to map
/// `node_id → data-cid`, then walk the laid-out [`Fragment`] galley and emit the
/// first (outermost, border-box) fragment for each mapped id. The map is exact —
/// no document-order fallback is needed — with one honest limitation: a
/// `display:inline` element is flattened into anonymous inline runs during box
/// generation and does not keep its own box/attrs, so a `data-cid` is only
/// resolved on box-generating elements (block, `inline-block`, flex/grid/table
/// items, replaced `<img>`). Tag the box-generating element under test.
///
/// `cb_height` bounds the (here absent) image height cap only; the galley height
/// is otherwise unbounded, so a percentage height against the root resolves to 0
/// as in the normal no-geometry layout entry.
pub fn layout_boxes(
    html: &str,
    extra_css: &str,
    cb_width: f32,
    cb_height: f32,
    fonts: &FontRegistry,
    diags: &mut Diagnostics,
) -> Result<Vec<CidBox>, RenderError> {
    let (nodes, roots) = parse_html_with_roots(html)?;
    let mut author_css = collect_style_css(&nodes);
    author_css.push_str(extra_css);
    let cascade = build_cascade_with_width(&author_css, "", TokenSet::default(), cb_width);
    let styled = style_tree_with_roots(&strip_non_visual(nodes), &cascade, &roots);

    // `build_box_tree` is deterministic (monotonic pre-order ids), so the ids in
    // this tree match the ones `crate::layout` stamps on the fragments below.
    let mut cids: HashMap<u32, String> = HashMap::new();
    collect_cids(&build_box_tree(&styled), &mut cids);

    let ctx = ImageCtx {
        resolver: &crate::image::NoImages,
        body_height: Some(cb_height),
    };
    let root = crate::layout_with_images(&styled, cb_width, fonts, &ctx, diags);

    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    collect_boxes(&root, &cids, &mut seen, &mut out);
    Ok(out)
}

/// Walk the box tree, recording `node_id → data-cid` for every box that carries
/// one. Anonymous boxes and flattened inline runs have empty `attrs`, so they
/// contribute nothing (see [`layout_boxes`]).
fn collect_cids(bx: &LayoutBox, out: &mut HashMap<u32, String>) {
    if let Some(cid) = bx.attrs.iter().find(|a| a.name == "data-cid") {
        out.insert(bx.node_id.0, cid.value.clone());
    }
    match &bx.kind {
        BoxKind::Lines(items) => collect_line_atoms(items, out),
        other => {
            for c in block_children(other) {
                collect_cids(c, out);
            }
        }
    }
}

/// The block-level children of a box (`Block`/`Flex`/`Grid`/`Table`); empty for a
/// `Lines`/`Directive` box.
fn block_children(kind: &BoxKind) -> &[LayoutBox] {
    match kind {
        BoxKind::Block(k) | BoxKind::Flex(k) | BoxKind::Grid(k) | BoxKind::Table(k) => k,
        _ => &[],
    }
}

/// Recurse into the atomic (`inline-block`) boxes of an inline `Lines` run.
fn collect_line_atoms(items: &[InlineItem], out: &mut HashMap<u32, String>) {
    for it in items {
        if let InlineItem::Atomic(b) = it {
            collect_cids(b, out);
        }
    }
}

/// Pre-order walk the fragment galley, emitting the FIRST fragment seen for each
/// mapped `node_id` — the outermost (border-box) fragment, since a parent is
/// visited before its background-fill/content children that reuse the same id.
fn collect_boxes(
    frag: &Fragment,
    cids: &HashMap<u32, String>,
    seen: &mut std::collections::HashSet<u32>,
    out: &mut Vec<CidBox>,
) {
    let id = frag.node_id.0;
    if let Some(cid) = cids.get(&id) {
        if seen.insert(id) {
            out.push(CidBox {
                cid: cid.clone(),
                x: frag.x,
                y: frag.y,
                width: frag.width,
                height: frag.height,
            });
        }
    }
    for c in &frag.children {
        collect_boxes(c, cids, seen, out);
    }
}

#[cfg(test)]
mod tests {
    use super::{collect_style_css, layout_boxes, parse_html};
    use crate::text::FontRegistry;
    use crate::Diagnostics;

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

    #[test]
    fn box_sizing_inherit_chains_from_the_html_root() {
        // The classic reset `html{box-sizing:border-box}` + `*{box-sizing:inherit}`.
        // The `<html>`/`<body>` shells are match-only, so their computed style must be
        // threaded as the inheritance parent — else `box-sizing:inherit` resolves to
        // nothing and each `width:50%` card becomes content-box + padding (320px), so
        // two no longer fit their 600px row and the second wraps (nike's half-width
        // hero stacking). With the border-box chain each card is 300px and both fit.
        use crate::layout::fragment::FragmentContent;
        use crate::text::FontRegistry;
        let html = r#"<html><body><div class="row"><span class="c">a</span><span class="c">b</span></div></body></html>"#;
        // `.c` also declares `margin-top:inherit` — a NON-inherited property whose
        // parent has no value, exercising the `inherit`-keyword drop path.
        let css = "html{box-sizing:border-box} *{box-sizing:inherit} \
                   .row{width:600px} \
                   .c{display:inline-block;width:50%;padding:0 10px;background:#f00;margin-top:inherit}";
        let mut diags = crate::Diagnostics::default();
        let root =
            super::layout_html(html, css, 600.0, &FontRegistry::new(), &mut diags).expect("layout");
        let mut boxes = Vec::new();
        let mut stack = vec![&root];
        while let Some(f) = stack.pop() {
            if matches!(
                f.content,
                FragmentContent::Box {
                    background: Some(_),
                    ..
                }
            ) {
                boxes.push((f.width, f.y));
            }
            stack.extend(f.children.iter());
        }
        assert_eq!(boxes.len(), 2, "two card boxes");
        for (w, _) in &boxes {
            assert!(
                (*w - 300.0).abs() < 1.0,
                "50% card is border-box (300px), got {w}"
            );
        }
        assert!(
            (boxes[0].1 - boxes[1].1).abs() < 1.0,
            "both cards share a row (border-box let them fit)"
        );
    }

    #[test]
    fn layout_boxes_reports_placed_data_cid_geometry() {
        // A padded, offset box tags itself; `layout_boxes` should report its
        // absolute border-box rectangle. box-sizing:border-box keeps width == 200.
        let html = r#"<html><body><style>
            body { margin: 0 }
            #a { box-sizing: border-box; width: 200px; height: 80px;
                 margin: 10px 0 0 30px; padding: 5px; border: 2px solid #000 }
        </style><div id="a" data-cid="a">hi</div></body></html>"#;
        let mut diags = Diagnostics::default();
        let boxes = layout_boxes(html, "", 600.0, 800.0, &FontRegistry::new(), &mut diags)
            .expect("layout_boxes");
        assert_eq!(boxes.len(), 1, "one tagged box");
        let b = &boxes[0];
        assert_eq!(b.cid, "a");
        assert!((b.x - 30.0).abs() < 0.5, "x == margin-left, got {}", b.x);
        assert!((b.y - 10.0).abs() < 0.5, "y == margin-top, got {}", b.y);
        assert!(
            (b.width - 200.0).abs() < 0.5,
            "border-box width == 200, got {}",
            b.width
        );
        assert!(
            (b.height - 80.0).abs() < 0.5,
            "border-box height == 80, got {}",
            b.height
        );
    }

    #[test]
    fn layout_boxes_ignores_untagged_and_inline_flattened() {
        // No data-cid anywhere -> empty; an inline element's data-cid is dropped
        // by inline flattening (documented limitation).
        let mut diags = Diagnostics::default();
        let plain = layout_boxes(
            "<body><div>x</div></body>",
            "",
            400.0,
            400.0,
            &FontRegistry::new(),
            &mut diags,
        )
        .expect("layout_boxes");
        assert!(plain.is_empty(), "no tagged boxes");
        let inline = layout_boxes(
            r#"<body><span data-cid="s">x</span></body>"#,
            "",
            400.0,
            400.0,
            &FontRegistry::new(),
            &mut diags,
        )
        .expect("layout_boxes");
        assert!(
            inline.is_empty(),
            "inline span's data-cid is flattened away"
        );
    }
}
