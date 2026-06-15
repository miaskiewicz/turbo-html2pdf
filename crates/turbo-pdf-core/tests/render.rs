//! Phase 7 keystone tests (§3.0, §6.5–6.8): running header/footer regions with
//! per-page late-evaluation of `{{ page.number }}` / `{{ page.total }}` (and the
//! `<t:page/>` / `<t:pages/>` field codes). Each test drives a compiled program
//! through `render_pages`, the higher-layer orchestrator, exactly as a caller
//! would, with the clock pinned (`now = Some(0)`) for determinism.

mod common;

use turbo_pdf_core::layout::fragment::{Fragment, FragmentContent};
use turbo_pdf_core::style::TokenSet;
use turbo_pdf_core::{
    build_cascade, compile, render_pages, style::AtRule, CompileOptions, Diagnostics, LintCode,
    Page, RenderInputs,
};

/// Parse a stylesheet's `@page` at-rules so the orchestrator and the geometry
/// resolver see the same geometry the cascade was built from.
fn at_rules(css: &str) -> Vec<AtRule> {
    turbo_pdf_core::style::parse_stylesheet(css).at_rules
}

/// Drive a template + CSS + data through the full Phase 7 pipeline.
fn pages(template: &str, css: &str, data: serde_json::Value) -> (Vec<Page>, Diagnostics) {
    let (program, cdiags) = compile(template, &CompileOptions::default()).expect("compile");
    assert!(cdiags.is_empty(), "compile diags: {cdiags:?}");
    let cascade = build_cascade(css, "", TokenSet::default());
    let fonts = common::registry();
    let rules = at_rules(css);
    let inputs = RenderInputs {
        program: &program,
        data: &data,
        cascade: &cascade,
        at_rules: &rules,
        fonts: &fonts,
        images: &turbo_pdf_core::NoImages,
        now: Some(0),
    };
    let mut diags = Diagnostics::default();
    let out = render_pages(&inputs, &mut diags).expect("render_pages");
    (out, diags)
}

/// Collect every glyph id painted across a band's fragment forest.
fn band_glyph_ids(band: &[Fragment]) -> Vec<u16> {
    let mut out = Vec::new();
    for frag in band {
        collect_glyphs(frag, &mut out);
    }
    out
}

fn collect_glyphs(frag: &Fragment, out: &mut Vec<u16>) {
    if let FragmentContent::TextLine { glyphs, .. } = &frag.content {
        out.extend(glyphs.iter().map(|g| g.glyph_id));
    }
    for child in &frag.children {
        collect_glyphs(child, out);
    }
}

/// The glyph id for `ch` in the first text face used in a band (the face the
/// footer was actually shaped with), so digit assertions are font-independent.
fn glyph_for(band: &[Fragment], ch: char) -> u16 {
    let face = first_face(band).expect("band has a shaped text line");
    face.glyph_index(ch).expect("face maps the digit")
}

fn first_face(band: &[Fragment]) -> Option<turbo_pdf_core::FontFace> {
    for frag in band {
        if let Some(f) = face_of(frag) {
            return Some(f);
        }
    }
    None
}

fn face_of(frag: &Fragment) -> Option<turbo_pdf_core::FontFace> {
    if let FragmentContent::TextLine { face, .. } = &frag.content {
        return Some(face.clone());
    }
    frag.children.iter().find_map(face_of)
}

/// A body long enough to span several pages on a short page, but kept under ten
/// pages so page numbers stay single-digit (unambiguous digit-glyph assertions).
fn long_body() -> String {
    let mut s = String::new();
    for i in 0..14 {
        s.push_str(&format!(
            "<p>Paragraph number {i} with some flowing body text.</p>"
        ));
    }
    s
}

/// A short page so a modest body paginates to multiple pages, with room in the
/// bottom margin for a footer band.
const SHORT_PAGE: &str = "@page { size: 300px 200px; margin: 30px }";

// --------------------------------------------------------------------------
// keystone: per-page page-number late evaluation (AC-6.7)
// --------------------------------------------------------------------------

