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

// --------------------------------------------------------------------------
// miaskiewicz/turbo-html2pdf#46: a vector image cut at a page boundary
// --------------------------------------------------------------------------

/// A 4x4 opaque red SVG.
const SVG_RED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4" fill="#ff0000"/></svg>"##;

/// A split `bottom: 20px` relative block whose 40px SVG image, moved to page 2, is
/// shifted 20px above that page's area: its top half paints at the end of page 1
/// (miaskiewicz/turbo-html2pdf#42).
fn carried_svg_pdf() -> Vec<u8> {
    let html = r#"<!DOCTYPE html><html><head><style>@page{size:600px 400px;margin:40px}body{margin:0}</style></head><body><div style="position:relative;bottom:20px"><div style="height:300px"></div><div style="height:40px;break-inside:avoid"><img src="red.svg" style="display:block;width:40px;height:40px"></div></div></body></html>"#;
    let resolver = MapResolver::new(vec![("red.svg", svg_bytes(SVG_RED))]);
    let (program, _) = turbo_html2pdf_core::compile(html, &Default::default()).expect("compile");
    let cascade = build_cascade("", "", TokenSet::default());
    let fonts = common::registry();
    let inputs = turbo_html2pdf_core::RenderInputs {
        program: &program,
        data: &serde_json::Value::Null,
        cascade: &cascade,
        at_rules: &[],
        fonts: &fonts,
        images: &resolver,
        now: Some(0),
    };
    let pages = turbo_html2pdf_core::render_pages(&inputs, &mut Diagnostics::default())
        .expect("render_pages");
    assert_eq!(pages.len(), 2);
    emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver)
}

#[test]
fn a_vector_image_cut_at_a_page_boundary_paints_inside_its_clip_on_both_pages() {
    let pdf = carried_svg_pdf();
    let text = String::from_utf8_lossy(&pdf);
    // Each page draws the image once, inside a saved state with a clipping path.
    let draws: Vec<usize> = text.match_indices(" Do\n").map(|(i, _)| i).collect();
    assert_eq!(draws.len(), 2, "drawn once per page");
    for &at in &draws {
        let clip = text[..at]
            .rfind("re\nW\nn")
            .expect("a clipping path before the draw");
        assert!(text[..clip].rfind("q\n").is_some(), "in a saved state");
        assert!(text[at..].find("Q\n").is_some(), "restored after the draw");
    }
    if std::process::Command::new("pdftoppm")
        .arg("-v")
        .output()
        .is_err()
    {
        eprintln!("pdftoppm not on PATH; skipping the raster probe");
        return;
    }
    let red = |page: usize, y: usize| raster_red(&pdf, page, 60, y);
    // Page 1: the image's top half at 340..359, cut at the page area's end (360).
    assert!(red(1, 341) && red(1, 358), "page 1 shows the carried half");
    assert!(!red(1, 362), "page 1 is cut at its area's end");
    // Page 2: the image at 20..59, shown from 1px above the page area (39).
    assert!(red(2, 41) && red(2, 58), "page 2 shows its own half");
    assert!(!red(2, 30), "page 2 is cut above its area");
}

/// Whether pixel (`x`, `y`) of page `page` (1-based), rasterized by `pdftoppm` at
/// 96 dpi (1 device px per CSS px), is red.
fn raster_red(pdf: &[u8], page: usize, x: usize, y: usize) -> bool {
    let dir = std::env::temp_dir().join(format!("turbo_svg_clip_{}_{page}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let input = dir.join("doc.pdf");
    std::fs::write(&input, pdf).expect("write pdf");
    let n = page.to_string();
    let out = std::process::Command::new("pdftoppm")
        .args(["-r", "96", "-f", &n, "-l", &n, "-singlefile"])
        .arg(&input)
        .arg(dir.join("page"))
        .output()
        .expect("run pdftoppm");
    assert!(out.status.success());
    let ppm = std::fs::read(dir.join("page.ppm")).expect("read ppm");
    std::fs::remove_dir_all(&dir).ok();
    let header: Vec<&[u8]> = ppm.splitn(4, |&b| b == b'\n').collect();
    let width: usize = String::from_utf8_lossy(header[1])
        .split(' ')
        .next()
        .and_then(|w| w.parse().ok())
        .expect("ppm width");
    let px = &header[3][(y * width + x) * 3..][..3];
    px[0] > 200 && px[1] < 60 && px[2] < 60
}
