//! Phase 15 `print-color` feature tests (AC-7.x print colour). Only compiled with
//! `--features print-color`. Asserts the emitter writes DeviceCMYK fills (`k`
//! operator) instead of DeviceRGB (`rg`), and that `cmyk(...)` colours parse.

#![cfg(feature = "print-color")]

use turbo_pdf_core::layout::fragment::{Fragment, FragmentContent, NodeId};
use turbo_pdf_core::layout::value::{parse_color, BorderEdges, Rgba};
use turbo_pdf_core::paginate::{Page, PageGeometry};
use turbo_pdf_core::{emit_pdf, EmitOptions, PageKind};

/// A solid-coloured box fragment of the given fill.
fn box_fragment(background: Rgba) -> Fragment {
    Fragment::new(
        NodeId(1),
        10.0,
        10.0,
        100.0,
        50.0,
        FragmentContent::Box {
            background: Some(background),
            border: BorderEdges::default(),
        },
    )
}

fn page_with(body: Vec<Fragment>) -> Page {
    Page {
        geometry: PageGeometry::a4(),
        kind: PageKind::First,
        number: 1,
        body,
        header: Vec::new(),
        footer: Vec::new(),
        footnotes: Vec::new(),
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn fills_are_emitted_as_device_cmyk_not_rgb() {
    let pdf = emit_pdf(
        &[page_with(vec![box_fragment(Rgba::new(255, 0, 0, 255))])],
        &EmitOptions::default(),
    );
    // `set_fill_cmyk` writes "c m y k k"; the trailing " k\n" operator is unique
    // to the CMYK fill path. The RGB " rg" operator must never appear.
    assert!(
        contains(&pdf, b" k\n"),
        "expected a DeviceCMYK fill operator"
    );
    assert!(
        !contains(&pdf, b" rg\n"),
        "no DeviceRGB fill must be emitted under print-color"
    );
}

#[test]
fn pure_red_maps_to_magenta_plus_yellow_ink() {
    // r=255,g=0,b=0 -> k=0, c=0, m=1, y=1.
    let pdf = emit_pdf(
        &[page_with(vec![box_fragment(Rgba::new(255, 0, 0, 255))])],
        &EmitOptions::default(),
    );
    assert!(contains(&pdf, b"0 1 1 0 k"), "red -> 0 1 1 0 CMYK");
}

#[test]
fn pure_black_is_all_key_ink() {
    let pdf = emit_pdf(
        &[page_with(vec![box_fragment(Rgba::BLACK)])],
        &EmitOptions::default(),
    );
    assert!(contains(&pdf, b"0 0 0 1 k"), "black -> 0 0 0 1 CMYK");
}

#[test]
fn pure_white_is_no_ink() {
    let pdf = emit_pdf(
        &[page_with(vec![box_fragment(Rgba::new(255, 255, 255, 255))])],
        &EmitOptions::default(),
    );
    assert!(contains(&pdf, b"0 0 0 0 k"), "white -> 0 0 0 0 CMYK");
}

#[test]
fn cmyk_function_parses_percentages_and_fractions() {
    // Pure cyan ink: cmyk(100%,0,0,0) -> rgb(0,255,255).
    assert_eq!(
        parse_color("cmyk(100%, 0, 0, 0)"),
        Some(Rgba::new(0, 255, 255, 255))
    );
    // Fractional form, key ink only.
    assert_eq!(
        parse_color("cmyk(0, 0, 0, 1)"),
        Some(Rgba::new(0, 0, 0, 255))
    );
    // Slash-separated, magenta.
    assert_eq!(
        parse_color("cmyk(0 / 1 / 0 / 0)"),
        Some(Rgba::new(255, 0, 255, 255))
    );
    // No ink at all -> white.
    assert_eq!(
        parse_color("cmyk(0,0,0,0)"),
        Some(Rgba::new(255, 255, 255, 255))
    );
}

#[test]
fn cmyk_function_rejects_malformed_input() {
    assert_eq!(parse_color("cmyk(1, 2, 3)"), None); // too few components
    assert_eq!(parse_color("cmyk(1, 2, 3, x)"), None); // non-numeric
    assert_eq!(parse_color("cmyk 1,0,0,0"), None); // no parens
}

#[test]
fn cmyk_components_clamp_out_of_range() {
    // Over-range components clamp to [0,1]: 200% -> 1.0 (full ink).
    assert_eq!(
        parse_color("cmyk(200%, 0, 0, 0)"),
        Some(Rgba::new(0, 255, 255, 255))
    );
    // Negative key clamps to 0 (no key ink).
    assert_eq!(
        parse_color("cmyk(0, 0, 0, -1)"),
        Some(Rgba::new(255, 255, 255, 255))
    );
}
