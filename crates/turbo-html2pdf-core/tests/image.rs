//! Phase 9b raster-image tests (§7.4, AC-7.4). Covers the decode layer
//! (`image::{sniff, probe, decode}` for PNG RGB/RGBA/gray + JPEG, plus the
//! malformed-input paths), the layout-time size caps (100% width, 60% body
//! height, aspect-ratio preservation), and the full layout → emit path (image
//! XObject, alpha → SMask, JPEG → DCTDecode, the no-image default, determinism,
//! and a `qpdf --check` when available).
//!
//! All test images are generated in-process (PNG via the `png` encoder; one tiny
//! committed baseline-JPEG byte array) so no external/system files are touched.
//!
//! Split at the S1 relocation: the layout-only cases moved to turbo-dom-layout's
//! `tests/image.rs`; the ones here need TKind directives, templating or the PDF
//! emitter.

mod common;

use std::collections::HashMap;
use std::io::Cursor;
use turbo_dom_layout::test_support::literal::Node;
use turbo_html2pdf_core::paginate::{paginate, Page, PageGeometry};
use turbo_html2pdf_core::style::TokenSet;
use turbo_html2pdf_core::{
    build_cascade, emit_pdf, emit_pdf_with_images, layout_with_images, ComputedStyle, Diagnostics,
    EmitOptions, ImageResolver, StyledElement, StyledNode, Tag,
};
use turbo_html2pdf_core::{Attr, FragmentContent, ImageCtx};

// --------------------------------------------------------------------------
// test image generation
// --------------------------------------------------------------------------

/// Encode an RGB (or RGBA, or gray) PNG of the given size from raw samples.
fn make_png(width: u32, height: u32, color: png::ColorType, samples: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut out), width, height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(samples).expect("png data");
    }
    out
}

/// A 2x2 solid-red RGB PNG.
fn png_rgb_2x2() -> Vec<u8> {
    let px = [255u8, 0, 0];
    let samples: Vec<u8> = px.iter().cycle().take(2 * 2 * 3).copied().collect();
    make_png(2, 2, png::ColorType::Rgb, &samples)
}

/// A 2x2 RGBA PNG with a varying alpha channel (so it needs an SMask).
fn png_rgba_2x2() -> Vec<u8> {
    let samples = [
        10, 20, 30, 0, // top-left, fully transparent
        40, 50, 60, 128, // top-right, half
        70, 80, 90, 200, //
        100, 110, 120, 255, // opaque
    ];
    make_png(2, 2, png::ColorType::Rgba, &samples)
}

/// A 2x2 grayscale PNG.
fn png_gray_2x2() -> Vec<u8> {
    make_png(2, 2, png::ColorType::Grayscale, &[10, 20, 30, 40])
}

