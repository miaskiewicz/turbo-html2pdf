//! Phase 15b `svg` feature tests (§7.4). Only compiled with `--features svg`.
//!
//! Covers the gated decode/probe/sniff layer (`image::{probe, decode}` routing an
//! `image/svg+xml` document through `svg::{is_svg, probe_svg, decode_svg}`), the
//! rasterizer's RGB + alpha-plane output, byte-determinism, malformed/degenerate
//! handling, and the full layout → emit path (rasterized SVG embeds as an image
//! XObject with an `SMask`, deterministic, `qpdf --check` clean when available).
//!
//! All SVG inputs are tiny committed byte strings (shapes only — no external
//! files, no fonts, so rendering is host-independent and deterministic).
//!
//! Split at the S1 relocation: the layout-only cases moved to turbo-dom-layout's
//! `tests/svg.rs`; the ones here need TKind directives, templating or the PDF emitter.

#![cfg(feature = "svg")]

mod common;

use std::collections::HashMap;
use turbo_dom_layout::test_support::literal::{Element, Node};
use turbo_html2pdf_core::paginate::{paginate, Page};
use turbo_html2pdf_core::{
    build_cascade, emit_pdf_with_images, layout_with_images, Attr, Diagnostics, EmitOptions,
    ImageCtx, ImageResolver, Tag,
};
use turbo_html2pdf_core::{style::TokenSet, StyledNode};

// --------------------------------------------------------------------------
// committed SVG inputs (shapes only — deterministic without fonts)
// --------------------------------------------------------------------------

/// A 4x4 SVG: a fully-opaque red square covering a half-opaque blue background,
/// so the raster carries both RGB colour and a varying alpha plane (→ SMask).
const SVG_RED_4X4: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4" viewBox="0 0 4 4">
  <rect x="0" y="0" width="4" height="4" fill="#0000ff" fill-opacity="0.5"/>
  <rect x="1" y="1" width="2" height="2" fill="#ff0000"/>
</svg>"##;

fn svg_bytes(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

/// A by-name resolver backed by a map (mirrors the raster image tests).
struct MapResolver {
    map: HashMap<String, Vec<u8>>,
}

impl MapResolver {
    fn new(pairs: Vec<(&str, Vec<u8>)>) -> MapResolver {
        MapResolver {
            map: pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        }
    }
}

impl ImageResolver for MapResolver {
    fn resolve(&self, name: &str) -> Option<&[u8]> {
        self.map.get(name).map(Vec::as_slice)
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

// --------------------------------------------------------------------------
// layout + emit pipeline
// --------------------------------------------------------------------------

fn img_node(src: &str) -> Vec<Node> {
    vec![Node::Element(Element {
        tag: Tag::Html("img".into()),
        attrs: vec![Attr {
            name: "src".into(),
            value: src.into(),
        }],
        children: Vec::new(),
    })]
}

fn styled_img(src: &str) -> Vec<StyledNode> {
    let cascade = build_cascade("", "", TokenSet::default());
    common::style_nodes(&img_node(src), &cascade)
}

fn paginate_img(src: &str, resolver: &dyn ImageResolver) -> Vec<Page> {
    let styled = styled_img(src);
    let ctx = ImageCtx {
        resolver,
        body_height: Some(700.0),
    };
    let mut diags = Diagnostics::default();
    let galley = layout_with_images(&styled, 540.0, &common::registry(), &ctx, &mut diags);
    paginate(&galley, &[], &mut diags).expect("paginate")
}

#[test]
fn svg_emits_image_xobject_with_smask() {
    let resolver = MapResolver::new(vec![("logo.svg", svg_bytes(SVG_RED_4X4))]);
    let pages = paginate_img("logo.svg", &resolver);
    let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(contains(&pdf, b"/Subtype /Image"), "missing image XObject");
    assert!(contains(&pdf, b"/Im0"), "missing image resource name");
    assert!(contains(&pdf, b"/DeviceRGB"), "rasterized svg is rgb");
    assert!(
        contains(&pdf, b"/SMask"),
        "transparent svg must emit an SMask"
    );
    assert!(contains(&pdf, b"/DeviceGray"), "smask is a gray image");
    assert!(contains(&pdf, b"%%EOF"));
}

#[test]
fn svg_emit_is_deterministic() {
    let resolver = MapResolver::new(vec![("logo.svg", svg_bytes(SVG_RED_4X4))]);
    let pages = paginate_img("logo.svg", &resolver);
    let a = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    let b = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert_eq!(a, b, "svg embedding must be byte-deterministic");
}

#[test]
fn malformed_svg_emits_nothing() {
    let resolver = MapResolver::new(vec![("bad.svg", b"<svg><rect".to_vec())]);
    let pages = paginate_img("bad.svg", &resolver);
    let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(
        !contains(&pdf, b"/Subtype /Image"),
        "malformed svg embeds nothing"
    );
}

// --------------------------------------------------------------------------
// qpdf structural check (gated on availability)
// --------------------------------------------------------------------------

#[test]
fn qpdf_check_svg_pdf_when_available() {
    if !qpdf_available() {
        return;
    }
    let resolver = MapResolver::new(vec![("logo.svg", svg_bytes(SVG_RED_4X4))]);
    let pages = paginate_img("logo.svg", &resolver);
    let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert_qpdf_clean("logo.svg", &pdf);
}

fn qpdf_available() -> bool {
    std::process::Command::new("which")
        .arg("qpdf")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn assert_qpdf_clean(name: &str, pdf: &[u8]) {
    let path = std::env::temp_dir().join(format!("turbo-pdf-svg-{name}.pdf"));
    std::fs::write(&path, pdf).expect("write temp pdf");
    let out = std::process::Command::new("qpdf")
        .arg("--check")
        .arg(&path)
        .output()
        .expect("run qpdf");
    assert!(
        out.status.success(),
        "{name}: qpdf --check failed: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}
