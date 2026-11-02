//! miaskiewicz/turbo-html2pdf#44: paint carried onto a neighbouring page by a split
//! relatively offset block (#42) repeats content its own page carries. A carried text
//! line is shown inside a `/Span` with an empty `/ActualText`, so text extraction finds
//! the text once (on its own page); when `pdftotext` is on `PATH` that is checked too.
//! The tagged render (the copy as an `/Artifact`) is covered in tests/pdf_ua.rs.

mod common;

use std::process::Command;

use turbo_html2pdf_core::style::TokenSet;
use turbo_html2pdf_core::{
    build_cascade, compile, emit_pdf, render_pages, CompileOptions, Diagnostics, EmitOptions,
    RenderInputs, CARRIED,
};

/// A split `bottom: 20px` relative block whose one text line, moved to page 2, is
/// shifted across the page boundary: its top part paints at the end of page 1.
const DOC: &str = r#"<!DOCTYPE html><html><head><style>@page{size:600px 400px;margin:40px}body{margin:0;font-family:Evolventa;font-size:12px}</style></head><body><div style="position:relative;bottom:20px"><div style="height:300px"></div><div style="height:40px;break-inside:avoid">Zebracorn</div></div></body></html>"#;

fn pages() -> Vec<turbo_html2pdf_core::Page> {
    let (program, _) = compile(DOC, &CompileOptions::default()).expect("compile");
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
    render_pages(&inputs, &mut Diagnostics::default()).expect("render_pages")
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

#[test]
fn the_copy_on_the_previous_page_is_carried() {
    let pages = pages();
    assert_eq!(pages.len(), 2);
    // Page 1: the 300px block, then the carried copy of the text block.
    let ids: Vec<_> = pages[0].body.iter().map(|f| f.node_id).collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], CARRIED);
    assert_eq!(ids[1], CARRIED);
    assert!(pages[1].body.iter().all(|f| f.node_id != CARRIED));
}

#[test]
fn carried_text_is_extracted_once() {
    let pdf = emit_pdf(&pages(), &EmitOptions::default());
    assert_eq!(count(&pdf, b"\nBT\n"), 2, "painted on both pages");
    assert_eq!(count(&pdf, b"/Span <<\n  /ActualText ()\n>> BDC"), 1);
    if Command::new("pdftotext").arg("-v").output().is_err() {
        eprintln!("pdftotext not on PATH; skipping the extraction check");
        return;
    }
    let path = std::env::temp_dir().join(format!("turbo_carried_{}.pdf", std::process::id()));
    std::fs::write(&path, &pdf).expect("write pdf");
    let out = Command::new("pdftotext")
        .arg(&path)
        .arg("-")
        .output()
        .expect("pdftotext");
    std::fs::remove_file(&path).ok();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        text.matches("Zebracorn").count(),
        1,
        "extracted once: {text:?}"
    );
}
