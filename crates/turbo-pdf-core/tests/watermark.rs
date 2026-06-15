//! Phase 17 page-watermark tests (§7). Covers the text watermark (default
//! `DRAFT`, custom word/color/angle/opacity, fade via `ExtGState /ca`, glyphs
//! embedded through the shared font store) and the image watermark (centered and
//! tiled, resolved + decoded through the *shared* Phase 9b
//! `ImageResolver`/`ImageStore` raster path), plus behind-body ordering, the
//! no-watermark default, determinism, and a `qpdf --check` when available.
//!
//! Content streams are emitted uncompressed, so these tests inspect the raw PDF
//! bytes directly for the operators (`gs`, `BT`, `Do`, `cm`) the watermark adds.

mod common;

use std::collections::HashMap;
use std::io::Cursor;

use turbo_pdf_core::layout::fragment::{Fragment, FragmentContent, NodeId, PositionedGlyph};
use turbo_pdf_core::layout::value::Rgba;
use turbo_pdf_core::paginate::{Page, PageGeometry};
use turbo_pdf_core::{
    emit_pdf, emit_pdf_with_images, EmitOptions, FontFace, ImageResolver, ImageWatermark,
    TextWatermark, Watermark,
};

// --------------------------------------------------------------------------
// fixtures
// --------------------------------------------------------------------------

/// A by-name image resolver backed by a map (mirrors the Phase 9b test helper).
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

/// A 4x4 solid-red RGB PNG to use as a watermark raster.
fn png_red_4x4() -> Vec<u8> {
    let samples: Vec<u8> = [255u8, 0, 0]
        .iter()
        .cycle()
        .take(4 * 4 * 3)
        .copied()
        .collect();
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut out), 4, 4);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(&samples).expect("png data");
    }
    out
}

/// A single A4 page wrapping the given body fragments.
fn page_with(body: Vec<Fragment>) -> Page {
    Page {
        geometry: PageGeometry::a4(),
        kind: turbo_pdf_core::PageKind::First,
        number: 1,
        body,
        header: Vec::new(),
        footer: Vec::new(),
        footnotes: Vec::new(),
    }
}