/// A tiny valid 2x2 baseline RGB JPEG (committed bytes; see module docs).
const JPEG_RGB_2X2: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x06, 0x04, 0x05, 0x06, 0x05, 0x04, 0x06,
    0x06, 0x05, 0x06, 0x07, 0x07, 0x06, 0x08, 0x0A, 0x10, 0x0A, 0x0A, 0x09, 0x09, 0x0A, 0x14, 0x0E,
    0x0F, 0x0C, 0x10, 0x17, 0x14, 0x18, 0x18, 0x17, 0x14, 0x16, 0x16, 0x1A, 0x1D, 0x25, 0x1F, 0x1A,
    0x1B, 0x23, 0x1C, 0x16, 0x16, 0x20, 0x2C, 0x20, 0x23, 0x26, 0x27, 0x29, 0x2A, 0x29, 0x19, 0x1F,
    0x2D, 0x30, 0x2D, 0x28, 0x30, 0x25, 0x28, 0x29, 0x28, 0xFF, 0xDB, 0x00, 0x43, 0x01, 0x07, 0x07,
    0x07, 0x0A, 0x08, 0x0A, 0x13, 0x0A, 0x0A, 0x13, 0x28, 0x1A, 0x16, 0x1A, 0x28, 0x28, 0x28, 0x28,
    0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28,
    0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28,
    0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0xFF, 0xC0,
    0x00, 0x11, 0x08, 0x00, 0x02, 0x00, 0x02, 0x03, 0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11,
    0x01, 0xFF, 0xC4, 0x00, 0x1F, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09,
    0x0A, 0x0B, 0xFF, 0xC4, 0x00, 0xB5, 0x10, 0x00, 0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05,
    0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7D, 0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21,
    0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08, 0x23,
    0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16, 0x17,
    0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A,
    0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A,
    0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A,
    0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99,
    0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7,
    0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5,
    0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1,
    0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFF, 0xC4, 0x00, 0x1F, 0x01, 0x00, 0x03,
    0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
    0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0xFF, 0xC4, 0x00, 0xB5, 0x11, 0x00,
    0x02, 0x01, 0x02, 0x04, 0x04, 0x03, 0x04, 0x07, 0x05, 0x04, 0x04, 0x00, 0x01, 0x02, 0x77, 0x00,
    0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13,
    0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xA1, 0xB1, 0xC1, 0x09, 0x23, 0x33, 0x52, 0xF0, 0x15,
    0x62, 0x72, 0xD1, 0x0A, 0x16, 0x24, 0x34, 0xE1, 0x25, 0xF1, 0x17, 0x18, 0x19, 0x1A, 0x26, 0x27,
    0x28, 0x29, 0x2A, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88,
    0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6,
    0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4,
    0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE2,
    0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9,
    0xFA, 0xFF, 0xDA, 0x00, 0x0C, 0x03, 0x01, 0x00, 0x02, 0x11, 0x03, 0x11, 0x00, 0x3F, 0x00, 0xF1,
    0x8D, 0x6F, 0x55, 0xD4, 0x60, 0xD6, 0x6F, 0xE2, 0x86, 0xFE, 0xEE, 0x38, 0xA3, 0xB8, 0x91, 0x51,
    0x12, 0x66, 0x0A, 0xA0, 0x31, 0x00, 0x00, 0x0F, 0x02, 0x8A, 0x28, 0xAF, 0xAB, 0xC3, 0xFF, 0x00,
    0x0A, 0x1E, 0x8B, 0xF2, 0x3D, 0x8A, 0xFF, 0x00, 0xC5, 0x97, 0xAB, 0xFC, 0xCF, 0xFF, 0xD9,
];

/// A by-name image resolver backed by a map.
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
// layout + emit pipeline tests
// --------------------------------------------------------------------------

fn img_node(src: &str, style: &str) -> Vec<Node> {
    let mut attrs = vec![Attr {
        name: "src".into(),
        value: src.into(),
    }];
    if !style.is_empty() {
        attrs.push(Attr {
            name: "style".into(),
            value: style.into(),
        });
    }
    vec![Node::Element(
        turbo_dom_layout::test_support::literal::Element {
            tag: Tag::Html("img".into()),
            attrs,
            children: Vec::new(),
        },
    )]
}

fn styled_img(src: &str, style: &str) -> Vec<StyledNode> {
    let cascade = build_cascade("", "", TokenSet::default());
    common::style_nodes(&img_node(src, style), &cascade)
}

fn page_with(body: Vec<turbo_html2pdf_core::Fragment>) -> Page {
    Page {
        geometry: PageGeometry::a4(),
        kind: turbo_html2pdf_core::PageKind::First,
        number: 1,
        body,
        header: Vec::new(),
        footer: Vec::new(),
        footnotes: Vec::new(),
        semantics: Default::default(),
        canvas: None,
    }
}

/// Lay out a single `<img>` and paginate it into pages, sizing against `images`.
fn paginate_img(src: &str, resolver: &dyn ImageResolver) -> Vec<Page> {
    paginate_styled_img(src, "", resolver)
}

