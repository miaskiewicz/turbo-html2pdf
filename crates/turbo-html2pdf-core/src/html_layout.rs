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

use crate::image::ImageResolver;
use crate::layout::boxgen::{build_box_tree, BoxKind, InlineItem, LayoutBox};
use crate::layout::fragment::{Fragment, FragmentContent, Transform2D};
use crate::layout::ImageCtx;
use crate::node::{Element, Node, Tag};
use crate::style::{
    build_cascade_with_width, set_media_viewport_height, style_tree_with_roots, TokenSet,
};
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
    // Decode any `data:` image URIs the fixtures embed so replaced `<img>` boxes
    // reach their intrinsic size (the default render path takes a host resolver; the
    // conformance seam is self-contained, so it decodes the inline data itself).
    // Scanned here while `nodes` is still owned — `style_tree_with_roots` consumes it.
    let mut data_images: HashMap<String, Vec<u8>> = HashMap::new();
    collect_data_uri_images(&nodes, &mut data_images);
    // Thread the layout viewport HEIGHT so `@media (min/max-height)` conditions and
    // `vh`/`vmin`/`vmax` units resolve against it — without this both fell back to the
    // 800px default (a `20vh` box measured against 800, height-gated `@media` rules
    // matched everywhere). `build_cascade_with_width` sets the matching width.
    set_media_viewport_height(cb_height);
    let cascade = build_cascade_with_width(&author_css, "", TokenSet::default(), cb_width);
    let styled = style_tree_with_roots(&strip_non_visual(nodes), &cascade, &roots);

    // `build_box_tree` is deterministic (monotonic pre-order ids), so the ids in
    // this tree match the ones `crate::layout` stamps on the fragments below.
    let mut cids: HashMap<u32, String> = HashMap::new();
    collect_cids(&build_box_tree(&styled), &mut cids);

    let resolver = DataUriImages(data_images);
    let ctx = ImageCtx {
        resolver: &resolver,
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
            let (x, y, width, height) = transformed_rect(frag);
            out.push(CidBox {
                cid: cid.clone(),
                x,
                y,
                width,
                height,
            });
        }
    }
    for c in &frag.children {
        collect_boxes(c, cids, seen, out);
    }
}

/// A self-contained image resolver for the conformance seam: it holds the bytes of
/// every `data:` URI decoded from the document, keyed by the exact `src` string the
/// layout looks up. Nothing else (no host, no I/O) — the default render path still
/// takes a caller-supplied [`ImageResolver`].
struct DataUriImages(HashMap<String, Vec<u8>>);

impl ImageResolver for DataUriImages {
    fn resolve(&self, name: &str) -> Option<&[u8]> {
        self.0.get(name).map(Vec::as_slice)
    }
}

/// Walk the node tree and decode every `<img src="data:...;base64,...">` into
/// `out`, keyed by the full `src` (the name the layout resolves against).
fn collect_data_uri_images(nodes: &[Node], out: &mut HashMap<String, Vec<u8>>) {
    for node in nodes {
        let Node::Element(el) = node else { continue };
        record_img_data_uri(el, out);
        collect_data_uri_images(&el.children, out);
    }
}

/// Decode this element's `<img src="data:...;base64,...">` into `out` (keyed by
/// `src`), if it is an `img` with an as-yet-unseen, decodable base64 data URI.
fn record_img_data_uri(el: &Element, out: &mut HashMap<String, Vec<u8>>) {
    let Tag::Html(name) = &el.tag else { return };
    if name != "img" {
        return;
    }
    let Some(src) = el.attr("src").filter(|s| s.starts_with("data:")) else {
        return;
    };
    if !out.contains_key(src) {
        if let Some(bytes) = decode_data_uri(src) {
            out.insert(src.to_string(), bytes);
        }
    }
}