/// A body text line (its glyphs land in the font store as body content).
fn body_text(face: FontFace) -> Fragment {
    let glyphs = [10u16, 11, 12]
        .iter()
        .enumerate()
        .map(|(i, &glyph_id)| PositionedGlyph {
            glyph_id,
            x: i as f32 * 10.0,
            y: 12.0,
        })
        .collect();
    Fragment::new(
        NodeId(1),
        20.0,
        30.0,
        200.0,
        16.0,
        FragmentContent::TextLine {
            glyphs,
            face,
            font_size: 12.0,
            color: Rgba::new(0, 0, 0, 255),
        },
    )
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The byte offset of `needle` in `haystack`, if present.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// An emit-options carrying a text watermark.
fn text_opts(mark: TextWatermark) -> EmitOptions {
    EmitOptions {
        watermark: Some(Watermark::Text(Box::new(mark))),
        ..EmitOptions::default()
    }
}

/// An emit-options carrying an image watermark.
fn image_opts(mark: ImageWatermark) -> EmitOptions {
    EmitOptions {
        watermark: Some(Watermark::Image(mark)),
        ..EmitOptions::default()
    }
}

// --------------------------------------------------------------------------
// default: no watermark
// --------------------------------------------------------------------------

#[test]
fn default_emit_has_no_watermark_state() {
    let pages = vec![page_with(vec![body_text(common::evolventa())])];
    let pdf = emit_pdf(&pages, &EmitOptions::default());
    assert!(pdf.starts_with(b"%PDF-1.7"));
    // No fade ExtGState and no `gs` watermark operator without a watermark.
    assert!(
        !contains(&pdf, b"/GSwm"),
        "no watermark ExtGState by default"
    );
    assert!(
        !contains(&pdf, b"/ExtGState"),
        "no ExtGState dict by default"
    );
}

// --------------------------------------------------------------------------
// text watermark: DRAFT default, faded
// --------------------------------------------------------------------------

#[test]
fn draft_text_watermark_is_present_and_faded() {
    let pages = vec![page_with(vec![body_text(common::evolventa())])];
    let opts = text_opts(TextWatermark::draft(common::evolventa()));
    let pdf = emit_pdf(&pages, &opts);
    // The fade ExtGState exists, carries a `/ca` < 1, and is set in the stream.
    assert!(
        contains(&pdf, b"/GSwm"),
        "draft watermark needs its ExtGState"
    );
    assert!(
        contains(&pdf, b"/ca 0.25"),
        "DRAFT default fades to ca 0.25"
    );
    assert!(
        contains(&pdf, b"/GSwm gs"),
        "fade gs must be set in the stream"
    );
    // A text object is drawn (BT/ET) and a rotation matrix (cm) is applied.
    assert!(contains(&pdf, b"BT"), "watermark text object expected");
    assert!(contains(&pdf, b" cm"), "rotation matrix expected");
}

// --------------------------------------------------------------------------
// text watermark: custom word / color / angle / opacity
// --------------------------------------------------------------------------

#[test]
fn custom_text_watermark_word_color_angle_opacity() {
    let pages = vec![page_with(vec![body_text(common::evolventa())])];
    let mark = TextWatermark {
        text: "BROUILLON".to_string(),
        face: common::evolventa(),
        font_size: 48.0,
        color: Rgba::new(200, 0, 0, 255),
        opacity: 0.4,
        angle_deg: 30.0,
    };
    let pdf = emit_pdf(&pages, &text_opts(mark));
    // Custom opacity reaches the ExtGState `/ca`.
    assert!(contains(&pdf, b"/ca 0.4"), "custom opacity must set ca 0.4");
    // Custom red fill color (200/255 ≈ 0.7843) reaches the stream.
    assert!(
        contains(&pdf, b"0.78431") || contains(&pdf, b"0.784313"),
        "custom red fill expected"
    );
    // The word's glyphs are subset into a font program: the document embeds a
    // font (the same store body text uses), so the watermark word is real text.
    assert!(
        contains(&pdf, b"/FontFile2"),
        "watermark word must embed glyphs"
    );
}

#[test]
fn custom_angle_changes_the_rotation_matrix() {
    let pages = vec![page_with(vec![body_text(common::evolventa())])];
    let at_30 = emit_pdf(
        &pages,
        &text_opts(TextWatermark {
            angle_deg: 30.0,
            ..TextWatermark::draft(common::evolventa())
        }),
    );
    let at_60 = emit_pdf(
        &pages,
        &text_opts(TextWatermark {
            angle_deg: 60.0,
            ..TextWatermark::draft(common::evolventa())
        }),
    );
    assert_ne!(
        at_30, at_60,
        "different angles must yield different matrices"
    );
}

// --------------------------------------------------------------------------
// image watermark: centered + tiled, via the shared resolver
// --------------------------------------------------------------------------

#[test]
fn image_watermark_centered_via_shared_resolver() {
    let resolver = MapResolver::new(vec![("wm.png", png_red_4x4())]);
    let pages = vec![page_with(vec![])];
    let opts = image_opts(ImageWatermark {
        name: "wm.png".to_string(),
        opacity: 0.3,
        tiled: false,
    });
    let pdf = emit_pdf_with_images(&pages, &opts, &resolver);
    // The raster is embedded as an image XObject (shared Phase 9b path) and
    // drawn (`Do`) behind the body with the fade applied.
    assert!(contains(&pdf, b"/Subtype /Image"), "image XObject expected");
    assert!(contains(&pdf, b"/Im0"), "image resource name expected");
    assert!(contains(&pdf, b"/ca 0.3"), "image fade ca 0.3 expected");
    assert!(
        contains(&pdf, b"/Im0 Do"),
        "image must be drawn once, centered"
    );
}

#[test]
fn image_watermark_tiled_draws_many_copies() {
    let resolver = MapResolver::new(vec![("wm.png", png_red_4x4())]);
    let pages = vec![page_with(vec![])];
    let centered = emit_pdf_with_images(
        &pages,
        &image_opts(ImageWatermark {
            name: "wm.png".to_string(),
            opacity: 0.3,
            tiled: false,
        }),
        &resolver,
    );
    let tiled = emit_pdf_with_images(
        &pages,
        &image_opts(ImageWatermark {
            name: "wm.png".to_string(),
            opacity: 0.3,
            tiled: true,
        }),
        &resolver,
    );
    let count = |pdf: &[u8]| pdf.windows(7).filter(|w| *w == b"/Im0 Do").count();
    assert_eq!(count(&centered), 1, "centered draws exactly one copy");
    assert!(
        count(&tiled) > 1,
        "tiled must repeat the raster many times, got {}",
        count(&tiled)
    );
}

#[test]
fn unresolved_image_watermark_paints_nothing() {
    // The resolver lacks the name: nothing is embedded or drawn, still valid PDF.
    let resolver = MapResolver::new(vec![]);
    let pages = vec![page_with(vec![])];
    let opts = image_opts(ImageWatermark {
        name: "gone.png".to_string(),
        opacity: 0.3,
        tiled: false,
    });
    let pdf = emit_pdf_with_images(&pages, &opts, &resolver);
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(!contains(&pdf, b"/Subtype /Image"), "nothing to embed");
    assert!(!contains(&pdf, b"/Im0 Do"), "nothing to draw");
}

// --------------------------------------------------------------------------
// behind-body ordering
// --------------------------------------------------------------------------

#[test]
fn text_watermark_paints_behind_body() {
    let pages = vec![page_with(vec![body_text(common::evolventa())])];
    let pdf = emit_pdf(
        &pages,
        &text_opts(TextWatermark::draft(common::evolventa())),
    );
    // The watermark sets its fade gs before the first body text is shown. The
    // body line uses `Tf` (set_font) after `begin_text`; the watermark's `gs`
    // precedes it in the single content stream.
    let gs = find(&pdf, b"/GSwm gs").expect("watermark gs present");
    // The body text fill color (black: `0 0 0 rg`) is painted after the mark.
    let body = find(&pdf, b"0 0 0 rg").expect("body text fill present");
    assert!(gs < body, "watermark gs must come before body paint");
}

#[test]
fn image_watermark_paints_behind_body() {
    let resolver = MapResolver::new(vec![("wm.png", png_red_4x4())]);
    let pages = vec![page_with(vec![body_text(common::evolventa())])];
    let opts = image_opts(ImageWatermark {
        name: "wm.png".to_string(),
        opacity: 0.3,
        tiled: false,
    });
    let pdf = emit_pdf_with_images(&pages, &opts, &resolver);
    let watermark = find(&pdf, b"/Im0 Do").expect("watermark drawn");
    let body = find(&pdf, b"0 0 0 rg").expect("body text fill present");
    assert!(
        watermark < body,
        "image watermark must come before body paint"
    );
}

// --------------------------------------------------------------------------
// determinism + structural validity
// --------------------------------------------------------------------------

#[test]
fn watermark_emit_is_byte_deterministic() {
    let resolver = MapResolver::new(vec![("wm.png", png_red_4x4())]);
    let pages = vec![page_with(vec![body_text(common::evolventa())])];
    let opts = image_opts(ImageWatermark {
        name: "wm.png".to_string(),
        opacity: 0.3,
        tiled: true,
    });
    let a = emit_pdf_with_images(&pages, &opts, &resolver);
    let b = emit_pdf_with_images(&pages, &opts, &resolver);
    assert_eq!(a, b, "watermark emit must be byte-deterministic");

    let text = text_opts(TextWatermark::draft(common::evolventa()));
    let c = emit_pdf(&pages, &text);
    let d = emit_pdf(&pages, &text);
    assert_eq!(c, d, "text watermark emit must be byte-deterministic");
}

#[test]
fn qpdf_check_watermarked_pdf_when_available() {
    if !qpdf_available() {
        return;
    }
    let resolver = MapResolver::new(vec![("wm.png", png_red_4x4())]);
    let pages = vec![page_with(vec![body_text(common::evolventa())])];

    let text_pdf = emit_pdf(
        &pages,
        &text_opts(TextWatermark::draft(common::evolventa())),
    );
    assert_qpdf_clean("text", &text_pdf);

    let tiled = image_opts(ImageWatermark {
        name: "wm.png".to_string(),
        opacity: 0.3,
        tiled: true,
    });
    let image_pdf = emit_pdf_with_images(&pages, &tiled, &resolver);
    assert_qpdf_clean("image", &image_pdf);
}

fn qpdf_available() -> bool {
    std::process::Command::new("which")
        .arg("qpdf")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn assert_qpdf_clean(name: &str, pdf: &[u8]) {
    let path = std::env::temp_dir().join(format!("turbo-pdf-wm-{name}.pdf"));
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
