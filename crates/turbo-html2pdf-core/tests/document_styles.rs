//! miaskiewicz/turbo-html2pdf#29: the render path styles the whole document as Chrome
//! does — `<head>` (and body) `<style>` sheets are author CSS ahead of the caller's
//! `css`, `<html>`/`<body>` are real boxes (UA `body { margin: 8px }`), `rem` resolves
//! against the root font size, `<style>`/`<script>` are never painted, the document's
//! `@page` rules set the geometry, and the root/body background is painted as the
//! page canvas. The geometry semantics are Chrome-controlled in turbo-dom-layout
//! (`tests/html_document_chrome.rs`, `html_document_chrome156.json`); these tests pin
//! the render pipeline onto them.

mod common;

use turbo_html2pdf_core::layout::fragment::{Fragment, FragmentContent};
use turbo_html2pdf_core::layout::value::Rgba;
use turbo_html2pdf_core::style::TokenSet;
use turbo_html2pdf_core::{
    build_cascade, compile, emit_pdf, render_pages, CompileOptions, Diagnostics, EmitOptions, Page,
    RenderInputs,
};

fn pages(template: &str, css: &str) -> Vec<Page> {
    let (program, _) = compile(template, &CompileOptions::default()).expect("compile");
    let cascade = build_cascade(css, "", TokenSet::default());
    let fonts = common::registry();
    let rules = turbo_html2pdf_core::style::parse_stylesheet(css).at_rules;
    let data = serde_json::Value::Null;
    let inputs = RenderInputs {
        program: &program,
        data: &data,
        cascade: &cascade,
        at_rules: &rules,
        fonts: &fonts,
        images: &turbo_html2pdf_core::NoImages,
        now: Some(0),
    };
    render_pages(&inputs, &mut Diagnostics::default()).expect("render_pages")
}

fn text_lines(frags: &[Fragment], out: &mut Vec<(f32, f32, Rgba)>) {
    for f in frags {
        if let FragmentContent::TextLine { color, .. } = &f.content {
            out.push((f.x, f.y, *color));
        }
        text_lines(&f.children, out);
    }
}

fn lines(page: &Page) -> Vec<(f32, f32, Rgba)> {
    let mut out = Vec::new();
    text_lines(&page.body, &mut out);
    out
}

const DOC: &str = "<!DOCTYPE html><html><head>HEAD</head><body>BODY</body></html>";

fn doc(head: &str, body: &str) -> String {
    DOC.replace("HEAD", head).replace("BODY", body)
}

#[test]
fn head_styles_apply_and_body_has_its_ua_margin() {
    let page = &pages(
        &doc("<style>p{color:#c00000;margin:0}</style>", "<p>x</p>"),
        "",
    )[0];
    let lines = lines(page);
    assert_eq!(lines.len(), 1, "only the paragraph paints: {lines:?}");
    let (x, _, color) = lines[0];
    assert_eq!(color, Rgba::new(0xc0, 0, 0, 255), "the head rule applies");
    assert_eq!(x - page.geometry.margin.left, 8.0, "body's 8px UA margin");
}

#[test]
fn caller_css_follows_the_documents_sheets() {
    let page = &pages(
        &doc(
            "<style>p{color:#c00000}</style>",
            "<style>p{color:#00c000}</style><p>x</p>",
        ),
        "p{color:#0000c0} body{margin:0}",
    )[0];
    let lines = lines(page);
    assert_eq!(lines.len(), 1, "body <style> text is never painted");
    assert_eq!(lines[0].2, Rgba::new(0, 0, 0xc0, 255));
    assert_eq!(lines[0].0, page.geometry.margin.left);
}

#[test]
fn rem_resolves_against_the_root_font_size() {
    let page = &pages(
        &doc(
            "<style>html{font-size:20px} p{margin:0 0 0 2rem}</style>",
            "<p>x</p>",
        ),
        "",
    )[0];
    assert_eq!(lines(page)[0].0 - page.geometry.margin.left, 8.0 + 40.0);
}

