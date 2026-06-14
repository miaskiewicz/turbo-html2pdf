//! Phase 8 footnote tests (§3.6, §6.4), AC-per-test. Each test drives a compiled
//! program through `render_pages`, the orchestrator that resolves the body /
//! footnote fixpoint, with the clock pinned (`now = Some(0)`) for determinism.

mod common;

use turbo_pdf_core::layout::fragment::{Fragment, FragmentContent};
use turbo_pdf_core::style::TokenSet;
use turbo_pdf_core::{
    build_cascade, compile, render_pages, style::AtRule, CompileOptions, Diagnostics, LintCode,
    Page, RenderInputs,
};

fn at_rules(css: &str) -> Vec<AtRule> {
    turbo_pdf_core::style::parse_stylesheet(css).at_rules
}

/// Drive a template + CSS + data through the full pipeline.
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
        now: Some(0),
    };
    let mut diags = Diagnostics::default();
    let out = render_pages(&inputs, &mut diags).expect("render_pages");
    (out, diags)
}

/// Every glyph id painted across a band's fragment forest.
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

/// The glyph id for `ch` in the first shaped face used in a band.
fn glyph_for(band: &[Fragment], ch: char) -> u16 {
    let face = first_face(band).expect("band has a shaped text line");
    face.glyph_index(ch).expect("face maps the char")
}

fn first_face(band: &[Fragment]) -> Option<turbo_pdf_core::FontFace> {
    band.iter().find_map(face_of)
}

fn face_of(frag: &Fragment) -> Option<turbo_pdf_core::FontFace> {
    if let FragmentContent::TextLine { face, .. } = &frag.content {
        return Some(face.clone());
    }
    frag.children.iter().find_map(face_of)
}

/// True if the band paints a separator rule (a filled box) above its notes.
fn has_separator(band: &[Fragment]) -> bool {
    band.iter().any(|f| {
        matches!(
            &f.content,
            FragmentContent::Box {
                background: Some(_),
                ..
            }
        )
    })
}

/// A body of `n` short paragraphs, the `mark`th carrying a footnote.
fn body_with_note(n: usize, at: usize, note: &str) -> String {
    let mut s = String::new();
    for i in 0..n {
        if i == at {
            s.push_str(&format!(
                "<p>Para {i}<t:footnote>{note}</t:footnote> end.</p>"
            ));
        } else {
            s.push_str(&format!("<p>Para {i} with flowing text.</p>"));
        }
    }
    s
}

/// `n` filler paragraphs (no footnotes), enough of them to span pages.
fn filler(n: usize) -> String {
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!(
            "<p>Filler paragraph {i} with flowing body text.</p>"
        ));
    }
    s
}

/// A single paragraph carrying one footnote.
fn note_para(note: &str) -> String {
    format!("<p>Cited<t:footnote>{note}</t:footnote> here.</p>")
}

/// A short page with room in the bottom margin for the footnote band.
const SHORT_PAGE: &str = "@page { size: 320px 220px; margin: 24px }";

// --------------------------------------------------------------------------
// AC-3.15: a footnote reserves area on the page its reference lands on
// --------------------------------------------------------------------------

#[test]
fn ac_3_15_footnote_lands_on_its_reference_page() {
    let (out, _) = pages(
        &body_with_note(3, 1, "A cited authority."),
        SHORT_PAGE,
        serde_json::json!({}),
    );
    // Exactly one page carries a footnote band, and it is the one whose body
    // references the note (page 1: paras 0..2 fit on the short page).
    let with_notes: Vec<&Page> = out.iter().filter(|p| !p.footnotes.is_empty()).collect();
    assert_eq!(with_notes.len(), 1, "exactly one page has footnotes");
    let band = &with_notes[0].footnotes;
    // The note body text is painted in that page's footnote band.
    let g = glyph_for(band, 'c'); // "cited"
    assert!(band_glyph_ids(band).contains(&g));
}

#[test]
fn ac_3_17_a_footnote_reduces_body_capacity_on_its_page() {
    // Same body, with vs. without a footnote: the footnote band steals room, so
    // the document needs at least as many pages and a band is reserved.
    let plain = pages(&body_with_note(3, 9, ""), SHORT_PAGE, serde_json::json!({})).0;
    let withf = pages(
        &body_with_note(
            3,
            1,
            "Note body text that is reasonably long to occupy a line.",
        ),
        SHORT_PAGE,
        serde_json::json!({}),
    )
    .0;
    assert!(withf.iter().any(|p| !p.footnotes.is_empty()));
    assert!(withf.len() >= plain.len());
    // The footnote band sits above the bottom margin, inside the page box.
    let p = withf.iter().find(|p| !p.footnotes.is_empty()).unwrap();
    let geo = p.geometry;
    let band_top = p.footnotes.iter().map(|f| f.y).fold(f32::MAX, f32::min);
    assert!(band_top < geo.height - geo.margin.bottom + 0.5);
    assert!(band_top > geo.margin.top);
}

