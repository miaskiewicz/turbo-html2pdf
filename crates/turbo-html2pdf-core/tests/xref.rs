//! Phase 15b `xref` feature tests (§3, AC-3.25). Only compiled with `--features
//! xref`. Drives `<t:anchor name>` + `<a href="#name">` through the whole
//! pipeline (compile → render_pages → emit) and asserts
//! the emitted PDF carries a named GoTo destination and a Link annotation
//! targeting it. When `qpdf` is on PATH the output is also structurally checked.

#![cfg(feature = "xref")]

mod common;

use std::process::Command;

use turbo_html2pdf_core::layout::fragment::Fragment;
use turbo_html2pdf_core::paginate::Page;
use turbo_html2pdf_core::style::TokenSet;
use turbo_html2pdf_core::{
    build_cascade, compile, emit_pdf, render_pages, CompileOptions, Diagnostics, EmitOptions,
    RenderInputs, Semantics,
};

/// Run a bare HTML template (no data) through to paginated pages (the full
/// `render_pages` pipeline, which derives the link/anchor semantics).
fn pages(template: &str) -> Vec<Page> {
    let (program, _) = compile(template, &CompileOptions::default()).expect("compile");
    let cascade = build_cascade("", "", TokenSet::default());
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
    render_pages(&inputs, &mut Diagnostics::default()).expect("render pages")
}

/// Locate `needle` in a byte slice.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Concatenate every `link_href` found across a page's fragments.
fn collect_link_hrefs(sem: &Semantics, frag: &Fragment, out: &mut Vec<String>) {
    if let Some(href) = sem.link_href(frag) {
        out.push(href.to_string());
    }
    for child in &frag.children {
        collect_link_hrefs(sem, child, out);
    }
}

/// Concatenate every `anchor` name found across a page's fragments.
fn collect_anchors(sem: &Semantics, frag: &Fragment, out: &mut Vec<String>) {
    if let Some(name) = sem.anchor(frag) {
        out.push(name.to_string());
    }
    for child in &frag.children {
        collect_anchors(sem, child, out);
    }
}

fn all_hrefs(pages: &[Page]) -> Vec<String> {
    let mut out = Vec::new();
    for page in pages {
        for frag in &page.body {
            collect_link_hrefs(&page.semantics, frag, &mut out);
        }
    }
    out
}

fn all_anchors(pages: &[Page]) -> Vec<String> {
    let mut out = Vec::new();
    for page in pages {
        for frag in &page.body {
            collect_anchors(&page.semantics, frag, &mut out);
        }
    }
    out
}

// -- fragment threading --------------------------------------------------------

#[test]
fn anchor_name_rides_the_fragment() {
    let pages = pages("<t:anchor name=\"ch2\"/><div>body</div>");
    assert_eq!(all_anchors(&pages), vec!["ch2".to_string()]);
}

#[test]
fn internal_link_href_rides_the_fragment() {
    let pages = pages("<div><a href=\"#ch2\">see chapter 2</a></div>");
    assert_eq!(all_hrefs(&pages), vec!["ch2".to_string()]);
}

#[test]
fn external_and_empty_links_are_not_internal() {
    // An absolute href and a bare `#` carry no internal-link payload.
    let pages = pages("<div><a href=\"https://x\">x</a><a href=\"#\">y</a></div>");
    assert!(all_hrefs(&pages).is_empty());
}

/// Every external `link_uri` found across the pages' body fragments.
fn all_uris(pages: &[Page]) -> Vec<String> {
    fn walk(sem: &Semantics, frag: &Fragment, out: &mut Vec<String>) {
        if let Some(uri) = sem.link_uri(frag) {
            out.push(uri.to_string());
        }
        for child in &frag.children {
            walk(sem, child, out);
        }
    }
    let mut out = Vec::new();
    for page in pages {
        for frag in &page.body {
            walk(&page.semantics, frag, &mut out);
        }
    }
    out
}

// -- turbo-html2pdf#53 (Chrome 156: external links, id destinations) ------------

#[test]
fn id_attribute_defines_a_destination() {
    // Chrome 156 names a destination after every element `id`, so `href="#sec"`
    // resolves without a `<t:anchor>`.
    let pages = pages("<h2 id=\"sec\">Section</h2><p id=\"\">no name</p>");
    assert_eq!(all_anchors(&pages), vec!["sec".to_string()]);
}