/// [`paginate_img`] with an inline `style` on the `<img>`.
fn paginate_styled_img(src: &str, style: &str, resolver: &dyn ImageResolver) -> Vec<Page> {
    let styled = styled_img(src, style);
    let ctx = ImageCtx {
        resolver,
        body_height: Some(700.0),
    };
    let mut diags = Diagnostics::default();
    let galley = layout_with_images(&styled, 540.0, &common::registry(), &ctx, &mut diags);
    paginate(&galley, &[], &mut diags).expect("paginate")
}

#[test]
fn emits_image_xobject_for_resolved_img() {
    let resolver = MapResolver::new(vec![("logo.png", png_rgb_2x2())]);
    let pages = paginate_img("logo.png", &resolver);
    let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(contains(&pdf, b"/Subtype /Image"), "missing image XObject");
    assert!(contains(&pdf, b"/Im0"), "missing image resource name");
    assert!(contains(&pdf, b"/DeviceRGB"), "rgb color space expected");
    assert!(contains(&pdf, b"%%EOF"));
}

#[test]
fn alpha_png_emits_smask() {
    let resolver = MapResolver::new(vec![("a.png", png_rgba_2x2())]);
    let pages = paginate_img("a.png", &resolver);
    let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert!(contains(&pdf, b"/SMask"), "alpha image must emit an SMask");
    assert!(
        contains(&pdf, b"/DeviceGray"),
        "smask must be a DeviceGray image"
    );
}

#[test]
fn jpeg_emits_dct_decode() {
    let resolver = MapResolver::new(vec![("p.jpg", JPEG_RGB_2X2.to_vec())]);
    let pages = paginate_img("p.jpg", &resolver);
    let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert!(contains(&pdf, b"/DCTDecode"), "jpeg must ride DCTDecode");
    assert!(!contains(&pdf, b"/SMask"), "baseline jpeg has no smask");
}

/// [`JPEG_RGB_2X2`] with an EXIF APP1 (big-endian TIFF, Orientation=6) after its
/// JFIF APP0 — the layout of a camera JPEG.
fn jpeg_orientation_6() -> Vec<u8> {
    let app1: &[u8] = &[
        0xFF, 0xE1, 0x00, 0x22, b'E', b'x', b'i', b'f', 0, 0, b'M', b'M', 0x00, 0x2A, 0, 0, 0, 8,
        0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, 0, 0, 0, 0,
    ];
    let mut out = JPEG_RGB_2X2[..20].to_vec();
    out.extend_from_slice(app1);
    out.extend_from_slice(&JPEG_RGB_2X2[20..]);
    out
}

#[test]
fn exif_orientation_rotates_the_jpeg_placement() {
    // miaskiewicz/turbo-html2pdf#30: the image is embedded as stored (DCTDecode pass-
    // through) and the `cm` matrix turns it a quarter clockwise into its 2x2 px (1.5 pt)
    // box; `image-orientation: none` draws it as stored.
    let resolver = MapResolver::new(vec![("p.jpg", jpeg_orientation_6())]);
    let pdf = emit_pdf_with_images(
        &paginate_img("p.jpg", &resolver),
        &EmitOptions::default(),
        &resolver,
    );
    assert!(
        contains(&pdf, b"/DCTDecode"),
        "the stored JPEG rides through"
    );
    assert!(contains(&pdf, b"0 -1.5 1.5 0 "), "rotated placement matrix");
    let none = emit_pdf_with_images(
        &paginate_styled_img("p.jpg", "image-orientation:none", &resolver),
        &EmitOptions::default(),
        &resolver,
    );
    assert!(!contains(&none, b"0 -1.5 1.5 0 "));
    assert!(contains(&none, b"1.5 0 0 1.5 "), "stored placement matrix");
}

#[test]
fn grayscale_png_emits_device_gray() {
    let resolver = MapResolver::new(vec![("g.png", png_gray_2x2())]);
    let pages = paginate_img("g.png", &resolver);
    let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert!(contains(&pdf, b"/DeviceGray"));
}