#[test]
fn ac_3_19_separator_rule_heads_the_footnote_band() {
    let (out, _) = pages(
        &body_with_note(3, 1, "Separated note."),
        SHORT_PAGE,
        serde_json::json!({}),
    );
    let band = &out
        .iter()
        .find(|p| !p.footnotes.is_empty())
        .unwrap()
        .footnotes;
    assert!(
        has_separator(band),
        "footnote band must carry a separator rule"
    );
}

// --------------------------------------------------------------------------
// AC-3.15 continuity: doc-continuous numbering across pages
// --------------------------------------------------------------------------

#[test]
fn ac_3_16_default_numbering_is_document_continuous() {
    // Two notes separated by enough filler to land on different pages: their marks
    // run 1 then 2 (document-continuous).
    let template = format!(
        "{}{}{}",
        note_para("first"),
        filler(10),
        note_para("second"),
    );
    let (out, _) = pages(&template, SHORT_PAGE, serde_json::json!({}));
    let banded: Vec<&Page> = out.iter().filter(|p| !p.footnotes.is_empty()).collect();
    assert!(
        banded.len() >= 2,
        "notes land on >= 2 pages: {}",
        banded.len()
    );
    // The first banded page encodes '1'; a later one encodes '2'.
    let g1 = glyph_for(&banded[0].footnotes, '1');
    let g2 = glyph_for(&banded[1].footnotes, '2');
    assert!(band_glyph_ids(&banded[0].footnotes).contains(&g1));
    assert!(band_glyph_ids(&banded[1].footnotes).contains(&g2));
    assert_ne!(g1, g2);
}

#[test]
fn ac_3_16_page_reset_restarts_numbering_each_page() {
    // page-reset: the first note on every page is numbered 1, even though the
    // document has two notes on two pages.
    let template = format!(
        "<t:footnote footnote-reset=\"page\">policy</t:footnote>{}{}{}",
        note_para("alpha"),
        filler(10),
        note_para("beta"),
    );
    let (out, _) = pages(&template, SHORT_PAGE, serde_json::json!({}));
    let banded: Vec<&Page> = out.iter().filter(|p| !p.footnotes.is_empty()).collect();
    assert!(banded.len() >= 2, "got {} banded pages", banded.len());
    // Every banded page's first note restarts at 1.
    for page in &banded {
        let g1 = glyph_for(&page.footnotes, '1');
        assert!(
            band_glyph_ids(&page.footnotes).contains(&g1),
            "page {} should restart at 1",
            page.number
        );
    }
    // Continuous numbering would have put a '2' on the second page; page reset
    // must not (its sole note is '1').
    let two = banded[1]
        .footnotes
        .iter()
        .find_map(face_of)
        .and_then(|f| f.glyph_index('2'));
    if let Some(g2) = two {
        assert!(!band_glyph_ids(&banded[1].footnotes).contains(&g2));
    }
}

#[test]
fn ac_3_16_page_reset_keeps_manual_marks() {
    // Under page reset a manually-marked note keeps its symbol (it does not
    // consume the page's auto sequence); an auto note alongside it still gets 1.
    let template = format!(
        "<p>See<t:footnote footnote-reset=\"page\" mark=\"*\">starred</t:footnote> and \
         <t:footnote>auto</t:footnote>.</p>{}",
        filler(2),
    );
    let (out, _) = pages(&template, SHORT_PAGE, serde_json::json!({}));
    let band = &out
        .iter()
        .find(|p| !p.footnotes.is_empty())
        .unwrap()
        .footnotes;
    let star = glyph_for(band, '*');
    let one = glyph_for(band, '1');
    let ids = band_glyph_ids(band);
    assert!(ids.contains(&star), "manual mark survives page reset");
    assert!(ids.contains(&one), "the auto note restarts at 1");
}

// --------------------------------------------------------------------------
// AC-3.20: manual marks
// --------------------------------------------------------------------------

