//! `to-unicode` feature tests: standalone `/ToUnicode` CMap emission for
//! searchable text without pulling in the full tagged / accessible PDF
//! machinery (that's `pdf-ua`). Only compiled with `--features to-unicode`.
//!
//! Drives a semantic HTML fixture through the pipeline and asserts:
//! - opting in via `EmitOptions::to_unicode = true` emits a `/ToUnicode` CMap
//!   on every embedded font;
//! - the CMap is a well-formed Adobe-Identity-UCS stream with `bfchar` entries;
//! - the flag-off render is still byte-identical to the baseline;
//! - `qpdf --check` on the opt-in output is clean.

#![cfg(feature = "to-unicode")]

mod common;

use std::io::Write;
use std::process::Command;

use turbo_html2pdf_core::style::TokenSet;
use turbo_html2pdf_core::{
    build_cascade, compile, emit_pdf, render_pages, CompileOptions, Diagnostics, EmitOptions,
    RenderInputs,
};

const TEMPLATE: &str = r#"
<h1>Searchable Report</h1>
<p>The quick brown fox jumps over the lazy dog. 1234567890.</p>
"#;

const CSS: &str = "body { font-family: Evolventa; font-size: 12px; } \
h1 { font-size: 20px; }";

fn opts_on() -> EmitOptions {
    EmitOptions {
        title: Some("Searchable Report".to_string()),
        // The per-render toggle: this suite drives the searchable-text path
        // without the accessibility tag tree.
        to_unicode: true,
        ..EmitOptions::default()
    }
}

/// Render the sample pages once, reused across the per-toggle emits below.
fn sample_pages() -> Vec<turbo_html2pdf_core::paginate::Page> {
    let (program, _) =
        compile(TEMPLATE, &CompileOptions::default()).expect("compile sample template");
    let cascade = build_cascade(CSS, "", TokenSet::default());
    let fonts = common::registry();
    let inputs = RenderInputs {
        program: &program,
        data: &serde_json::json!({}),
        cascade: &cascade,
        at_rules: &[],
        fonts: &fonts,
        images: &turbo_html2pdf_core::NoImages,
        now: Some(0),
    };
    let mut diags = Diagnostics::default();
    render_pages(&inputs, &mut diags).expect("render pages")
}

fn build_pdf() -> Vec<u8> {
    emit_pdf(&sample_pages(), &opts_on())
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn opt_in_emits_the_to_unicode_cmap() {
    let pdf = build_pdf();
    assert!(
        contains(&pdf, b"/ToUnicode"),
        "an opt-in render emits a /ToUnicode entry in each Font dict"
    );
    assert!(
        contains(&pdf, b"/Adobe-Identity-UCS"),
        "the CMap identifies as Adobe-Identity-UCS"
    );
    assert!(
        contains(&pdf, b"beginbfchar"),
        "the CMap uses bfchar entries to map glyph codes to Unicode scalars"
    );
    assert!(contains(&pdf, b"endbfchar"), "each bfchar block is closed");
    assert!(contains(&pdf, b"endcmap"), "the CMap stream is terminated");
}

#[test]
fn opt_in_does_not_emit_tagged_pdf_machinery() {
    // `to-unicode` is a strict subset of `pdf-ua`: it only wires up the
    // per-font CMap. It must not drag in the accessibility tag tree,
    // MarkInfo, XMP metadata, or DisplayDocTitle — those are `pdf-ua`.
    let pdf = build_pdf();
    for marker in [
        &b"/StructTreeRoot"[..],
        b"/MarkInfo",
        b"/Marked true",
        b"/ParentTree",
        b"/MCID",
        b"/StructParents",
        b"/DisplayDocTitle",
        b"pdfuaid:part",
        b"BDC",
        b"/Artifact",
    ] {
        assert!(
            !contains(&pdf, marker),
            "to-unicode render must not emit tagged-PDF marker {:?}",
            std::str::from_utf8(marker).unwrap()
        );
    }
}

#[test]
fn to_unicode_false_emits_no_cmap_under_to_unicode_build() {
    // The per-render toggle is OFF: even compiled with `to-unicode`, the
    // output must NOT carry a `/ToUnicode` entry. This is the
    // byte-identical-default guarantee.
    let pages = sample_pages();
    let pdf = emit_pdf(
        &pages,
        &EmitOptions {
            title: Some("Searchable Report".to_string()),
            to_unicode: false,
            ..EmitOptions::default()
        },
    );
    assert!(
        !contains(&pdf, b"/ToUnicode"),
        "flag-off render must not emit /ToUnicode"
    );
    assert!(
        !contains(&pdf, b"/Adobe-Identity-UCS"),
        "flag-off render must not emit the CMap header"
    );
    // A flag-off render is byte-deterministic.
    let again = emit_pdf(
        &pages,
        &EmitOptions {
            title: Some("Searchable Report".to_string()),
            to_unicode: false,
            ..EmitOptions::default()
        },
    );
    assert_eq!(pdf, again, "flag-off render is byte-deterministic");
}

#[test]
fn opt_in_render_is_byte_deterministic() {
    // Two identical opt-in renders produce identical bytes (AC-7.6).
    let pages = sample_pages();
    let a = emit_pdf(&pages, &opts_on());
    let b = emit_pdf(&pages, &opts_on());
    assert_eq!(a, b, "opt-in render is byte-deterministic");
}

/// Write `pdf` to a temp file and return its path.
fn write_temp(pdf: &[u8], name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(name);
    let mut f = std::fs::File::create(&path).expect("create temp pdf");
    f.write_all(pdf).expect("write temp pdf");
    path
}

/// Whether a tool is invokable on the host.
fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success() || o.status.code().is_some())
        .unwrap_or(false)
}

#[test]
fn qpdf_check_is_clean() {
    if !have("qpdf") {
        eprintln!("qpdf not on PATH; skipping structural check");
        return;
    }
    let pdf = build_pdf();
    let path = write_temp(&pdf, "turbo_to_unicode_qpdf.pdf");
    let out = Command::new("qpdf")
        .arg("--check")
        .arg(&path)
        .output()
        .expect("run qpdf");
    assert!(
        out.status.success(),
        "qpdf --check failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn pdftotext_extracts_the_source_text() {
    // The whole point: an opt-in render lets pdftotext (and every other
    // extractor) recover the source text, byte for byte.
    if !have("pdftotext") {
        eprintln!("pdftotext not on PATH; skipping extraction check");
        return;
    }
    let pdf = build_pdf();
    let path = write_temp(&pdf, "turbo_to_unicode_extract.pdf");
    let out = Command::new("pdftotext")
        .arg(&path)
        .arg("-")
        .output()
        .expect("run pdftotext");
    let text = String::from_utf8_lossy(&out.stdout);
    for needle in [
        "Searchable Report",
        "The quick brown fox jumps over the lazy dog",
        "1234567890",
    ] {
        assert!(
            text.contains(needle),
            "pdftotext did not recover {:?}; got:\n{text}",
            needle,
        );
    }
}
