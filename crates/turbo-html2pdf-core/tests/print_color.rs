//! Phase 15 `print-color` feature tests (AC-7.x print colour). Only compiled with
//! `--features print-color`. Asserts the emitter writes DeviceCMYK fills (`k`
//! operator) instead of DeviceRGB (`rg`), and that `cmyk(...)` colours parse.
//!
//! Split at the S1 relocation: the layout-only cases moved to turbo-dom-layout's
//! `tests/print_color.rs`; the ones here need TKind directives, templating or the PDF
//! emitter.

#![cfg(feature = "print-color")]

use turbo_html2pdf_core::layout::fragment::{Fragment, FragmentContent, NodeId};
use turbo_html2pdf_core::layout::value::{BorderEdges, Rgba};
use turbo_html2pdf_core::paginate::{Page, PageGeometry};
use turbo_html2pdf_core::{emit_pdf, EmitOptions, PageKind};

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
            border_radius: 0.0,
            shadow: None,
            gradient: None,
            transform: None,
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
        semantics: Default::default(),
        canvas: None,
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// `EmitOptions` with the per-render CMYK toggle on.
fn cmyk_opts() -> EmitOptions {
    EmitOptions {
        cmyk: true,
        ..EmitOptions::default()
    }
}

#[test]
fn fills_are_emitted_as_device_cmyk_not_rgb() {
    let pdf = emit_pdf(
        &[page_with(vec![box_fragment(Rgba::new(255, 0, 0, 255))])],
        &cmyk_opts(),
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
        &cmyk_opts(),
    );
    assert!(contains(&pdf, b"0 1 1 0 k"), "red -> 0 1 1 0 CMYK");
}

#[test]
fn pure_black_is_all_key_ink() {
    let pdf = emit_pdf(&[page_with(vec![box_fragment(Rgba::BLACK)])], &cmyk_opts());
    assert!(contains(&pdf, b"0 0 0 1 k"), "black -> 0 0 0 1 CMYK");
}

#[test]
fn pure_white_is_no_ink() {
    let pdf = emit_pdf(
        &[page_with(vec![box_fragment(Rgba::new(255, 255, 255, 255))])],
        &cmyk_opts(),
    );
    assert!(contains(&pdf, b"0 0 0 0 k"), "white -> 0 0 0 0 CMYK");
}

#[test]
fn cmyk_false_is_byte_identical_device_rgb_under_print_color_build() {
    // The per-render toggle is OFF: even compiled with `print-color`, the output
    // must be the byte-for-byte DeviceRGB fill — no CMYK `k` operator, a `rg`
    // operator instead — proving the feature gates capability, not behaviour.
    let page = page_with(vec![box_fragment(Rgba::new(255, 0, 0, 255))]);
    let pdf = emit_pdf(&[page], &EmitOptions::default());
    assert!(
        contains(&pdf, b" rg\n"),
        "cmyk:false must emit a DeviceRGB fill operator"
    );
    assert!(
        !contains(&pdf, b" k\n"),
        "cmyk:false must emit no DeviceCMYK fill operator"
    );
    // Pure red in DeviceRGB is `1 0 0 rg`.
    assert!(contains(&pdf, b"1 0 0 rg"), "red -> 1 0 0 DeviceRGB");
}

mod common;

/// The text-line colours and box backgrounds of the rendered pages' bodies.
fn painted(frags: &[Fragment], out: &mut Vec<Rgba>) {
    for f in frags {
        match &f.content {
            FragmentContent::TextLine { color, .. } => out.push(*color),
            FragmentContent::Box {
                background: Some(c),
                ..
            } => out.push(*c),
            _ => {}
        }
        painted(&f.children, out);
    }
}

#[test]
fn a_rendered_document_accepts_cmyk_colours() {
    use turbo_html2pdf_core::style::TokenSet;
    use turbo_html2pdf_core::{
        build_cascade, compile, render_pages, CompileOptions, Diagnostics, RenderInputs,
    };
    let template = r#"<!DOCTYPE html><html><head><style>.c{color:cmyk(0,1,1,0)}</style></head><body><p class="c">x</p><div style="height:10px;background:cmyk(1,0,1,0)"></div></body></html>"#;
    let (program, _) = compile(template, &CompileOptions::default()).expect("compile");
    let cascade = build_cascade("", "", TokenSet::default());
    let fonts = common::registry();
    let data = serde_json::Value::Null;
    let inputs = RenderInputs {
        program: &program,
        data: &data,
        cascade: &cascade,
        at_rules: &[],
        fonts: &fonts,
        images: &turbo_html2pdf_core::NoImages,
        now: Some(0),
    };
    let pages = render_pages(&inputs, &mut Diagnostics::default()).expect("render");
    let mut colours = Vec::new();
    painted(&pages[0].body, &mut colours);
    assert_eq!(
        colours,
        [Rgba::new(255, 0, 0, 255), Rgba::new(0, 255, 0, 255)]
    );
}