#[test]
fn anchor_id_attribute_names_the_destination() {
    // packages/react's `Anchor({ id })` emits `<t:anchor id=…>`; that id is a
    // destination like any other element id.
    let pages = pages("<t:anchor id=\"ch2\"/><div>body</div>");
    assert_eq!(all_anchors(&pages), vec!["ch2".to_string()]);
}

#[test]
fn id_destination_is_written_to_dests() {
    let pages = pages("<p><a href=\"#sec\">go</a></p><h2 id=\"sec\">Section</h2>");
    let pdf = emit_pdf(&pages, &EmitOptions::default());
    assert!(contains(&pdf, b"/Dests"), "an id defines a destination");
    assert!(
        contains(&pdf, b"/sec ["),
        "the destination is named after the id"
    );
}

#[test]
fn block_external_link_is_a_uri_link() {
    // Chrome 156 writes a `/S /URI` Link annotation for an absolute href.
    let pages = pages(
        "<a href=\"https://example.com/x\" style=\"display:block\">external</a>\
         <a href=\"page.html\" style=\"display:block\">relative</a>",
    );
    assert_eq!(all_uris(&pages), vec!["https://example.com/x".to_string()]);
    assert!(
        all_hrefs(&pages).is_empty(),
        "an external href is not internal"
    );
    let pdf = emit_pdf(&pages, &EmitOptions::default());
    assert!(contains(&pdf, b"/S /URI"), "the link uses a URI action");
    assert!(
        contains(&pdf, b"(https://example.com/x)"),
        "the URI is written"
    );
    assert!(!contains(&pdf, b"/S /GoTo"), "no GoTo for an external link");
    assert!(
        !contains(&pdf, b"page.html"),
        "a relative href has no base to resolve"
    );
}

#[test]
fn internal_links_are_not_uris() {
    let pages = pages("<a href=\"#ch2\" style=\"display:block\">go</a>");
    assert!(all_uris(&pages).is_empty());
}

// -- emit ----------------------------------------------------------------------

fn anchor_and_link_pdf() -> Vec<u8> {
    let pages = pages("<t:anchor name=\"ch2\"/><div>intro</div><div><a href=\"#ch2\">go</a></div>");
    emit_pdf(&pages, &EmitOptions::default())
}

#[test]
fn emits_named_destination_and_goto_link() {
    let pdf = anchor_and_link_pdf();
    assert!(contains(&pdf, b"/Dests"), "catalog references a Dests dict");
    assert!(contains(&pdf, b"/ch2"), "destination is named ch2");
    assert!(
        contains(&pdf, b"/Subtype /Link"),
        "a Link annotation is written"
    );
    assert!(contains(&pdf, b"/S /GoTo"), "the link uses a GoTo action");
    assert!(
        contains(&pdf, b"/Annots"),
        "the page references its annotations"
    );
}

#[test]
fn no_xref_markup_writes_no_annotations() {
    let pdf = emit_pdf(&pages("<div>plain body</div>"), &EmitOptions::default());
    assert!(!contains(&pdf, b"/Dests"), "no dests without an anchor");
    assert!(
        !contains(&pdf, b"/Subtype /Link"),
        "no links without an <a>"
    );
    assert!(!contains(&pdf, b"/Annots"), "no Annots array");
}

#[test]
fn output_is_deterministic() {
    let a = anchor_and_link_pdf();
    let b = anchor_and_link_pdf();
    assert_eq!(a, b, "identical inputs produce byte-identical xref output");
}

#[test]
fn qpdf_accepts_the_document() {
    if which_qpdf().is_none() {
        eprintln!("qpdf not on PATH; skipping structural check");
        return;
    }
    let pdf = anchor_and_link_pdf();
    let dir = std::env::temp_dir();
    let path = dir.join("turbo_pdf_xref_check.pdf");
    std::fs::write(&path, &pdf).expect("write pdf");
    let out = Command::new("qpdf")
        .arg("--check")
        .arg(&path)
        .output()
        .expect("run qpdf");
    assert!(
        out.status.success(),
        "qpdf --check failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

fn which_qpdf() -> Option<()> {
    Command::new("qpdf")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| ())
}