#[test]
fn unresolved_image_emits_nothing_and_lays_out_zero_sized() {
    // The resolver has no entry for this name: layout sizes the box at zero and
    // the emitter embeds no XObject, but the document is still valid.
    let resolver = MapResolver::new(vec![]);
    let pages = paginate_img("missing.png", &resolver);
    let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(!contains(&pdf, b"/Subtype /Image"), "no image to embed");
}

#[test]
fn no_image_default_path_skips_image_fragments() {
    // `emit_pdf` (no resolver) on a page that carries an Image fragment must skip
    // it: no XObject, still a valid PDF. Exercises the painter's resolve-miss arm.
    let placement = turbo_html2pdf_core::ImagePlacement {
        name: "x.png".into(),
        intrinsic_w: 2,
        intrinsic_h: 2,
        has_alpha: false,
        tint: None,
        orientation: Default::default(),
    };
    let frag = turbo_html2pdf_core::Fragment::new(
        turbo_html2pdf_core::NodeId(1),
        10.0,
        10.0,
        20.0,
        20.0,
        FragmentContent::Image(placement),
    );
    let pdf = emit_pdf(&[page_with(vec![frag])], &EmitOptions::default());
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(!contains(&pdf, b"/Subtype /Image"));
}

fn image_frag(name: &str) -> turbo_html2pdf_core::Fragment {
    let placement = turbo_html2pdf_core::ImagePlacement {
        name: name.into(),
        intrinsic_w: 2,
        intrinsic_h: 2,
        has_alpha: false,
        tint: None,
        orientation: Default::default(),
    };
    let mut frag = turbo_html2pdf_core::Fragment::new(
        turbo_html2pdf_core::NodeId(1),
        0.0,
        0.0,
        2.0,
        2.0,
        FragmentContent::Box {
            background: None,
            border: Default::default(),
            border_radius: 0.0,
            shadow: None,
            gradient: None,
            transform: None,
        },
    );
    // Carry the image as a child so collect recurses into it too.
    frag.children.push(turbo_html2pdf_core::Fragment::new(
        turbo_html2pdf_core::NodeId(2),
        0.0,
        0.0,
        2.0,
        2.0,
        FragmentContent::Image(placement),
    ));
    frag
}

#[test]
fn image_store_collect_dedups_and_reports_size() {
    use turbo_html2pdf_core::emit::ImageStore;
    let resolver = MapResolver::new(vec![("dup.png", png_rgb_2x2())]);
    // The same name appears twice (across two fragments): collect must dedup.
    let page = page_with(vec![image_frag("dup.png"), image_frag("dup.png")]);
    let store = ImageStore::collect(&[page], &resolver);
    assert_eq!(store.len(), 1, "duplicate names collapse to one image");
    assert!(!store.is_empty());

    // An empty store reports emptiness.
    let empty = ImageStore::collect(&[page_with(vec![])], &resolver);
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
}

#[test]
fn image_store_skips_unresolvable_names_during_collect() {
    use turbo_html2pdf_core::emit::ImageStore;
    // The resolver lacks this name: collect resolves nothing and stores nothing.
    let resolver = MapResolver::new(vec![]);
    let store = ImageStore::collect(&[page_with(vec![image_frag("gone.png")])], &resolver);
    assert!(store.is_empty(), "unresolved names are skipped in collect");
}

#[test]
fn image_store_skips_undecodable_names_during_collect() {
    use turbo_html2pdf_core::emit::ImageStore;
    // The name resolves to bytes that are not a valid image: skipped, not panic.
    let resolver = MapResolver::new(vec![("bad.png", b"not an image".to_vec())]);
    let store = ImageStore::collect(&[page_with(vec![image_frag("bad.png")])], &resolver);
    assert!(store.is_empty(), "undecodable bytes are skipped in collect");
}

#[test]
fn image_emit_is_deterministic() {
    let resolver = MapResolver::new(vec![
        ("a.png", png_rgba_2x2()),
        ("b.jpg", JPEG_RGB_2X2.to_vec()),
    ]);
    let pages = paginate_img("a.png", &resolver);
    let a = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    let b = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert_eq!(a, b, "image embedding must be byte-deterministic");
}