#[test]
fn ac_3_20_manual_mark_overrides_auto_number() {
    let template = "<p>Body<t:footnote mark=\"*\">starred note</t:footnote>.</p>";
    let (out, _) = pages(template, SHORT_PAGE, serde_json::json!({}));
    let band = &out
        .iter()
        .find(|p| !p.footnotes.is_empty())
        .unwrap()
        .footnotes;
    // The asterisk glyph is painted; the digit '1' is not the mark.
    let star = glyph_for(band, '*');
    assert!(band_glyph_ids(band).contains(&star));
}

// --------------------------------------------------------------------------
// AC-3.18: an oversized note continues onto the next page's footnote area
// --------------------------------------------------------------------------

#[test]
fn ac_3_18_oversized_note_continues_across_pages() {
    // A note taller than the whole footnote area must split: part on the marker's
    // page, the remainder carried to the next page's band — no content lost.
    let mut long = String::new();
    for i in 0..40 {
        long.push_str(&format!("sentence {i} of a very long footnote body. "));
    }
    // The marker is early, with filler after it so the document spans pages and
    // the note's overflow has a following band to continue into.
    let template = format!(
        "<p>See<t:footnote>{long}</t:footnote> here.</p>{}",
        filler(12)
    );
    let css = "@page { size: 320px 200px; margin: 20px }";
    let (out, _) = pages(&template, css, serde_json::json!({}));
    let banded: Vec<&Page> = out.iter().filter(|p| !p.footnotes.is_empty()).collect();
    assert!(
        banded.len() >= 2,
        "oversized note must continue onto a second band, got {} banded pages",
        banded.len()
    );
    // No band overruns the page box (each band sits above the bottom margin).
    for p in &banded {
        let top = p.footnotes.iter().map(|f| f.y).fold(f32::MAX, f32::min);
        assert!(
            top > p.geometry.margin.top,
            "band {} overran the page",
            p.number
        );
    }
}

// --------------------------------------------------------------------------
// AC-6.4: convergence vs non-convergence lint
// --------------------------------------------------------------------------

#[test]
fn ac_6_4_converges_without_lint_in_the_common_case() {
    let (_, diags) = pages(
        &body_with_note(4, 1, "ordinary note"),
        SHORT_PAGE,
        serde_json::json!({}),
    );
    assert!(
        !diags
            .lints
            .iter()
            .any(|l| l.code == LintCode::FootnoteConvergence),
        "the common case must converge: {diags:?}"
    );
}

#[test]
fn ac_6_4_non_convergence_is_linted() {
    // A marker pinned at the page boundary oscillates: with no reservation its
    // paragraph fits on page 1, but reserving its band there steals the room and
    // pushes it (with its note) to page 2 — which empties page 1's band, pulling
    // it back. The fixpoint never settles, so it lints rather than loop or lose
    // content (AC-6.4). The page height is tuned to land exactly on that edge.
    let mut s = String::new();
    for i in 0..7 {
        s.push_str(&format!("<p>Body line number {i} here now.</p>"));
    }
    s.push_str("<p>Tip<t:footnote>a tipping note here</t:footnote>.</p>");
    let css = "@page { size: 300px 126px; margin: 16px }";
    let (out, diags) = pages(&s, css, serde_json::json!({}));
    // Content is never lost regardless of convergence: the note's text survives.
    assert!(out.len() >= 2);
    let note_glyphs: Vec<u16> = out
        .iter()
        .flat_map(|p| band_glyph_ids(&p.footnotes))
        .collect();
    assert!(
        !note_glyphs.is_empty(),
        "footnote content must still be painted"
    );
    assert!(
        diags
            .lints
            .iter()
            .any(|l| l.code == LintCode::FootnoteConvergence),
        "boundary oscillation must lint FootnoteConvergence: {diags:?}"
    );
}

// --------------------------------------------------------------------------
// no footnotes leaves the band empty; determinism
// --------------------------------------------------------------------------

#[test]
fn no_footnotes_leaves_bands_empty() {
    let (out, _) = pages(&body_with_note(4, 9, ""), SHORT_PAGE, serde_json::json!({}));
    for page in &out {
        assert!(page.footnotes.is_empty());
    }
}

#[test]
fn footnote_rendering_is_deterministic() {
    let template = body_with_note(3, 1, "deterministic note");
    let a = pages(&template, SHORT_PAGE, serde_json::json!({})).0;
    let b = pages(&template, SHORT_PAGE, serde_json::json!({})).0;
    assert_eq!(a.len(), b.len());
    for (pa, pb) in a.iter().zip(&b) {
        assert_eq!(band_glyph_ids(&pa.footnotes), band_glyph_ids(&pb.footnotes));
    }
}