/// Decode a `data:[<mediatype>];base64,<payload>` URI to bytes. Only base64 data
/// URIs are handled (the raster fixtures embed encoded images that way); a
/// percent-encoded/text data URI yields `None`.
fn decode_data_uri(src: &str) -> Option<Vec<u8>> {
    let rest = src.strip_prefix("data:")?;
    let comma = rest.find(',')?;
    if !rest[..comma].contains(";base64") {
        return None;
    }
    base64_decode(&rest[comma + 1..])
}

/// Minimal standard-alphabet base64 decoder (no dependency): skips padding and
/// whitespace, packs 6-bit groups into bytes. `None` on an invalid character.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in s.as_bytes() {
        base64_feed(c, &mut acc, &mut bits, &mut out)?;
    }
    Some(out)
}

/// Fold one base64 character into the rolling accumulator, emitting a decoded byte
/// once at least 8 bits are buffered. Padding and whitespace are skipped; an
/// invalid character yields `None`.
fn base64_feed(c: u8, acc: &mut u32, bits: &mut u32, out: &mut Vec<u8>) -> Option<()> {
    fn sextet(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    if c == b'=' || c.is_ascii_whitespace() {
        return Some(());
    }
    *acc = (*acc << 6) | sextet(c)?;
    *bits += 6;
    if *bits >= 8 {
        *bits -= 8;
        out.push((*acc >> *bits) as u8);
    }
    Some(())
}

/// A fragment's border-box rect after its CSS 2D `transform`, as the axis-aligned
/// bounding box of the four transformed corners — exactly what a browser's
/// `getBoundingClientRect()` reports for a transformed element. Untransformed
/// fragments return their plain rect. The transform lives on the paint (never on
/// `x`/`y`), so this is applied only on this debug/conformance read-back.
fn transformed_rect(frag: &Fragment) -> (f32, f32, f32, f32) {
    let t = match &frag.content {
        FragmentContent::Box {
            transform: Some(t), ..
        } => *t,
        _ => return (frag.x, frag.y, frag.width, frag.height),
    };
    let Transform2D {
        matrix: [a, b, c, d, e, f],
        origin_x,
        origin_y,
    } = t;
    // Map a corner (relative to the box's top-left) about `transform-origin`:
    // p' = origin + M·(p − origin), with M mapping (x,y) → (a·x+c·y+e, b·x+d·y+f).
    let map = |px: f32, py: f32| {
        let (u, v) = (px - origin_x, py - origin_y);
        (a * u + c * v + e + origin_x, b * u + d * v + f + origin_y)
    };
    let corners = [
        map(0.0, 0.0),
        map(frag.width, 0.0),
        map(0.0, frag.height),
        map(frag.width, frag.height),
    ];
    let min_x = corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|p| p.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    let max_y = corners
        .iter()
        .map(|p| p.1)
        .fold(f32::NEG_INFINITY, f32::max);
    (frag.x + min_x, frag.y + min_y, max_x - min_x, max_y - min_y)
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
    fn base64_and_data_uri_decode_a_png_header() {
        // Round-trips the base64 decoder and the data-URI split on a real 8x6 PNG.
        let uri = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAG\
                   CAIAAABxZ0isAAAAEUlEQVR4nGMI6DmBFTEMpAQA/aBOwe19mXAAAAAASUVORK5CYII=";
        let bytes = super::decode_data_uri(uri).expect("data uri decodes");
        let intrinsic = crate::image::probe(&bytes).expect("png intrinsic");
        assert_eq!((intrinsic.width, intrinsic.height), (8, 6));
        // A non-base64 data URI is not handled (returns None, no panic).
        assert!(super::decode_data_uri("data:text/plain,hello").is_none());
    }

    #[test]
    fn layout_boxes_sizes_a_data_uri_image_by_intrinsic_aspect() {
        // An 8x6 (4:3) PNG with `width:100%` of a 200px frame + `height:auto` derives
        // its height from the intrinsic ratio (200 * 6/8 = 150). Guards both the
        // data-URI decode in the conformance seam and % image sizing (0.2.10).
        let html = r#"<html><body><style>
            body { margin: 0 }
            #frame { width: 200px }
            #pic { display: block; width: 100%; height: auto }
        </style><div id="frame"><img id="pic" data-cid="pic"
            src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAGCAIAAABxZ0isAAAAEUlEQVR4nGMI6DmBFTEMpAQA/aBOwe19mXAAAAAASUVORK5CYII="></div></body></html>"#;
        let mut diags = Diagnostics::default();
        let boxes = layout_boxes(html, "", 600.0, 800.0, &FontRegistry::new(), &mut diags)
            .expect("layout_boxes");
        let pic = boxes.iter().find(|b| b.cid == "pic").expect("pic box");
        assert!(
            (pic.width - 200.0).abs() < 0.5,
            "100% width, got {}",
            pic.width
        );
        assert!(
            (pic.height - 150.0).abs() < 0.5,
            "intrinsic-aspect height 150, got {}",
            pic.height
        );
    }

    #[test]
    fn layout_boxes_reports_transformed_bounding_box() {
        // `getBoundingClientRect` parity: a `scale(2)` about `top left` reports the
        // scaled 200x80 border box at the origin (not the untransformed 100x40).
        let html = r#"<html><body><style>
            body { margin: 0 }
            #s { width: 100px; height: 40px; transform: scale(2);
                 transform-origin: top left; background: #ccc }
        </style><div id="s" data-cid="s">s</div></body></html>"#;
        let mut diags = Diagnostics::default();
        let boxes = layout_boxes(html, "", 600.0, 800.0, &FontRegistry::new(), &mut diags)
            .expect("layout_boxes");
        let s = boxes.iter().find(|b| b.cid == "s").expect("s box");
        assert!((s.x).abs() < 0.5 && (s.y).abs() < 0.5, "origin at (0,0)");
        assert!(
            (s.width - 200.0).abs() < 0.5 && (s.height - 80.0).abs() < 0.5,
            "scaled to 200x80, got {}x{}",
            s.width,
            s.height
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

    #[test]
    fn inline_block_data_cid_is_reported() {
        // Unlike a plain inline `<span>`, an `inline-block` becomes an atomic box, so
        // `collect_line_atoms` recurses into it and its `data-cid` is reported.
        let html = r#"<html><body><div><span
            style="display:inline-block;width:20px;height:10px" data-cid="ib">x</span></div></body></html>"#;
        let mut diags = Diagnostics::default();
        let boxes = layout_boxes(html, "", 400.0, 400.0, &FontRegistry::new(), &mut diags)
            .expect("layout_boxes");
        assert!(
            boxes.iter().any(|b| b.cid == "ib"),
            "inline-block atom's data-cid is reported"
        );
    }

    #[test]
    fn non_data_img_src_is_not_collected() {
        // `collect_data_uri_images` skips an `<img>` whose `src` is not a `data:` URI
        // (a plain http src) — no panic, nothing decoded, layout still succeeds.
        let html = r#"<html><body><img src="http://example.com/a.png"></body></html>"#;
        let mut diags = Diagnostics::default();
        let boxes = layout_boxes(html, "", 400.0, 400.0, &FontRegistry::new(), &mut diags)
            .expect("layout_boxes");
        assert!(boxes.is_empty(), "no tagged boxes, non-data img ignored");
    }

    #[test]
    fn base64_decode_handles_plus_slash_and_rejects_invalid() {
        // `+`/`/` are the standard-alphabet's 62/63 sextets; "Tnk+" -> "Ny>".
        assert_eq!(super::base64_decode("Tnk+").unwrap(), b"Ny>");
        // whitespace and `=` padding are skipped; an out-of-alphabet char fails.
        assert_eq!(super::base64_decode("Tm8=").unwrap(), b"No");
        assert!(super::base64_decode("!!!!").is_none());
    }

    #[test]
    fn block_children_of_a_directive_box_is_empty() {
        // A `Directive` box is not a block/flex/grid/table container, so it exposes
        // no block-level children (the `_ => &[]` arm).
        use crate::layout::boxgen::BoxKind;
        use crate::node::TKind;
        assert!(super::block_children(&BoxKind::Directive(TKind::Footnote)).is_empty());
    }
}
