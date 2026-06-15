//! Phase 15 `pdf-a` feature tests (AC-11.2). Only compiled with `--features
//! pdf-a`. Drives a real fixture through the full pipeline, asserts the
//! PDF/A-specific structure (OutputIntent + sRGB ICC + XMP `pdfaid` packet +
//! trailer `/ID`, and the absence of the watermark `/ca` transparency), and —
//! when the tooling is on PATH — validates the output with `qpdf --check` and
//! veraPDF `--flavour 2b`.
//!
//! veraPDF was installed via `brew install verapdf` in this environment, so the
//! gated conformance assertion runs here; if a future host lacks it, the test
//! skips the veraPDF step (printing a note) and still asserts the structure.

#![cfg(feature = "pdf-a")]

mod common;

use std::io::Write;
use std::process::Command;

use turbo_pdf_core::layout::value::Rgba;
use turbo_pdf_core::paginate::{paginate, Page};
use turbo_pdf_core::style::TokenSet;
use turbo_pdf_core::{
    build_cascade, compile, emit_pdf, layout, style_tree, CompileOptions, Diagnostics, EmitOptions,
    StyledNode, TextWatermark, Watermark,
};

fn fixture_path(name: &str, file: &str) -> String {
    format!(
        "{}/tests/fixtures/{name}/{file}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn read_optional(name: &str, file: &str) -> Option<String> {
    std::fs::read_to_string(fixture_path(name, file)).ok()
}

fn read_required(name: &str, file: &str) -> String {
    let path = fixture_path(name, file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// Run one fixture through the whole pipeline into pages.
fn paginate_fixture(name: &str) -> Vec<Page> {
    let template = read_required(name, "template.html");
    let css = read_optional(name, "style.css").unwrap_or_default();
    let data_src = read_required(name, "data.json");
    let data: serde_json::Value =
        serde_json::from_str(&data_src).unwrap_or_else(|e| panic!("parse {name}/data.json: {e}"));

    let (program, _) = compile(&template, &CompileOptions::default())
        .unwrap_or_else(|e| panic!("compile {name}: {:?}", e.code));
    let (nodes, _) = program
        .render_nodes(&data, Some(0))
        .unwrap_or_else(|e| panic!("render {name}: {:?}", e.code));

    let cascade = build_cascade(&css, "", TokenSet::default());
    let styled: Vec<StyledNode> = style_tree(&nodes, &cascade);

    let mut diags = Diagnostics::default();
    let galley = layout(&styled, 540.0, &common::registry(), &mut diags);
    paginate(&galley, &[], &mut diags).expect("paginate")
}

/// Full metadata so every XMP property (title/author/subject/keywords) is
/// exercised and must agree with the info dict.
fn full_options() -> EmitOptions {
    EmitOptions {
        title: Some("Archival Report & <Notes>".to_string()),
        author: Some("Wojtek \"Kurwa\" Test".to_string()),
        subject: Some("PDF/A-2b conformance".to_string()),
        keywords: Some("archive, pdfa, sRGB".to_string()),
        creation_date: None,
        watermark: None,
        // Spread the rest so feature-gated fields (e.g. `lang` under `pdf-ua`)
        // are filled when this test is compiled alongside other features.
        ..EmitOptions::default()
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

// --- structural assertions ----------------------------------------------------

#[test]
fn emits_output_intent_icc_and_xmp() {
    let pages = paginate_fixture("invoice");
    let pdf = emit_pdf(&pages, &full_options());

    // The OutputIntent (GTS_PDFA) and its sRGB output condition.
    assert!(contains(&pdf, b"/OutputIntents"), "OutputIntents array");
    assert!(contains(&pdf, b"GTS_PDFA"), "GTS_PDFA subtype");
    assert!(
        contains(&pdf, b"sRGB IEC61966-2.1"),
        "sRGB output condition id"
    );
    assert!(contains(&pdf, b"DestOutputProfile"), "DestOutputProfile");
    // The ICC stream marks itself ICCBased with three components.
    assert!(contains(&pdf, b"/N 3"), "ICC component count");
    // The XMP metadata stream and the PDF/A id properties.
    assert!(contains(&pdf, b"/Type /Metadata"), "Metadata stream");
    assert!(
        contains(&pdf, b"<pdfaid:part>2</pdfaid:part>"),
        "pdfaid part"
    );
    assert!(
        contains(&pdf, b"<pdfaid:conformance>B</pdfaid:conformance>"),
        "pdfaid conformance B"
    );
    // The trailer /ID PDF/A requires.
    assert!(contains(&pdf, b"/ID ["), "trailer file id");
}

#[test]
fn xmp_mirrors_info_dict() {
    let pages = paginate_fixture("invoice");
    let pdf = emit_pdf(&pages, &full_options());

    // The producer agrees between info dict and XMP.
    assert!(contains(&pdf, b"<pdf:Producer>turbo-pdf</pdf:Producer>"));
    // Title/author/subject/keywords flow into the XMP, XML-escaped.
    assert!(contains(&pdf, b"Archival Report &amp; &lt;Notes&gt;"));
    assert!(contains(&pdf, b"Wojtek &quot;Kurwa&quot; Test"));
    assert!(contains(&pdf, b"<dc:description>"));
    assert!(contains(
        &pdf,
        b"<pdf:Keywords>archive, pdfa, sRGB</pdf:Keywords>"
    ));
    // The sentinel create/modify dates in ISO-8601.
    assert!(contains(&pdf, b"2000-01-01T00:00:00Z"));
}

#[test]
fn empty_metadata_omits_optional_xmp_properties() {
    let pages = paginate_fixture("invoice");
    let pdf = emit_pdf(&pages, &EmitOptions::default());
    // No title/author/subject/keywords supplied → those XMP properties are
    // absent (so nothing can disagree with the info dict), but the packet still
    // carries the producer and the PDF/A id.
    assert!(!contains(&pdf, b"<dc:title>"));
    assert!(!contains(&pdf, b"<dc:creator>"));
    assert!(!contains(&pdf, b"<pdf:Keywords>"));
    assert!(contains(&pdf, b"<pdf:Producer>turbo-pdf</pdf:Producer>"));
    assert!(contains(&pdf, b"<pdfaid:part>2</pdfaid:part>"));
}

#[test]
fn watermark_fade_transparency_is_suppressed() {
    let pages = paginate_fixture("invoice");
    let mut opts = full_options();
    opts.watermark = Some(Watermark::Text(Box::new(TextWatermark {
        text: "DRAFT".to_string(),
        face: common::evolventa(),
        font_size: 64.0,
        color: Rgba::new(128, 128, 128, 255),
        opacity: 0.25,
        angle_deg: 45.0,
    })));
    let pdf = emit_pdf(&pages, &opts);
    // PDF/A-2b forbids transparency: no `/ca` alpha and no fade ExtGState.
    assert!(
        !contains(&pdf, b"/ca "),
        "no non-stroking alpha under pdf-a"
    );
    assert!(!contains(&pdf, b"/GSwm"), "no fade ExtGState under pdf-a");
}

#[test]
fn output_is_byte_deterministic() {
    let pages = paginate_fixture("invoice");
    let a = emit_pdf(&pages, &full_options());
    let b = emit_pdf(&pages, &full_options());
    assert_eq!(a, b, "identical inputs must produce identical bytes");
}

// --- tool-gated validation ----------------------------------------------------

/// Write `pdf` to a temp file and return its path (kept until process exit).
fn write_temp(pdf: &[u8], name: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("turbo_pdfa_{name}_{}.pdf", std::process::id()));
    let mut f = std::fs::File::create(&path).expect("create temp pdf");
    f.write_all(pdf).expect("write temp pdf");
    path
}

fn tool_on_path(tool: &str) -> bool {
    Command::new("which")
        .arg(tool)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn verapdf_validates_2b() {
    let pages = paginate_fixture("invoice");
    let pdf = emit_pdf(&pages, &full_options());
    let path = write_temp(&pdf, "verapdf");

    if !tool_on_path("verapdf") {
        eprintln!(
            "SKIP: veraPDF not on PATH; PDF/A-2b conformance not externally \
             validated (structural assertions still ran). Install with \
             `brew install verapdf`."
        );
        return;
    }

    let out = Command::new("verapdf")
        .arg("--flavour")
        .arg("2b")
        .arg(&path)
        .output()
        .expect("run verapdf");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // veraPDF exits 0 and reports `isCompliant="true"` on a passing document.
    assert!(
        out.status.success() && stdout.contains("isCompliant=\"true\""),
        "veraPDF --flavour 2b did not pass:\nstatus={:?}\nstdout=\n{stdout}\nstderr=\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn qpdf_check_clean() {
    if !tool_on_path("qpdf") {
        eprintln!("SKIP: qpdf not on PATH; structural integrity not checked.");
        return;
    }
    let pages = paginate_fixture("invoice");
    let pdf = emit_pdf(&pages, &full_options());
    let path = write_temp(&pdf, "qpdf");

    let out = Command::new("qpdf")
        .arg("--check")
        .arg(&path)
        .output()
        .expect("run qpdf");
    assert!(
        out.status.success(),
        "qpdf --check failed:\nstdout=\n{}\nstderr=\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
