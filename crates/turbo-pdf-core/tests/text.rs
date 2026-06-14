//! Fonts + inline-layout tests (§4.4, §5.2). Fonts are loaded from in-repo
//! fixtures under `assets/fonts/` (never from a system path), so the suite is
//! self-contained and deterministic.

use turbo_pdf_core::text::FontRegistry;
use turbo_pdf_core::{layout_text, Align, FontFace, TextStyle, WhiteSpace};

fn load(name: &str) -> Vec<u8> {
    let path = format!("{}/assets/fonts/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|_| panic!("fixture {path}"))
}

fn evolventa() -> FontFace {
    FontFace::from_bytes(load("Evolventa-zLXL.ttf"), "Evolventa", 400, false).unwrap()
}

fn evolventa_bold() -> FontFace {
    FontFace::from_bytes(load("EvolventaBold-55Xv.ttf"), "Evolventa", 700, false).unwrap()
}

fn evolventa_oblique() -> FontFace {
    FontFace::from_bytes(load("EvolventaOblique-yPLV.ttf"), "Evolventa", 400, true).unwrap()
}

// --------------------------------------------------------------------------
// FontFace
// --------------------------------------------------------------------------

#[test]
fn loads_ttf_and_reads_metrics() {
    let f = evolventa();
    assert_eq!(f.family(), "Evolventa");
    assert_eq!(f.weight(), 400);
    assert!(!f.is_italic());
    assert!(f.ascent_px(16.0) > 0.0);
    assert!(f.descent_px(16.0) > 0.0);
    assert!(f.line_height_px(16.0) > f.ascent_px(16.0));
}

#[test]
fn invalid_font_bytes_return_none() {
    assert!(FontFace::from_bytes(vec![1, 2, 3, 4], "x", 400, false).is_none());
}

#[test]
fn loads_otf_cff_outlines() {
    let f =
        FontFace::from_bytes(load("WarsawGothic-BnBV.otf"), "Warsaw Gothic", 400, false).unwrap();
    assert!(f.has_glyph('A'));
    assert!(!f.shape("Abc").is_empty());
}

#[test]
fn glyph_coverage() {
    let f = evolventa();
    assert!(f.has_glyph('A'));
    assert!(f.glyph_index('A').is_some());
    assert!(!f.has_glyph('\u{10FFFF}'));
}

#[test]
fn measure_scales_with_size_and_letter_spacing() {
    let f = evolventa();
    let w16 = f.measure("Hello", 16.0, 0.0);
    let w32 = f.measure("Hello", 32.0, 0.0);
    assert!(w16 > 0.0);
    assert!((w32 - w16 * 2.0).abs() < 0.5);
    let spaced = f.measure("Hello", 16.0, 4.0);
    assert!(spaced > w16);
}

#[test]
fn shape_produces_glyphs() {
    let glyphs = evolventa().shape("Ab");
    assert_eq!(glyphs.len(), 2);
    assert!(glyphs[0].x_advance > 0);
}

// --------------------------------------------------------------------------
// FontRegistry
// --------------------------------------------------------------------------

#[test]
fn registry_select_by_weight_and_style() {
    let mut reg = FontRegistry::new();
    assert!(reg.is_empty());
    reg.add(evolventa());
    reg.add(evolventa_bold());
    reg.add(evolventa_oblique());
    assert_eq!(reg.len(), 3);

    let bold = reg.select(&["Evolventa"], 700, false).unwrap();
    assert_eq!(bold.weight(), 700);
    let italic = reg.select(&["Evolventa"], 400, true).unwrap();
    assert!(italic.is_italic());
    let regular = reg.select(&["Evolventa"], 400, false).unwrap();
    assert_eq!(regular.weight(), 400);
    assert!(!regular.is_italic());
}

#[test]
fn registry_unknown_family_falls_back_to_first() {
    let mut reg = FontRegistry::new();
    reg.add(evolventa());
    assert!(reg.select(&["Nonexistent"], 400, false).is_some());
    assert!(FontRegistry::new().select(&["x"], 400, false).is_none());
}

#[test]
fn registry_glyph_fallback_chain() {
    let mut reg = FontRegistry::new();
    reg.add(evolventa());
    // requested family does not exist, but another face covers 'A'.
    let face = reg.resolve_glyph(&["Missing"], 400, false, 'A');
    assert!(face.is_some());
    // matched family covers the glyph directly.
    assert!(reg.resolve_glyph(&["Evolventa"], 400, false, 'A').is_some());
    // no face covers this codepoint.
    assert!(reg
        .resolve_glyph(&["Evolventa"], 400, false, '\u{10FFFF}')
        .is_none());
}

// --------------------------------------------------------------------------
// inline layout
// --------------------------------------------------------------------------

fn style(white_space: WhiteSpace, align: Align) -> TextStyle {
    TextStyle {
        white_space,
        align,
        ..Default::default()
    }
}

#[test]
fn wraps_into_multiple_lines() {
    let f = evolventa();
    let s = style(WhiteSpace::Normal, Align::Left);
    let lines = layout_text("aa bb cc dd ee ff gg hh", &f, &s, 60.0);
    assert!(lines.len() > 1);
    for line in &lines {
        assert!(line.width <= 60.5, "line {:?} too wide", line);
        assert_eq!(line.x_offset, 0.0);
    }
}

#[test]
fn short_text_is_one_line() {
    let lines = layout_text("hi", &evolventa(), &TextStyle::default(), 500.0);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].text, "hi");
}

#[test]
fn empty_text_yields_no_lines() {
    assert!(layout_text("   ", &evolventa(), &TextStyle::default(), 100.0).is_empty());
    assert!(layout_text(
        "",
        &evolventa(),
        &style(WhiteSpace::NoWrap, Align::Left),
        100.0
    )
    .is_empty());
}

#[test]
fn collapses_whitespace() {
    let lines = layout_text("a    b", &evolventa(), &TextStyle::default(), 500.0);
    assert_eq!(lines[0].text, "a b");
}

#[test]
fn pre_preserves_explicit_line_breaks() {
    let lines = layout_text(
        "one\ntwo\nthree",
        &evolventa(),
        &style(WhiteSpace::Pre, Align::Left),
        1000.0,
    );
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1].text, "two");
}

#[test]
fn nowrap_never_breaks() {
    let lines = layout_text(
        "aa bb cc dd ee ff gg hh",
        &evolventa(),
        &style(WhiteSpace::NoWrap, Align::Left),
        10.0,
    );
    assert_eq!(lines.len(), 1);
}

#[test]
fn alignment_offsets() {
    let f = evolventa();
    let right = layout_text("hi", &f, &style(WhiteSpace::Normal, Align::Right), 200.0);
    assert!(right[0].x_offset > 0.0);
    let center = layout_text("hi", &f, &style(WhiteSpace::Normal, Align::Center), 200.0);
    assert!(center[0].x_offset > 0.0);
    assert!(center[0].x_offset < right[0].x_offset);
    let justify = layout_text("hi", &f, &style(WhiteSpace::Normal, Align::Justify), 200.0);
    assert_eq!(justify[0].x_offset, 0.0);
}

#[test]
fn line_height_override_is_used() {
    let f = evolventa();
    let s = TextStyle {
        line_height: Some(40.0),
        ..Default::default()
    };
    let lines = layout_text("hi", &f, &s, 500.0);
    assert_eq!(lines[0].height, 40.0);
}