#[test]
fn ac_6_7_footer_page_number_is_late_evaluated_per_page() {
    let template = format!(
        "{}<t:running-footer>Page <t:page/> of <t:pages/></t:running-footer>",
        long_body()
    );
    let (out, _) = pages(&template, SHORT_PAGE, serde_json::json!({}));
    assert!(out.len() >= 2, "expected >= 2 pages, got {}", out.len());

    let total = out.len() as u32;
    // Each page's footer is rendered with ITS number and the SAME total.
    for page in &out {
        assert!(!page.footer.is_empty(), "page {} footer empty", page.number);
        let glyphs = band_glyph_ids(&page.footer);
        let g_number = glyph_for(&page.footer, digit(page.number));
        let g_total = glyph_for(&page.footer, digit(total));
        assert!(
            glyphs.contains(&g_number),
            "page {} footer must encode its own number {}",
            page.number,
            page.number
        );
        assert!(
            glyphs.contains(&g_total),
            "page {} footer must encode the total {}",
            page.number,
            total
        );
    }
}

/// The first digit of a small page number (the corpus stays under 10 pages).
fn digit(n: u32) -> char {
    std::char::from_digit(n % 10, 10).expect("0..=9 is a digit")
}

#[test]
fn ac_6_7_page_one_and_two_footers_differ_in_number() {
    let template = format!(
        "{}<t:running-footer>Page <t:page/> of <t:pages/></t:running-footer>",
        long_body()
    );
    let (out, _) = pages(&template, SHORT_PAGE, serde_json::json!({}));
    assert!(out.len() >= 2);
    // The number glyph differs (1 vs 2); the total glyph is shared.
    let one = glyph_for(&out[0].footer, '1');
    let two = glyph_for(&out[1].footer, '2');
    assert!(band_glyph_ids(&out[0].footer).contains(&one));
    assert!(band_glyph_ids(&out[1].footer).contains(&two));
    assert_ne!(one, two);
}

// --------------------------------------------------------------------------
// band reservation + placement (AC-3.0.3, AC-3.0.4)
// --------------------------------------------------------------------------

#[test]
fn ac_3_0_3_footer_band_extent_reserved_from_content() {
    let template = format!(
        "{}<t:running-footer>Footer line</t:running-footer>",
        long_body()
    );
    let (out, _) = pages(&template, SHORT_PAGE, serde_json::json!({}));
    // The bottom band is reserved (extent > 0) and capped at the bottom margin.
    let geo = out[0].geometry;
    assert!(geo.footer_extent > 0.0, "footer band not reserved");
    assert!(geo.footer_extent <= geo.margin.bottom);
}

#[test]
fn ac_3_0_3_header_band_reserved_and_painted() {
    let template = format!(
        "<t:running-header>Doc header</t:running-header>{}",
        long_body()
    );
    let (out, _) = pages(&template, SHORT_PAGE, serde_json::json!({}));
    let geo = out[0].geometry;
    assert!(geo.header_extent > 0.0, "header band not reserved");
    for page in &out {
        assert!(!page.header.is_empty(), "page {} header empty", page.number);
        // Header sits inside the top margin.
        let top = page.header[0].y;
        assert!(top >= geo.margin.top - 0.5 && top < geo.margin.top + geo.header_extent + 0.5);
    }
}

#[test]
fn reserving_a_band_lowers_body_capacity() {
    // Same body: with a footer band the body has less room, so >= as many pages.
    let body = long_body();
    let plain = pages(&body, SHORT_PAGE, serde_json::json!({})).0;
    let withf = pages(
        &format!("{body}<t:running-footer>F</t:running-footer>"),
        SHORT_PAGE,
        serde_json::json!({}),
    )
    .0;
    assert!(withf.len() >= plain.len());
    assert!(withf[0].geometry.body_height() < plain[0].geometry.body_height());
}

// --------------------------------------------------------------------------
// data + page state together (§6.6)
// --------------------------------------------------------------------------