#[test]
fn background_image_emits_xobject_behind_box() {
    let style = ComputedStyle::from_pairs([
        ("background-image", "url('bg.png')"),
        ("width", "100px"),
        ("height", "60px"),
    ]);
    let el = StyledElement {
        tag: Tag::Html("div".into()),
        attrs: Vec::new(),
        style,
        children: vec![StyledNode::Text("hi".into())],
        source: None,
    };
    let styled = vec![StyledNode::Element(el)];
    let resolver = MapResolver::new(vec![("bg.png", png_rgb_2x2())]);
    let ctx = ImageCtx {
        resolver: &resolver,
        body_height: Some(700.0),
    };
    let mut diags = Diagnostics::default();
    let galley = layout_with_images(&styled, 540.0, &common::registry(), &ctx, &mut diags);
    let pages = paginate(&galley, &[], &mut diags).expect("paginate");
    let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert!(contains(&pdf, b"/Subtype /Image"), "bg image must embed");
}

// --------------------------------------------------------------------------
// qpdf structural check (gated on availability)
// --------------------------------------------------------------------------

#[test]
fn qpdf_check_image_pdf_when_available() {
    if !qpdf_available() {
        return;
    }
    let resolver = MapResolver::new(vec![
        ("rgb.png", png_rgb_2x2()),
        ("rgba.png", png_rgba_2x2()),
        ("gray.png", png_gray_2x2()),
        ("photo.jpg", JPEG_RGB_2X2.to_vec()),
    ]);
    for name in ["rgb.png", "rgba.png", "gray.png", "photo.jpg"] {
        let pages = paginate_img(name, &resolver);
        let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
        assert_qpdf_clean(name, &pdf);
    }
}

fn qpdf_available() -> bool {
    std::process::Command::new("which")
        .arg("qpdf")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn assert_qpdf_clean(name: &str, pdf: &[u8]) {
    let path = std::env::temp_dir().join(format!("turbo-pdf-img-{name}.pdf"));
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

/// Every image XObject's stream body, asserted `FlateDecode` and inflated back to
/// its raw samples, in file order.
fn inflated_image_streams(pdf: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut rest = pdf;
    while let Some(at) = rest.windows(15).position(|w| w == b"/Subtype /Image") {
        let tail = &rest[at..];
        let dict_end = tail.windows(6).position(|w| w == b"stream").unwrap();
        assert!(
            contains(&tail[..dict_end], b"/FlateDecode"),
            "image stream must be Flate-compressed"
        );
        let body = &tail[dict_end + 7..];
        let end = body.windows(9).position(|w| w == b"endstream").unwrap();
        out.push(miniz_oxide::inflate::decompress_to_vec_zlib(&body[..end]).expect("valid zlib"));
        rest = &tail[dict_end..];
    }
    out
}

#[test]
fn png_samples_and_smask_are_flate_compressed_losslessly() {
    // A 300x300 RGBA PNG: raw it is 270 000 colour + 90 000 alpha bytes. Chrome 156
    // Flate-compresses both planes; embedding them uncompressed bloats every PDF that
    // carries a photo/logo PNG by its full decoded size.
    let (w, h) = (300u32, 300u32);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for i in 0..w * h {
        px.extend_from_slice(&[(i % 251) as u8, 40, 200, if i % 3 == 0 { 255 } else { 128 }]);
    }
    let resolver = MapResolver::new(vec![("big.png", make_png(w, h, png::ColorType::Rgba, &px))]);
    let pages = paginate_img("big.png", &resolver);
    let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert!(pdf.len() < 60_000, "uncompressed image: {} bytes", pdf.len());
    let streams = inflated_image_streams(&pdf);
    assert_eq!(streams.len(), 2, "colour XObject + SMask");
    let rgb: Vec<u8> = px.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let alpha: Vec<u8> = px.chunks(4).map(|p| p[3]).collect();
    assert!(streams.contains(&rgb), "colour plane round-trips");
    assert!(streams.contains(&alpha), "alpha plane round-trips");
}
