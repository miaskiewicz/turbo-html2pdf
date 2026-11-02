//! miaskiewicz/turbo-html2pdf#55: CSS `opacity` (miaskiewicz/turbo-dom#206) paints
//! through an `ExtGState` with `/ca` and `/CA`, on a page that declares a transparency
//! group, as Chrome 156 `page.pdf()` does (headless: `/ca .5`, page
//! `/Group <</S /Transparency>>`, the 50% red block rasterized as #ff8080 / #ff7f7f);
//! an `opacity: 0` subtree is not painted at all (Chrome draws nothing and extracts no
//! text). Nested opacities multiply. A PDF/A render, which emits no transparency,
//! paints the visible boxes opaque.

mod common;

use turbo_html2pdf_core::style::TokenSet;
use turbo_html2pdf_core::{
    build_cascade, compile, emit_pdf, render_pages, CompileOptions, Diagnostics, EmitOptions,
    RenderInputs,
};

fn pdf_with(body: &str, opts: &EmitOptions) -> Vec<u8> {
    let html = format!("<!DOCTYPE html><html><head><style>@page{{size:600px 400px;margin:40px}}body{{margin:0;font-family:Evolventa;font-size:12px}}</style></head><body>{body}</body></html>");
    let (program, _) = compile(&html, &CompileOptions::default()).expect("compile");
    let cascade = build_cascade("", "", TokenSet::default());
    let fonts = common::registry();
    let inputs = RenderInputs {
        program: &program,
        data: &serde_json::Value::Null,
        cascade: &cascade,
        at_rules: &[],
        fonts: &fonts,
        images: &turbo_html2pdf_core::NoImages,
        now: Some(0),
    };
    let pages = render_pages(&inputs, &mut Diagnostics::default()).expect("render_pages");
    emit_pdf(&pages, opts)
}

fn pdf(body: &str) -> Vec<u8> {
    pdf_with(body, &EmitOptions::default())
}

fn count(hay: &[u8], needle: &[u8]) -> usize {
    hay.windows(needle.len()).filter(|w| *w == needle).count()
}

const RED: &str = r#"<div style="height:50px;background:#ff0000"></div>"#;

#[test]
fn a_translucent_box_paints_through_an_alpha_ext_g_state() {
    let pdf = pdf(&format!(r#"<div style="opacity:0.5">{RED}</div>"#));
    assert_eq!(
        count(&pdf, b"/GSo500 <<\n"),
        1,
        "{}",
        String::from_utf8_lossy(&pdf)
    );
    assert_eq!(count(&pdf, b"/ca 0.5"), 1);
    assert_eq!(count(&pdf, b"/CA 0.5"), 1);
    assert_eq!(count(&pdf, b"/S /Transparency"), 1);
    // The red fill sits between `q /GSo500 gs` and its `Q`.
    let text = String::from_utf8_lossy(&pdf);
    let start = text.find("q\n/GSo500 gs\n").expect("group opened");
    let red = text.find("1 0 0 rg").expect("red painted");
    let end = start + text[start..].find("\nQ").expect("group closed");
    assert!(start < red && red < end);
}

#[test]
fn an_opaque_document_declares_no_transparency() {
    let pdf = pdf(RED);
    assert_eq!(count(&pdf, b" gs\n"), 0);
    assert_eq!(count(&pdf, b"/S /Transparency"), 0);
    let pdf = pdf_with(
        &format!(r#"<div style="opacity:1">{RED}</div>"#),
        &EmitOptions::default(),
    );
    assert_eq!(count(&pdf, b" gs\n"), 0);
}

#[test]
fn nested_opacities_multiply() {
    let pdf = pdf(&format!(
        r#"<div style="opacity:0.5"><div style="opacity:0.5">{RED}</div></div>"#
    ));
    assert_eq!(count(&pdf, b"/GSo250 gs"), 1);
    assert_eq!(count(&pdf, b"/ca 0.25"), 1);
}

#[test]
fn an_invisible_subtree_is_not_painted() {
    let pdf = pdf(
        r#"<div style="opacity:0"><div style="height:50px;background:#0000ff">Zebracorn</div></div><div style="height:50px;background:#00ff00"></div>"#,
    );
    assert_eq!(count(&pdf, b"0 0 1 rg"), 0, "blue skipped");
    assert_eq!(count(&pdf, b"\nBT\n"), 0, "its text skipped");
    assert_eq!(count(&pdf, b"0 1 0 rg"), 1, "the next box still paints");
}

#[cfg(feature = "pdf-a")]
#[test]
fn a_pdf_a_render_paints_translucent_boxes_opaque() {
    let opts = EmitOptions {
        pdf_a: true,
        ..EmitOptions::default()
    };
    let pdf = pdf_with(&format!(r#"<div style="opacity:0.5">{RED}</div>"#), &opts);
    assert_eq!(count(&pdf, b" gs\n"), 0);
    assert_eq!(count(&pdf, b"/S /Transparency"), 0);
    assert_eq!(count(&pdf, b"1 0 0 rg"), 1);
}

#[cfg(feature = "pdf-ua")]
#[test]
fn a_tagged_render_skips_invisible_text_in_the_structure_too() {
    let opts = EmitOptions {
        pdf_ua: true,
        ..EmitOptions::default()
    };
    let pdf = pdf_with(r#"<p style="opacity:0">Hidden</p><p>Shown</p>"#, &opts);
    assert_eq!(count(&pdf, b"/MCID 0"), 1);
    assert_eq!(count(&pdf, b"/MCID 1"), 0);
}
