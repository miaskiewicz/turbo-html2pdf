//! Phase 15 `append` feature tests (AC: append). Only compiled with `--features
//! append`. Builds "foreign" PDF bytes deterministically with turbo's own
//! emitter, then exercises [`append_pdfs`]: page-count is the sum of inputs,
//! multiple extras append in order, the merge is byte-deterministic, malformed
//! input is an error, and (when `qpdf` is on `PATH`) the merged output is clean.

#![cfg(feature = "append")]

use turbo_html2pdf_core::paginate::{Page, PageGeometry};
use turbo_html2pdf_core::{append_pdfs, emit_pdf, AppendError, EmitOptions, PageKind};

/// `n` empty A4 pages. Empty bodies need no fonts, so the emitter output is fully
/// deterministic with no fixture dependency.
fn empty_pages(n: u32) -> Vec<Page> {
    (1..=n)
        .map(|number| Page {
            geometry: PageGeometry::a4(),
            kind: PageKind::First,
            number,
            body: Vec::new(),
            header: Vec::new(),
            footer: Vec::new(),
            footnotes: Vec::new(),
            semantics: Default::default(),
        })
        .collect()
}

/// A turbo-emitted PDF with `n` pages, standing in for a "foreign" provider PDF.
fn turbo_pdf(n: u32) -> Vec<u8> {
    emit_pdf(&empty_pages(n), &EmitOptions::default())
}

/// Count leaf `/Type/Page` objects (excludes the `/Type/Pages` tree node). lopdf
/// serialises names back-to-back with no space, so the marker has no space.
fn count_pages(pdf: &[u8]) -> usize {
    let marker = b"/Type/Page";
    (0..pdf.len().saturating_sub(marker.len()))
        .filter(|&i| {
            &pdf[i..i + marker.len()] == marker && pdf.get(i + marker.len()) != Some(&b's')
        })
        .count()
}

#[test]
fn appends_single_extra_sums_pages() {
    let base = turbo_pdf(2);
    let extra = turbo_pdf(3);
    let merged = append_pdfs(&base, &[&extra]).expect("append");

    assert!(merged.starts_with(b"%PDF-"), "bad header");
    assert_eq!(count_pages(&merged), 5, "page count must be base + extra");
}

#[test]
fn appends_multiple_extras_in_order() {
    let base = turbo_pdf(1);
    let a = turbo_pdf(2);
    let b = turbo_pdf(4);
    let merged = append_pdfs(&base, &[&a, &b]).expect("append");

    assert_eq!(
        count_pages(&merged),
        1 + 2 + 4,
        "page count must be the sum"
    );
    assert_eq!(
        count_catalogs(&merged),
        1,
        "exactly one catalog after rebuild"
    );
}

/// Count `/Type/Catalog` markers to prove the page tree was rebuilt to one root.
fn count_catalogs(pdf: &[u8]) -> usize {
    let marker = b"/Type/Catalog";
    (0..pdf.len().saturating_sub(marker.len()))
        .filter(|&i| &pdf[i..i + marker.len()] == marker)
        .count()
}

#[test]
fn append_with_no_extras_returns_base_pages() {
    let base = turbo_pdf(3);
    let merged = append_pdfs(&base, &[]).expect("append with no extras");
    assert_eq!(count_pages(&merged), 3);
}

#[test]
fn append_is_byte_deterministic() {
    let base = turbo_pdf(2);
    let extra = turbo_pdf(2);
    let a = append_pdfs(&base, &[&extra]).expect("append a");
    let b = append_pdfs(&base, &[&extra]).expect("append b");
    assert_eq!(a, b, "same inputs must yield byte-identical output");
}

#[test]
fn malformed_base_is_error() {
    let extra = turbo_pdf(1);
    let err = append_pdfs(b"not a pdf at all", &[&extra]).unwrap_err();
    assert!(matches!(err, AppendError::Malformed(_)), "got {err:?}");
    // Exercise the Display impl so the error message path is covered.
    assert!(err.to_string().contains("malformed PDF"));
}

#[test]
fn malformed_extra_is_error() {
    let base = turbo_pdf(1);
    let err = append_pdfs(&base, &[b"%PDF-1.7 garbage \x00\xff"]).unwrap_err();
    assert!(matches!(err, AppendError::Malformed(_)), "got {err:?}");
}

#[test]
fn no_pages_is_error() {
    // A structurally valid PDF whose catalog points at an empty page tree: it
    // parses fine but yields zero pages, so the merge has nothing to append. The
    // fixture is a real lopdf-emitted document (an xref-stream PDF with a binary
    // body), checked in rather than hand-written so its offsets stay valid.
    let empty = include_bytes!("fixtures/append/empty_tree.pdf");
    let err = append_pdfs(empty, &[]).unwrap_err();
    assert!(matches!(err, AppendError::NoPages), "got {err:?}");
    assert!(err.to_string().contains("no pages"));
}

#[test]
fn merged_output_passes_qpdf_check() {
    if !qpdf_available() {
        return;
    }
    let base = turbo_pdf(2);
    let extra = turbo_pdf(3);
    let merged = append_pdfs(&base, &[&extra]).expect("append");

    let path = std::env::temp_dir().join("turbo-pdf-append-check.pdf");
    std::fs::write(&path, &merged).expect("write temp pdf");
    let out = std::process::Command::new("qpdf")
        .arg("--check")
        .arg(&path)
        .output()
        .expect("run qpdf");
    assert!(
        out.status.success(),
        "qpdf --check failed: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Whether the `qpdf` binary is on `PATH`.
fn qpdf_available() -> bool {
    std::process::Command::new("which")
        .arg("qpdf")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