#[test]
fn footer_can_interpolate_document_data_and_page_state() {
    let template = format!(
        "{}<t:running-footer>{{{{ data.doc }}}} p<t:page/></t:running-footer>",
        long_body()
    );
    let (out, _) = pages(&template, SHORT_PAGE, serde_json::json!({ "doc": "ACME" }));
    // The footer renders both the data field and the page number without error.
    for page in &out {
        assert!(!page.footer.is_empty());
    }
    let g = glyph_for(&out[0].footer, 'A');
    assert!(band_glyph_ids(&out[0].footer).contains(&g));
}

#[test]
fn page_context_exposes_is_first_and_is_last() {
    // A footer that only prints on the last page proves is_last late-evaluates.
    let template = format!(
        "{}<t:running-footer>{{% if page.is_last %}}END{{% endif %}}</t:running-footer>",
        long_body()
    );
    let (out, _) = pages(&template, SHORT_PAGE, serde_json::json!({}));
    let last = out.len() - 1;
    // Last page footer carries the END glyphs; an interior page footer does not.
    let end_e = glyph_for(&out[last].footer, 'E');
    assert!(band_glyph_ids(&out[last].footer).contains(&end_e));
    assert!(band_glyph_ids(&out[0].footer).is_empty());
}

// --------------------------------------------------------------------------
// no-region path stays a no-op (mirrors switch::desugar)
// --------------------------------------------------------------------------

#[test]
fn no_region_leaves_bands_empty() {
    let (out, _) = pages(&long_body(), SHORT_PAGE, serde_json::json!({}));
    for page in &out {
        assert!(page.header.is_empty());
        assert!(page.footer.is_empty());
    }
    assert_eq!(out[0].geometry.header_extent, 0.0);
    assert_eq!(out[0].geometry.footer_extent, 0.0);
}

// --------------------------------------------------------------------------
// region overflow clipped + linted (AC-6.8)
// --------------------------------------------------------------------------

#[test]
fn ac_6_8_region_taller_than_band_is_clipped_and_linted() {
    // A footer with a large font produces content taller than the band cap; the
    // band is capped at the bottom margin and the overflow is linted.
    let css = "@page { size: 300px 200px; margin: 12px } .big { font-size: 40px }";
    let template = format!(
        "{}<t:running-footer><div class=\"big\">TALL</div></t:running-footer>",
        long_body()
    );
    let (out, diags) = pages(&template, css, serde_json::json!({}));
    assert!(out[0].geometry.footer_extent <= out[0].geometry.margin.bottom + 0.01);
    assert!(diags
        .lints
        .iter()
        .any(|l| l.code == LintCode::RegionOverflow));
}

// --------------------------------------------------------------------------
// determinism
// --------------------------------------------------------------------------

#[test]
fn render_pages_is_deterministic() {
    let template = format!(
        "{}<t:running-footer>Page <t:page/> of <t:pages/></t:running-footer>",
        long_body()
    );
    let a = pages(&template, SHORT_PAGE, serde_json::json!({})).0;
    let b = pages(&template, SHORT_PAGE, serde_json::json!({})).0;
    assert_eq!(a.len(), b.len());
    for (pa, pb) in a.iter().zip(&b) {
        assert_eq!(band_glyph_ids(&pa.footer), band_glyph_ids(&pb.footer));
    }
}

// --------------------------------------------------------------------------
// program region presence accessors
// --------------------------------------------------------------------------

#[test]
fn program_reports_which_regions_are_present() {
    let (header_only, _) = compile(
        "<t:running-header>H</t:running-header><p>x</p>",
        &CompileOptions::default(),
    )
    .expect("compile");
    assert!(header_only.has_header());
    assert!(!header_only.has_footer());

    let (footer_only, _) = compile(
        "<p>x</p><t:running-footer>F</t:running-footer>",
        &CompileOptions::default(),
    )
    .expect("compile");
    assert!(!footer_only.has_header());
    assert!(footer_only.has_footer());
}

#[test]
fn render_region_returns_none_for_absent_region() {
    // A template with no footer: render_region(FOOTER, …) is None (no panic).
    let (program, _) = compile("<p>only body</p>", &CompileOptions::default()).expect("compile");
    let ctx = serde_json::json!({ "page": { "number": 1, "total": 1 } });
    assert!(program.render_region("__footer__", &ctx, Some(0)).is_none());
}