#[test]
fn document_page_rules_set_the_geometry() {
    let a5 = &pages(&doc("<style>@page{size:A5}</style>", "<p>x</p>"), "")[0];
    let a4 = &pages(&doc("", "<p>x</p>"), "")[0];
    assert!(a5.geometry.width < a4.geometry.width);
    // A caller `@page` rule takes precedence over the document's.
    let caller = &pages(
        &doc("<style>@page{size:A5}</style>", "<p>x</p>"),
        "@page{size:A4}",
    )[0];
    assert_eq!(caller.geometry.width, a4.geometry.width);
}

#[test]
fn forced_breaks_inside_the_body_still_paginate() {
    let pages = pages(
        &doc(
            "",
            "<p>one</p><div style=\"break-before:page\"><p>two</p></div>",
        ),
        "",
    );
    assert_eq!(
        pages.len(),
        2,
        "html/body boxes are dissolved into the page flow"
    );
}

#[test]
fn the_canvas_is_the_root_or_body_background() {
    let body = &pages(
        &doc("<style>body{background:#123456}</style>", "<p>x</p>"),
        "",
    )[0];
    assert_eq!(body.canvas, Some(Rgba::new(0x12, 0x34, 0x56, 255)));
    let html = &pages(
        &doc(
            "<style>html{background:#abcdef} body{background:#123456}</style>",
            "<p>x</p>",
        ),
        "",
    )[0];
    assert_eq!(html.canvas, Some(Rgba::new(0xab, 0xcd, 0xef, 255)));
    let none = &pages(&doc("", "<p>x</p>"), "")[0];
    assert_eq!(none.canvas, None);
}

/// Locate `needle` in a byte slice.
fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn the_canvas_paints_the_page_area_composited_on_white() {
    let mut page = pages(&doc("", ""), "").remove(0);
    // An 800x600px page with four different margins: the page area is
    // x 20..770, y 10..550 (750x540 px), i.e. in PDF points (y up, 450pt tall page)
    // the rectangle `15 37.5 562.5 405`.
    page.geometry.width = 800.0;
    page.geometry.height = 600.0;
    page.geometry.margin = turbo_html2pdf_core::layout::value::Edges {
        top: 10.0,
        right: 30.0,
        bottom: 50.0,
        left: 20.0,
    };
    let plain = emit_pdf(std::slice::from_ref(&page), &EmitOptions::default());
    assert!(!contains(&plain, b" re\nf"), "no canvas, no fill");
    // rgb(1 0 255 / 200) over white, rounded to nearest: (v*200 + 255*55 + 127) / 255
    // = 56, 55, 255 -> `rg` operands 56/255, 55/255, 1.
    page.canvas = Some(Rgba::new(1, 0, 255, 200));
    let painted = emit_pdf(std::slice::from_ref(&page), &EmitOptions::default());
    let fill = b"0.21960784 0.21568628 1 rg\n15 37.5 562.5 405 re\nf";
    assert!(
        contains(&painted, fill),
        "canvas fill operators missing: {}",
        String::from_utf8_lossy(&painted)
    );
}

#[cfg(feature = "pdf-ua")]
#[test]
fn a_tagged_render_paints_the_canvas_as_an_artifact() {
    let mut page = pages(&doc("", "<p>x</p>"), "").remove(0);
    let opts = EmitOptions {
        lang: Some("en-US".to_string()),
        pdf_ua: true,
        ..EmitOptions::default()
    };
    let plain = emit_pdf(std::slice::from_ref(&page), &opts);
    page.canvas = Some(Rgba::new(0x12, 0x34, 0x56, 255));
    let painted = emit_pdf(std::slice::from_ref(&page), &opts);
    assert_ne!(plain, painted);
    assert!(contains(&painted, b"/MarkInfo"));
}

#[test]
fn the_documents_sheets_style_the_footnote_band() {
    // Everything rendered after the body (footnote bodies here) cascades against the
    // document's own `<style>` rules too, not only the caller's sheet.
    let page = &pages(
        &doc(
            "<style>.n{color:#00c000}</style>",
            "<p>See<t:footnote><span class=\"n\">note</span></t:footnote>.</p>",
        ),
        "",
    )[0];
    let mut band = Vec::new();
    text_lines(&page.footnotes, &mut band);
    assert!(!band.is_empty(), "the footnote band paints");
    assert!(
        band.iter().any(|l| l.2 == Rgba::new(0, 0xc0, 0, 255)),
        "the head rule colours the note: {band:?}"
    );
}
