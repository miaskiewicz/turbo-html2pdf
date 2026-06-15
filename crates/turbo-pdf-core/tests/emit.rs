//! Phase 9 PDF emitter tests (§7, §14). Drives the real fixture corpus through
//! the full pipeline (compile → render → cascade → layout → paginate → emit) and
//! asserts the output is a structurally valid, byte-deterministic PDF 1.7. Also
//! exercises the emitter directly on synthetic pages to cover every paint path
//! (boxes, borders, CFF + TrueType fonts, all metadata fields, the date paths).

mod common;

use turbo_pdf_core::layout::fragment::{
    BreakMeta, Fragment, FragmentContent, NodeId, PositionedGlyph,
};
use turbo_pdf_core::layout::value::{BorderEdges, BorderSide, Edges, Rgba};
use turbo_pdf_core::paginate::{paginate, Page, PageGeometry};
use turbo_pdf_core::style::TokenSet;
use turbo_pdf_core::{
    build_cascade, compile, emit_pdf, layout, style_tree, CompileOptions, Diagnostics, EmitOptions,
    FontFace, StyledNode,
};

const FIXTURES: &[&str] = &["invoice", "report", "legal", "mixed"];

fn fixture_path(name: &str, file: &str) -> String {
    format!(
        "{}/tests/fixtures/{name}/{file}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn read_optional(name: &str, file: &str) -> Option<String> {
    std::fs::read_to_string(fixture_path(name, file)).ok()
}

fn read_required(name: &str, file: &str) -> String {
    let path = fixture_path(name, file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// Run one fixture through the whole pipeline and paginate it into pages.
fn paginate_fixture(name: &str) -> Vec<Page> {
    let template = read_required(name, "template.html");
    let css = read_optional(name, "style.css").unwrap_or_default();
    let data_src = read_required(name, "data.json");
    let data: serde_json::Value =
        serde_json::from_str(&data_src).unwrap_or_else(|e| panic!("parse {name}/data.json: {e}"));

    let (program, _) = compile(&template, &CompileOptions::default())
        .unwrap_or_else(|e| panic!("compile {name}: {:?}", e.code));
    let (nodes, _) = program
        .render_nodes(&data, Some(0))
        .unwrap_or_else(|e| panic!("render {name}: {:?}", e.code));

    let cascade = build_cascade(&css, "", TokenSet::default());
    let styled: Vec<StyledNode> = style_tree(&nodes, &cascade);

    let mut diags = Diagnostics::default();
    let galley = layout(&styled, 540.0, &common::registry(), &mut diags);
    paginate(&galley, &[], &mut diags).expect("paginate")
}

/// Locate `needle` in a byte slice.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Count occurrences of a `/Type /Page` (non-tree) page object marker.
fn count_pages_in_pdf(pdf: &[u8]) -> usize {
    let mut count = 0;
    let marker = b"/Type /Page";
    let mut i = 0;
    while i + marker.len() <= pdf.len() {
        if &pdf[i..i + marker.len()] == marker {
            // Exclude `/Type /Pages` (the tree node) which extends the match.
            let after = pdf.get(i + marker.len());
            if after != Some(&b's') {
                count += 1;
            }
        }
        i += 1;
    }
    count
}

#[test]
fn emits_valid_pdf_for_every_fixture() {
    for &name in FIXTURES {
        let pages = paginate_fixture(name);
        let pdf = emit_pdf(&pages, &EmitOptions::default());
        assert!(pdf.starts_with(b"%PDF-1.7"), "{name}: bad header");
        assert!(contains(&pdf, b"%%EOF"), "{name}: missing EOF");
        assert_eq!(
            count_pages_in_pdf(&pdf),
            pages.len(),
            "{name}: page count mismatch"
        );
    }
}

#[test]
fn emit_is_byte_deterministic() {
    for &name in FIXTURES {
        let pages = paginate_fixture(name);
        let a = emit_pdf(&pages, &EmitOptions::default());
        let b = emit_pdf(&pages, &EmitOptions::default());
        assert_eq!(a, b, "{name}: emit not deterministic");
    }
}

#[test]
fn qpdf_check_when_available() {
    if !qpdf_available() {
        return;
    }
    for &name in FIXTURES {
        let pages = paginate_fixture(name);
        let pdf = emit_pdf(&pages, &EmitOptions::default());
        assert_qpdf_clean(name, &pdf);
    }
}

/// Whether the `qpdf` binary is on `PATH`.
fn qpdf_available() -> bool {
    std::process::Command::new("which")
        .arg("qpdf")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Shell out to `qpdf --check` on a temp file and assert it reports no errors.
fn assert_qpdf_clean(name: &str, pdf: &[u8]) {
    let path = std::env::temp_dir().join(format!("turbo-pdf-emit-{name}.pdf"));
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
// Synthetic-page tests: exercise every paint path the fixtures may not reach.
// --------------------------------------------------------------------------

fn text_fragment(face: FontFace, glyph_ids: &[u16]) -> Fragment {
    let glyphs = glyph_ids
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
            color: Rgba::new(20, 40, 60, 255),
        },
    )
}

fn box_fragment(background: Option<Rgba>, border: BorderEdges) -> Fragment {
    Fragment::new(
        NodeId(2),
        10.0,
        10.0,
        100.0,
        50.0,
        FragmentContent::Box { background, border },
    )
}

/// A single page wrapping the given body fragments with default A4 geometry.
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

fn all_borders() -> BorderEdges {
    let side = BorderSide {
        width: 2,
        color: Some(Rgba::new(200, 0, 0, 255)),
    };
    BorderEdges {
        top: side,
        right: side,
        bottom: side,
        left: BorderSide {
            width: 3,
            color: None,
        },
    }
}

#[test]
fn paints_boxes_and_borders() {
    let bg = box_fragment(Some(Rgba::new(240, 240, 240, 255)), all_borders());
    let plain = box_fragment(None, BorderEdges::default());
    let pdf = emit_pdf(&[page_with(vec![bg, plain])], &EmitOptions::default());
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(contains(&pdf, b"%%EOF"));
}

#[test]
fn embeds_truetype_and_cff_fonts_with_children() {
    let mut child = text_fragment(common::go(), &[5, 9, 5]);
    child.break_meta = BreakMeta::default();
    let mut parent = box_fragment(Some(Rgba::BLACK), BorderEdges::default());
    parent.children.push(child);

    let cff = FontFace::from_bytes(
        common::font_bytes("WarsawGothic-BnBV.otf"),
        "WarsawGothic",
        400,
        false,
    )
    .expect("load CFF font");
    let cff_line = text_fragment(cff, &[3, 4, 5]);

    let pages = vec![page_with(vec![parent, cff_line])];
    let pdf = emit_pdf(&pages, &EmitOptions::default());
    assert!(contains(&pdf, b"/Type0"), "missing Type0 font");
    assert!(contains(&pdf, b"FontFile2"), "missing TrueType program");
    assert!(contains(&pdf, b"FontFile3"), "missing CFF program");
    // Re-emit: same fonts must dedupe to the same bytes.
    let again = emit_pdf(&pages, &EmitOptions::default());
    assert_eq!(pdf, again);
}

#[test]
fn directive_fragments_and_empty_bands_are_skipped() {
    let directive = Fragment::new(
        NodeId(3),
        0.0,
        0.0,
        10.0,
        10.0,
        FragmentContent::Directive(turbo_pdf_core::TKind::Page),
    );
    let pdf = emit_pdf(&[page_with(vec![directive])], &EmitOptions::default());
    assert!(pdf.starts_with(b"%PDF-1.7"));
}

#[test]
fn header_footer_footnote_bands_paint() {
    let mut page = page_with(vec![box_fragment(
        Some(Rgba::BLACK),
        BorderEdges::default(),
    )]);
    page.header.push(text_fragment(common::go(), &[1]));
    page.footer.push(text_fragment(common::go(), &[2]));
    page.footnotes.push(text_fragment(common::go(), &[3]));
    let pdf = emit_pdf(&[page], &EmitOptions::default());
    assert!(contains(&pdf, b"/Type0"));
}

#[test]
fn font_store_reports_emptiness() {
    use turbo_pdf_core::emit::FontStore;
    let empty = FontStore::collect(&[page_with(vec![])]);
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    let withfont = FontStore::collect(&[page_with(vec![text_fragment(common::go(), &[1])])]);
    assert!(!withfont.is_empty());
    assert_eq!(withfont.len(), 1);
}

#[test]
fn unsubsettable_glyphs_fall_back_to_full_embed() {
    // Glyph ids past the font's range make the subsetter reject the font; the
    // emitter must fall back to embedding the original program and still produce
    // a valid PDF.
    let line = text_fragment(common::go(), &[60_000, 65_000]);
    let pdf = emit_pdf(&[page_with(vec![line])], &EmitOptions::default());
    assert!(contains(&pdf, b"FontFile2"), "fallback embed missing");
    assert!(pdf.starts_with(b"%PDF-1.7"));
}

#[test]
fn full_metadata_round_trips() {
    let opts = EmitOptions {
        title: Some("Quarterly Report".to_string()),
        author: Some("Wojtek".to_string()),
        subject: Some("Finance".to_string()),
        keywords: Some("pdf, report".to_string()),
        creation_date: Some(1_700_000_000),
        ..EmitOptions::default()
    };
    let pdf = emit_pdf(&[page_with(vec![])], &opts);
    assert!(contains(&pdf, b"/Title"));
    assert!(contains(&pdf, b"/Author"));
    assert!(contains(&pdf, b"/Subject"));
    assert!(contains(&pdf, b"/Keywords"));
    assert!(contains(&pdf, b"/CreationDate"));
    assert!(contains(&pdf, b"/Producer"));
}

#[test]
fn default_metadata_uses_sentinel_date() {
    let pdf = emit_pdf(&[page_with(vec![])], &EmitOptions::default());
    // 2000-01-01T00:00:00Z sentinel.
    assert!(
        contains(&pdf, b"D:20000101000000Z"),
        "sentinel date missing"
    );
    assert!(!contains(&pdf, b"/Title"), "absent title must be omitted");
}

#[test]
fn out_of_range_date_falls_back_to_sentinel() {
    let opts = EmitOptions {
        creation_date: Some(i64::MAX),
        ..EmitOptions::default()
    };
    let pdf = emit_pdf(&[page_with(vec![])], &opts);
    assert!(contains(&pdf, b"D:20000101000000Z"));
}

#[test]
fn geometry_uses_edges_margins() {
    // Touches the Edges import indirectly via a non-default geometry page.
    let mut page = page_with(vec![]);
    page.geometry.margin = Edges::all(10.0);
    let pdf = emit_pdf(&[page], &EmitOptions::default());
    assert!(pdf.starts_with(b"%PDF-1.7"));
}
