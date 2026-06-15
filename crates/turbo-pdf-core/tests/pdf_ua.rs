//! Phase 15b `pdf-ua` feature tests (AC-11.1): tagged / accessible PDF. Only
//! compiled with `--features pdf-ua`.
//!
//! Drives a semantic HTML fixture (headings, paragraphs, a list, a table) through
//! the whole pipeline and asserts the emitted PDF is tagged: a `StructTreeRoot`,
//! `/MarkInfo <</Marked true>>`, marked content (`/MCID`) and a `/ParentTree`,
//! plus `/Lang` and `DisplayDocTitle`. When `verapdf` is on `PATH` the document
//! is additionally validated against PDF/UA-1 (`--flavour ua1`); when `qpdf` is
//! on `PATH` its structural check must pass.

#![cfg(feature = "pdf-ua")]

mod common;

use std::io::Write;
use std::process::Command;

use turbo_pdf_core::style::TokenSet;
use turbo_pdf_core::{
    build_cascade, compile, emit_pdf, render_pages, CompileOptions, Diagnostics, EmitOptions,
    RenderInputs,
};

const TEMPLATE: &str = r#"
<h1>Accessible Report</h1>
<p>This document is tagged for assistive technology. It carries a structure tree
so a screen reader knows the reading order and the role of every block.</p>
<h2>Findings</h2>
<p>The first finding is summarised in the list below.</p>
<ul>
  <li>The header is a level-one heading.</li>
  <li>Each paragraph is a P element.</li>
  <li>The list is an L with LI children.</li>
</ul>
<h2>Data</h2>
<table>
  <tr><th>Quarter</th><th>Revenue</th></tr>
  <tr><td>Q1</td><td>100</td></tr>
  <tr><td>Q2</td><td>140</td></tr>
</table>
"#;

const CSS: &str = "body { font-family: Evolventa; font-size: 12px; } \
table, td, th { border: 1px solid #000; } h1 { font-size: 20px; } \
h2 { font-size: 16px; }";

fn opts() -> EmitOptions {
    EmitOptions {
        title: Some("Accessible Report".to_string()),
        lang: Some("en-US".to_string()),
        ..EmitOptions::default()
    }
}

/// Run the sample template through the pipeline and emit a tagged PDF.
fn build_pdf() -> Vec<u8> {
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
        images: &turbo_pdf_core::NoImages,
        now: Some(0),
    };
    let mut diags = Diagnostics::default();
    let pages = render_pages(&inputs, &mut diags).expect("render pages");
    emit_pdf(&pages, &opts())
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn emits_the_tagged_pdf_skeleton() {
    let pdf = build_pdf();
    assert!(
        contains(&pdf, b"/StructTreeRoot"),
        "catalog references a StructTreeRoot"
    );
    assert!(
        contains(&pdf, b"/Type /StructTreeRoot"),
        "a StructTreeRoot object is written"
    );
    assert!(
        contains(&pdf, b"/Marked true"),
        "MarkInfo marks the document as tagged"
    );
    assert!(contains(&pdf, b"/ParentTree"), "a ParentTree is present");
    assert!(contains(&pdf, b"/MCID"), "marked content carries MCIDs");
    assert!(
        contains(&pdf, b"/S /Document"),
        "the root Document structure element is written"
    );
    assert!(contains(&pdf, b"/Lang"), "the document language is set");
    assert!(
        contains(&pdf, b"/DisplayDocTitle true"),
        "the viewer is told to show the document title"
    );
    assert!(
        contains(&pdf, b"/StructParents"),
        "pages declare their StructParents key"
    );
}

#[test]
fn tags_the_semantic_roles() {
    let pdf = build_pdf();
    for role in [
        &b"/S /H1"[..],
        b"/S /H2",
        b"/S /P",
        b"/S /L",
        b"/S /LI",
        b"/S /Table",
        b"/S /TR",
        b"/S /TH",
        b"/S /TD",
    ] {
        assert!(
            contains(&pdf, role),
            "expected structure role {:?} in the tree",
            std::str::from_utf8(role).unwrap()
        );
    }
}

#[test]
fn box_decoration_is_an_artifact() {
    let pdf = build_pdf();
    // The table borders are decoration, bracketed as /Artifact in the stream.
    assert!(
        contains(&pdf, b"/Artifact"),
        "decorative box paints are marked as artifacts"
    );
}

#[test]
fn carries_the_pdfua_xmp_identifier() {
    let pdf = build_pdf();
    assert!(
        contains(&pdf, b"pdfuaid:part"),
        "the XMP packet identifies the document as PDF/UA"
    );
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
    let path = write_temp(&pdf, "turbo_pdf_ua_qpdf.pdf");
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
fn passes_verapdf_ua1() {
    if !have("verapdf") {
        eprintln!("verapdf not on PATH; skipping PDF/UA-1 validation");
        return;
    }
    let pdf = build_pdf();
    let path = write_temp(&pdf, "turbo_pdf_ua_verapdf.pdf");
    let out = Command::new("verapdf")
        .args(["--flavour", "ua1"])
        .arg(&path)
        .output()
        .expect("run verapdf");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("isCompliant=\"true\""),
        "verapdf --flavour ua1 did not report compliance:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
