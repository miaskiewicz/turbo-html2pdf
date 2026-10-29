//! miaskiewicz/turbo-html2pdf#54 (the emitter side of miaskiewicz/turbo-dom#205): each
//! glyph's `/ToUnicode` entry is the source text of the cluster it was shaped from, not
//! the lowest code point the font's cmap maps to it. Expected text is Chrome 156
//! `page.pdf()` (headless, the same fixture fonts through `@font-face`) read back by
//! `pdftotext`: `“quotes” ‘single’ don’t` (Inter) and `a -> b != c => d` (Fira Code
//! ligatures).

mod common;

use std::process::Command;

use turbo_dom_layout::test_support::fixture_fonts;
use turbo_html2pdf_core::style::TokenSet;
use turbo_html2pdf_core::{
    build_cascade, compile, emit_pdf, render_pages, CompileOptions, Diagnostics, EmitOptions,
    RenderInputs,
};

const DOC: &str = r#"<!DOCTYPE html><html><head><style>@page{size:600px 400px;margin:40px}body{margin:0;font-size:12px}</style></head><body><p style="font-family:sans-serif">“quotes” ‘single’ don’t</p><p style="font-family:monospace">a -> b != c => d</p></body></html>"#;

fn pdf() -> Vec<u8> {
    let (program, _) = compile(DOC, &CompileOptions::default()).expect("compile");
    let cascade = build_cascade("", "", TokenSet::default());
    let fonts = fixture_fonts();
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
    emit_pdf(&pages, &EmitOptions::default())
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn ligatures_and_curly_quotes_map_to_their_source_text() {
    let pdf = pdf();
    // Fira Code draws `->`, `!=` and `=>` with contextual glyphs that have no cmap
    // entry of their own: each maps to the character it was shaped from, so both
    // contextual `=` and both contextual `>` glyphs are mapped.
    let count = |needle: &str| {
        pdf.windows(needle.len())
            .filter(|w| *w == needle.as_bytes())
            .count()
    };
    assert_eq!(count("> <003E>\n"), 2);
    assert_eq!(count("> <003D>\n"), 2);
    assert_eq!(count("> <002D>\n"), 1);
    assert_eq!(count("> <0021>\n"), 1);
    // Inter's curly quotes map to themselves, not a lower code point sharing the glyph.
    for quote in ["201C", "201D", "2018", "2019"] {
        assert!(
            contains(&pdf, format!("> <{quote}>\n").as_bytes()),
            "{quote}"
        );
    }
    assert!(!contains(&pdf, b"> <02EE>\n"), "U+02EE is not in the text");
    assert!(!contains(&pdf, b"> <02BC>\n"), "U+02BC is not in the text");
}

#[test]
fn pdftotext_reads_the_text_like_chrome_156() {
    if Command::new("pdftotext").arg("-v").output().is_err() {
        eprintln!("pdftotext not on PATH; skipping the extraction check");
        return;
    }
    let path = std::env::temp_dir().join(format!("turbo_to_unicode_{}.pdf", std::process::id()));
    std::fs::write(&path, pdf()).expect("write pdf");
    let out = Command::new("pdftotext")
        .args(["-enc", "UTF-8"])
        .arg(&path)
        .arg("-")
        .output()
        .expect("pdftotext");
    std::fs::remove_file(&path).ok();
    let text = String::from_utf8(out.stdout).expect("utf-8");
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines, ["“quotes” ‘single’ don’t", "a -> b != c => d"]);
}
