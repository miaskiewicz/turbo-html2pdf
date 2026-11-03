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

use turbo_html2pdf_core::style::TokenSet;
use turbo_html2pdf_core::{
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
        // The per-render PDF/UA toggle: this whole suite drives the tagged path.
        pdf_ua: true,
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

/// Run the sample template through the pipeline and emit a tagged PDF.
fn build_pdf() -> Vec<u8> {
    emit_pdf(&sample_pages(), &opts())
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

#[test]
fn pdf_ua_false_emits_no_tagged_machinery_under_pdf_ua_build() {
    // The per-render toggle is OFF: even compiled with `pdf-ua`, the output must
    // carry NONE of the tagged-PDF machinery — no StructTreeRoot, no marked
    // content, no /MarkInfo / /Lang / DisplayDocTitle. This is the
    // byte-identical-default guarantee.
    let pages = sample_pages();
    let pdf = emit_pdf(
        &pages,
        &EmitOptions {
            title: Some("Accessible Report".to_string()),
            pdf_ua: false,
            ..EmitOptions::default()
        },
    );
    for marker in [
        &b"/StructTreeRoot"[..],
        b"/MarkInfo",
        b"/Marked true",
        b"/ParentTree",
        b"/MCID",
        b"/StructParents",
        b"/DisplayDocTitle",
        b"BDC",
        b"/Artifact",
    ] {
        assert!(
            !contains(&pdf, marker),
            "flag-off render must not emit {:?}",
            std::str::from_utf8(marker).unwrap()
        );
    }
    // A flag-off render still produces a valid, deterministic PDF.
    let again = emit_pdf(
        &pages,
        &EmitOptions {
            title: Some("Accessible Report".to_string()),
            pdf_ua: false,
            ..EmitOptions::default()
        },
    );
    assert_eq!(pdf, again, "flag-off render is byte-deterministic");
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

/// A tagged render of a document whose first paragraph is positioned (painted after
/// the second, miaskiewicz/turbo-html2pdf#35).
fn positioned_pdf() -> Vec<u8> {
    let template = r#"<p style="position:relative;top:4px">First</p><p>Second</p>"#;
    let (program, _) = compile(template, &CompileOptions::default()).expect("compile");
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
    let pages = render_pages(&inputs, &mut Diagnostics::default()).expect("render pages");
    emit_pdf(&pages, &opts())
}

#[test]
fn marked_content_follows_paint_order_and_the_tree_reading_order() {
    // The painter numbers content in paint order (the positioned paragraph last); the
    // structure keeps document order: the first P owns the last-painted MCID.
    let pdf = positioned_pdf();
    let at = |needle: &[u8]| pdf.windows(needle.len()).position(|w| w == needle);
    let (first, second) = (
        at(b"/S /P\n  /P 12 0 R\n  /Pg 3 0 R\n  /K [1]"),
        at(b"/K [0]"),
    );
    assert!(
        first.is_some() && first < second,
        "the first P owns MCID 1, written first"
    );
    if have("verapdf") {
        let path = write_temp(&pdf, "turbo_pdf_ua_positioned_verapdf.pdf");
        let out = Command::new("verapdf")
            .args(["--flavour", "ua1"])
            .arg(&path)
            .output()
            .expect("run verapdf");
        assert!(String::from_utf8_lossy(&out.stdout).contains("isCompliant=\"true\""));
    }
}

/// A captioned table (miaskiewicz/turbo-dom#51), tagged: the template and the PDF.
fn caption_pdf() -> Vec<u8> {
    const CAPTIONED: &str = "<table><caption>Quarterly revenue</caption>\
        <tr><th>Quarter</th><th>Revenue</th></tr><tr><td>Q1</td><td>100</td></tr></table>";
    let (program, _) = compile(CAPTIONED, &CompileOptions::default()).expect("compile");
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
    emit_pdf(&render_pages(&inputs, &mut diags).expect("render"), &opts())
}

/// The structure elements of an (uncompressed-object) PDF: object number -> (`/S` role,
/// `/P` parent object number).
fn struct_elems(pdf: &[u8]) -> std::collections::BTreeMap<u32, (String, u32)> {
    let text = String::from_utf8_lossy(pdf);
    let mut out = std::collections::BTreeMap::new();
    for (at, _) in text.match_indices(" 0 obj") {
        let num: u32 = text[..at]
            .rsplit(|c: char| !c.is_ascii_digit())
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        let body = &text[at..at + text[at..].find("endobj").unwrap_or(0)];
        if !body.contains("/Type /StructElem") {
            continue;
        }
        let word = |key: &str| {
            body.split(key)
                .nth(1)
                .and_then(|rest| rest.split_whitespace().next())
                .map(str::to_string)
        };
        let role = word("/S /").unwrap_or_default();
        let parent = word("/P ").and_then(|n| n.parse().ok()).unwrap_or(0);
        out.insert(num, (role, parent));
    }
    out
}

#[test]
fn tags_a_table_caption_as_the_tables_caption() {
    // PDF/UA: a <caption> is the `Caption` child of its `Table`, and the rows stay the
    // table's children (the anonymous grid box of a captioned table is no structure).
    let pdf = caption_pdf();
    let elems = struct_elems(&pdf);
    let of = |role: &str| -> Vec<u32> {
        elems
            .iter()
            .filter(|(_, (r, _))| r == role)
            .map(|(&n, _)| n)
            .collect()
    };
    let tables = of("Table");
    assert_eq!(tables.len(), 1, "one Table: {elems:?}");
    let captions = of("Caption");
    assert_eq!(captions.len(), 1, "one Caption: {elems:?}");
    assert_eq!(
        elems[&captions[0]].1, tables[0],
        "the Caption's parent is the Table"
    );
    let rows = of("TR");
    assert_eq!(rows.len(), 2, "{elems:?}");
    // The rows are in the table's TBody (PDF/UA-1 7.2-4, #60).
    let bodies = of("TBody");
    assert_eq!(bodies.len(), 1, "{elems:?}");
    assert_eq!(elems[&bodies[0]].1, tables[0], "{elems:?}");
    for row in rows {
        assert_eq!(
            elems[&row].1, bodies[0],
            "a TR's parent is the TBody: {elems:?}"
        );
    }
}

#[test]
fn a_captioned_table_passes_verapdf_ua1() {
    if !have("verapdf") {
        eprintln!("verapdf not on PATH; skipping PDF/UA-1 validation");
        return;
    }
    let path = write_temp(&caption_pdf(), "turbo_pdf_ua_caption_verapdf.pdf");
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

/// A tagged render of a split `bottom: 20px` relative block whose one text line, moved to
/// page 2, is shifted across the page boundary: Chrome 156 paints its top part at the end
/// of page 1 (miaskiewicz/turbo-html2pdf#42).
fn carried_text_pdf() -> Vec<u8> {
    let template = r#"<!DOCTYPE html><html><head><style>@page{size:600px 400px;margin:40px}body{margin:0;font-family:Evolventa;font-size:12px}</style></head><body><div style="position:relative;bottom:20px"><div style="height:300px"></div><div style="height:40px;break-inside:avoid">Zebracorn</div></div></body></html>"#;
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
    let pages = render_pages(&inputs, &mut Diagnostics::default()).expect("render pages");
    assert_eq!(pages.len(), 2);
    emit_pdf(&pages, &opts())
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

#[test]
fn text_carried_onto_the_previous_page_is_an_artifact() {
    // miaskiewicz/turbo-html2pdf#44: the line paints on both pages (its copy on page 1
    // clipped to that page's end), but only its own page tags it as content.
    let pdf = carried_text_pdf();
    assert_eq!(count(&pdf, b"\nBT\n"), 2, "painted twice");
    assert_eq!(count(&pdf, b"/MCID"), 1, "tagged once");
    if have("pdftotext") {
        let path = write_temp(&pdf, "turbo_pdf_ua_carried_text.pdf");
        let out = Command::new("pdftotext")
            .arg(&path)
            .arg("-")
            .output()
            .expect("pdftotext");
        let text = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            text.matches("Zebracorn").count(),
            1,
            "extracted once: {text:?}"
        );
    }
}

#[test]
fn tags_list_markers_as_the_items_label() {
    // turbo-dom 0.7 lays out list markers (`::marker`, miaskiewicz/turbo-dom#128). PDF/UA-1
    // 7.2-20: an `LI` holds only `Lbl` / `LBody` structure, so each item's marker is its
    // `Lbl`, and its content is the item's `LBody`.
    let elems = struct_elems(&build_pdf());
    let items: Vec<u32> = elems
        .iter()
        .filter(|(_, (r, _))| r == "LI")
        .map(|(&n, _)| n)
        .collect();
    assert_eq!(items.len(), 3, "{elems:?}");
    for item in items {
        let kids: Vec<&str> = elems
            .values()
            .filter(|(_, p)| *p == item)
            .map(|(r, _)| r.as_str())
            .collect();
        assert_eq!(kids, ["Lbl", "LBody"], "LI {item}: {elems:?}");
    }
    // A marker's inner box (same marker id) is part of its Lbl, not a nested Lbl.
    for (role, parent) in elems.values() {
        if role == "Lbl" {
            assert_eq!(elems[parent].0, "LI", "{elems:?}");
        }
    }
}

/// Render `html` (with the suite CSS plus `css`) to a tagged PDF.
fn ua_pdf(html: &str, css: &str) -> Vec<u8> {
    let (program, _) = compile(html, &CompileOptions::default()).expect("compile");
    let cascade = build_cascade(&format!("{CSS} {css}"), "", TokenSet::default());
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
    emit_pdf(&render_pages(&inputs, &mut diags).expect("render"), &opts())
}

/// The roles of `node`'s structure children, in object order.
fn kids_of(elems: &std::collections::BTreeMap<u32, (String, u32)>, node: u32) -> Vec<&str> {
    elems
        .values()
        .filter(|(_, p)| *p == node)
        .map(|(r, _)| r.as_str())
        .collect()
}

/// PDF/UA-1 7.2-20 shape of every list item: `LI` holds exactly `want`, every `Lbl` and
/// `LBody` sits directly in an `LI`, and the PDF passes veraPDF (when on `PATH`).
fn assert_list_items(pdf: &[u8], name: &str, items: usize, want: &[&str]) {
    assert_struct_tree(pdf);
    let elems = struct_elems(pdf);
    let lis: Vec<u32> = elems
        .iter()
        .filter(|(_, (r, _))| r == "LI")
        .map(|(&n, _)| n)
        .collect();
    assert_eq!(lis.len(), items, "{elems:?}");
    for li in lis {
        assert_eq!(kids_of(&elems, li), want, "LI {li}: {elems:?}");
    }
    for (role, parent) in elems.values() {
        if role == "Lbl" || role == "LBody" {
            assert_eq!(elems[parent].0, "LI", "{role} parent: {elems:?}");
        }
    }
    assert_verapdf_ua1(pdf, name);
}

/// The PDF passes `verapdf --flavour ua1` (skipped when veraPDF is not on `PATH`).
fn assert_verapdf_ua1(pdf: &[u8], name: &str) {
    if !have("verapdf") {
        return;
    }
    let path = write_temp(pdf, name);
    let out = Command::new("verapdf")
        .args(["--flavour", "ua1"])
        .arg(&path)
        .output()
        .expect("run verapdf");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("isCompliant=\"true\""), "{name}:\n{stdout}");
}

#[test]
fn a_paragraph_in_a_list_item_is_in_its_lbody() {
    let pdf = ua_pdf("<ul><li><p>One</p></li><li><p>Two</p></li></ul>", "");
    assert_list_items(&pdf, "turbo_pdf_ua_li_p.pdf", 2, &["Lbl", "LBody"]);
    let elems = struct_elems(&pdf);
    for (role, parent) in elems.values() {
        if role == "P" {
            assert_eq!(elems[parent].0, "LBody", "{elems:?}");
        }
    }
}

#[test]
fn a_list_nested_in_an_ordered_list_item_is_in_its_lbody() {
    let pdf = ua_pdf(
        "<ol><li>Outer<ul><li>Inner a</li><li>Inner b</li></ul></li><li>Last</li></ol>",
        "",
    );
    assert_list_items(&pdf, "turbo_pdf_ua_nested.pdf", 4, &["Lbl", "LBody"]);
    let elems = struct_elems(&pdf);
    let lists: Vec<&u32> = elems
        .iter()
        .filter(|(_, (r, _))| r == "L")
        .map(|(_, (_, p))| p)
        .collect();
    assert_eq!(lists.len(), 2, "{elems:?}");
    assert!(lists.iter().any(|p| elems[p].0 == "LBody"), "{elems:?}");
}

#[test]
fn an_unmarked_list_item_has_only_an_lbody() {
    let pdf = ua_pdf(
        "<ul><li>One</li><li>Two</li></ul>",
        "li { list-style: none; }",
    );
    assert_list_items(&pdf, "turbo_pdf_ua_none.pdf", 2, &["LBody"]);
}

#[test]
fn an_inside_marker_is_the_items_label() {
    let css = "li { list-style-position: inside; }";
    for (html, name) in [
        (
            "<ul><li>One</li><li>Two</li></ul>",
            "turbo_pdf_ua_inside_ul.pdf",
        ),
        (
            "<ol><li>One</li><li>Two</li></ol>",
            "turbo_pdf_ua_inside_ol.pdf",
        ),
    ] {
        assert_list_items(&ua_pdf(html, css), name, 2, &["Lbl", "LBody"]);
    }
}

#[test]
fn an_ordered_lists_markers_are_one_label_each() {
    let pdf = ua_pdf("<ol><li>One</li><li>Two</li><li>Three</li></ol>", "");
    assert_list_items(&pdf, "turbo_pdf_ua_ol.pdf", 3, &["Lbl", "LBody"]);
}

/// A table's rows sit in `Table` or its row groups, never loose (PDF/UA-1 7.2-4).
fn assert_rows_in_tables(pdf: &[u8], name: &str) {
    assert_struct_tree(pdf);
    let elems = struct_elems(pdf);
    for (role, parent) in elems.values() {
        if role == "TR" {
            let up = elems[parent].0.as_str();
            assert!(
                matches!(up, "Table" | "THead" | "TBody" | "TFoot"),
                "TR in {up}: {elems:?}"
            );
        }
        if matches!(role.as_str(), "THead" | "TBody" | "TFoot") {
            assert_eq!(elems[parent].0, "Table", "{role} parent: {elems:?}");
        }
    }
    assert_verapdf_ua1(pdf, name);
}

#[test]
fn table_rows_are_in_their_row_groups() {
    let row = |c: &str| format!("<tr><td>{c}</td><td>1</td></tr>");
    let cases = [
        (
            format!("<table>{}{}</table>", row("a"), row("b")),
            "turbo_pdf_ua_table_plain.pdf",
        ),
        (
            format!(
                "<table><thead><tr><th>H</th><th>V</th></tr></thead><tbody>{}</tbody>\
                 <tfoot>{}</tfoot></table>",
                row("a"),
                row("f")
            ),
            "turbo_pdf_ua_table_groups.pdf",
        ),
    ];
    for (html, name) in cases {
        let pdf = ua_pdf(&html, "");
        assert_rows_in_tables(&pdf, name);
        let elems = struct_elems(&pdf);
        if html.contains("<thead>") {
            for group in ["THead", "TBody", "TFoot"] {
                assert!(
                    elems.values().any(|(r, _)| r == group),
                    "{group}: {elems:?}"
                );
            }
        }
    }
}

#[test]
fn rows_of_a_table_split_across_pages_stay_in_a_table() {
    let rows: String = (0..120)
        .map(|i| format!("<tr><td>Row {i}</td><td>{i}</td></tr>"))
        .collect();
    let html = format!(
        "<table><thead><tr><th>Name</th><th>Value</th></tr></thead><tbody>{rows}</tbody></table>"
    );
    assert_rows_in_tables(&ua_pdf(&html, ""), "turbo_pdf_ua_table_split.pdf");
}

// --- Multi-page structure (round-2 review of #59): a split block's own box becomes an
// id-0 slice, so its continuation pages must rebuild its Table / L / LI / TR.

/// The elements with role `role`.
fn of_role(elems: &std::collections::BTreeMap<u32, (String, u32)>, role: &str) -> Vec<u32> {
    elems
        .iter()
        .filter(|(_, (r, _))| r == role)
        .map(|(&n, _)| n)
        .collect()
}

/// The nearest `role` ancestor of element `n`, if any.
fn ancestor(
    elems: &std::collections::BTreeMap<u32, (String, u32)>,
    mut n: u32,
    role: &str,
) -> Option<u32> {
    while let Some((_, p)) = elems.get(&n) {
        if elems.get(p).is_some_and(|(r, _)| r == role) {
            return Some(*p);
        }
        n = *p;
    }
    None
}

/// `n` words of filler text.
fn words(n: usize) -> String {
    (0..n).map(|i| format!("word{i} ")).collect()
}

/// Rows `from..to` of a two-column table body.
fn rows(from: usize, to: usize) -> String {
    (from..to)
        .map(|i| format!("<tr><td>Row {i}</td><td>{i}</td></tr>"))
        .collect()
}

/// Every TD/TH is in a TR, every TR in a row group of a Table.
fn assert_cells_in_rows(elems: &std::collections::BTreeMap<u32, (String, u32)>) {
    for (role, parent) in elems.values() {
        if role == "TD" || role == "TH" {
            assert_eq!(elems[parent].0, "TR", "{role} parent: {elems:?}");
        }
    }
}

#[test]
fn two_split_tables_on_one_page_stay_two_tables() {
    let head = "<thead><tr><th>Name</th><th>Value</th></tr></thead>";
    for between in ["", "<p>Between the tables.</p>"] {
        let html = format!(
            "<table>{head}<tbody>{}</tbody></table>{between}<table>{head}<tbody>{}</tbody></table>",
            rows(0, 70),
            rows(70, 140)
        );
        let pdf = ua_pdf(&html, "");
        assert_rows_in_tables(&pdf, "turbo_pdf_ua_two_split_tables.pdf");
        let elems = struct_elems(&pdf);
        assert_cells_in_rows(&elems);
        assert_eq!(of_role(&elems, "Table").len(), 2, "{between:?}: {elems:?}");
        assert_eq!(of_role(&elems, "TR").len(), 142, "{elems:?}");
        // Each table's rows stay in it: the first table holds rows 0..70 (+ its head).
        let tables = of_role(&elems, "Table");
        let in_first = of_role(&elems, "TR")
            .into_iter()
            .filter(|&r| ancestor(&elems, r, "Table") == Some(tables[0]))
            .count();
        assert_eq!(in_first, 71, "{elems:?}");
    }
}

#[test]
fn a_repeated_table_header_is_tagged_once() {
    let html = format!(
        "<table><thead><tr><th>Name</th><th>Value</th></tr></thead><tbody>{}</tbody></table>",
        rows(0, 200)
    );
    let pdf = ua_pdf(&html, "");
    assert_rows_in_tables(&pdf, "turbo_pdf_ua_repeated_head.pdf");
    let elems = struct_elems(&pdf);
    assert_eq!(of_role(&elems, "Table").len(), 1, "{elems:?}");
    assert_eq!(of_role(&elems, "THead").len(), 1, "{elems:?}");
    assert_eq!(of_role(&elems, "TH").len(), 2, "{elems:?}");
    assert_eq!(of_role(&elems, "TR").len(), 201, "{elems:?}");
}

#[test]
fn a_split_tables_caption_is_its_tables_first_child() {
    let html = format!(
        "<p>{}</p><table><caption>Quarterly revenue</caption><tbody>{}</tbody></table>",
        words(500),
        rows(0, 60)
    );
    let pdf = ua_pdf(&html, "");
    assert_rows_in_tables(&pdf, "turbo_pdf_ua_split_caption.pdf");
    let elems = struct_elems(&pdf);
    let tables = of_role(&elems, "Table");
    assert_eq!(tables.len(), 1, "{elems:?}");
    let caption = of_role(&elems, "Caption");
    assert_eq!(caption.len(), 1, "{elems:?}");
    assert_eq!(elems[&caption[0]].1, tables[0], "{elems:?}");
    assert_eq!(kids_of(&elems, tables[0])[0], "Caption", "{elems:?}");
}

#[test]
fn a_row_taller_than_a_page_keeps_its_tr() {
    let html = format!(
        "<table><tr><td>A</td><td>B</td></tr><tr><td>{}</td><td>x</td></tr>\
         <tr><td>C</td><td>D</td></tr></table>",
        words(2500)
    );
    let pdf = ua_pdf(&html, "");
    assert_rows_in_tables(&pdf, "turbo_pdf_ua_tall_row.pdf");
    let elems = struct_elems(&pdf);
    assert_cells_in_rows(&elems);
    assert_eq!(of_role(&elems, "Table").len(), 1, "{elems:?}");
    assert_eq!(of_role(&elems, "TR").len(), 3, "{elems:?}");
    assert_eq!(of_role(&elems, "TD").len(), 6, "{elems:?}");
}

#[test]
fn a_list_longer_than_a_page_is_one_list() {
    let items: String = (0..150).map(|i| format!("<li>Item {i}</li>")).collect();
    let pdf = ua_pdf(&format!("<ul>{items}</ul>"), "");
    assert_list_items(&pdf, "turbo_pdf_ua_long_list.pdf", 150, &["Lbl", "LBody"]);
    let elems = struct_elems(&pdf);
    assert_eq!(of_role(&elems, "L").len(), 1, "{elems:?}");
    for li in of_role(&elems, "LI") {
        assert_eq!(elems[&elems[&li].1].0, "L", "{elems:?}");
    }
}

#[test]
fn a_list_item_taller_than_a_page_is_one_item() {
    let html = format!(
        "<ul><li>First</li><li>{}</li><li>Last</li></ul>",
        words(2500)
    );
    let pdf = ua_pdf(&html, "");
    assert_list_items(&pdf, "turbo_pdf_ua_tall_item.pdf", 3, &["Lbl", "LBody"]);
    let elems = struct_elems(&pdf);
    assert_eq!(of_role(&elems, "L").len(), 1, "{elems:?}");
    assert!(
        of_role(&elems, "P").is_empty(),
        "no loose paragraphs: {elems:?}"
    );
}

// --- Struct-tree integrity (round-3 review of #59) ---------------------------------

/// The objects of an (uncompressed-object) PDF: number -> body text.
fn objects(pdf: &[u8]) -> std::collections::BTreeMap<u32, String> {
    let text = String::from_utf8_lossy(pdf);
    let mut out = std::collections::BTreeMap::new();
    for (at, _) in text.match_indices(" 0 obj") {
        let num: u32 = text[..at]
            .rsplit(|c: char| !c.is_ascii_digit())
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        let end = text[at..].find("endobj").unwrap_or(0);
        out.insert(num, text[at + 6..at + end].to_string());
    }
    out
}

/// The integer after `key` in `body` (`/Pg 3 0 R` -> 3).
fn int_after(body: &str, key: &str) -> Option<u32> {
    body.split(key)
        .nth(1)?
        .split_whitespace()
        .next()?
        .trim_end_matches(['>', ']'])
        .parse()
        .ok()
}

/// One `/K` kid of a struct element.
#[derive(Debug, PartialEq)]
enum Kid {
    Elem(u32),
    /// (page object, MCID).
    Mcid(u32, u32),
    /// The annotation object of an `OBJR`.
    Objr(u32),
}

/// The `/K` value of struct element `body`, without its brackets.
fn k_value(body: &str) -> String {
    let Some(k) = body.split("/K ").nth(1).map(str::trim_start) else {
        return String::new();
    };
    let Some(rest) = k.strip_prefix('[') else {
        return k.lines().next().unwrap_or("").to_string();
    };
    // The closing `]` of the array (MCR / OBJR dicts hold no brackets).
    rest[..rest.find(']').unwrap_or(rest.len())].to_string()
}

/// One `<< ... >>` kid: an `OBJR` or an `MCR` (its `/Pg`, else the element's `pg`).
fn dict_kid(dict: &str, pg: Option<u32>) -> Kid {
    if dict.contains("/OBJR") {
        return Kid::Objr(int_after(dict, "/Obj ").expect("OBJR /Obj"));
    }
    let page = int_after(dict, "/Pg ").or(pg).expect("MCR page");
    Kid::Mcid(page, int_after(dict, "/MCID ").expect("MCR MCID"))
}

/// The `/K` kids of struct element `body` (bare MCIDs belong to its `/Pg`).
fn kids(body: &str) -> Vec<Kid> {
    let pg = int_after(body, "/Pg ");
    let k = k_value(body).replace("<<", " << ").replace(">>", " >> ");
    let toks: Vec<&str> = k.split_whitespace().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        if toks[i] == "<<" {
            let end = i + toks[i..].iter().position(|t| *t == ">>").expect("dict end");
            out.push(dict_kid(&toks[i + 1..end].join(" "), pg));
            i = end + 1;
        } else if toks.get(i + 2) == Some(&"R") {
            out.push(Kid::Elem(toks[i].parse().expect("ref")));
            i += 3;
        } else {
            out.push(Kid::Mcid(
                pg.expect("bare MCID needs /Pg"),
                toks[i].parse().expect("MCID"),
            ));
            i += 1;
        }
    }
    out
}

/// The object numbers in an array body (`[3 0 R 7 0 R]`).
fn refs(body: &str) -> Vec<u32> {
    let toks: Vec<&str> = body
        .split(|c: char| c.is_whitespace() || c == '[' || c == ']')
        .filter(|t| !t.is_empty())
        .collect();
    toks.windows(3)
        .filter(|w| w[2] == "R")
        .filter_map(|w| w[0].parse().ok())
        .collect()
}

type Objs = std::collections::BTreeMap<u32, String>;
type Pairs = std::collections::BTreeMap<(u32, u32), u32>;
/// Every page's (page, MCID) marks and each page's (page, `/StructParents` key).
type Marks = (std::collections::BTreeSet<(u32, u32)>, Vec<(u32, u32)>);

/// Every page's content-stream (page, MCID) pairs (no duplicates) and `/StructParents`
/// key.
fn page_marks(objs: &Objs) -> Marks {
    let mut marks = std::collections::BTreeSet::new();
    let mut keys = Vec::new();
    for (&n, body) in objs.iter().filter(|(_, b)| b.contains("/Type /Page\n")) {
        let stream = &objs[&int_after(body, "/Contents ").expect("page contents")];
        for m in stream.split("/MCID ").skip(1) {
            let mcid: u32 = m.split_whitespace().next().unwrap().parse().unwrap();
            assert!(marks.insert((n, mcid)), "MCID {mcid} twice on page {n}");
        }
        keys.push((
            n,
            int_after(body, "/StructParents ").expect("StructParents"),
        ));
    }
    (marks, keys)
}

/// The owner of each (page, MCID) and of each `OBJR` annotation; each `LI` starts with
/// its `Lbl`.
fn kid_owners(objs: &Objs) -> (Pairs, std::collections::BTreeMap<u32, u32>) {
    let (mut owner, mut objr) = (Pairs::new(), std::collections::BTreeMap::new());
    for (&n, body) in objs.iter().filter(|(_, b)| b.contains("/Type /StructElem")) {
        let ks = kids(body);
        for k in &ks {
            let dup = match *k {
                Kid::Mcid(p, m) => owner.insert((p, m), n).is_some(),
                Kid::Objr(a) => objr.insert(a, n).is_some(),
                Kid::Elem(_) => false,
            };
            assert!(!dup, "{k:?} owned twice");
        }
        assert_label_first(objs, n, body, &ks);
    }
    (owner, objr)
}

/// An `LI` holding an `Lbl` has it as kids[0].
fn assert_label_first(objs: &Objs, n: u32, body: &str, ks: &[Kid]) {
    let is_lbl = |k: &Kid| matches!(k, Kid::Elem(e) if objs[e].contains("/S /Lbl\n"));
    if body.contains("/S /LI\n") && ks.iter().any(is_lbl) {
        assert!(is_lbl(&ks[0]), "LI {n}: kids[0] is not its Lbl");
    }
}

/// The ParentTree's `/Nums`: key -> object.
fn parent_tree(objs: &Objs) -> std::collections::BTreeMap<u32, u32> {
    let root = objs
        .values()
        .find(|b| b.contains("/Type /StructTreeRoot"))
        .expect("root");
    let nums = root.split("/Nums").nth(1).expect("Nums");
    let toks: Vec<&str> = nums[..nums.find(']').unwrap()]
        .split(|c: char| c.is_whitespace() || c == '[')
        .filter(|t| !t.is_empty())
        .collect();
    toks.chunks(4)
        .map(|e| (e[0].parse().unwrap(), e[1].parse().unwrap()))
        .collect()
}

/// Check the tagged PDF's structure tree against its pages:
/// - every (page, MCID) of the content streams is owned by exactly one struct element
///   kid, and every kid's MCID is in its `/Pg`'s content stream (no duplicates either side);
/// - the ParentTree maps each page's MCID to its owning element, and each `OBJR`
///   annotation's `/StructParent` to the element holding it;
/// - an `LI`'s first kid is its `Lbl` (when it has one).
fn assert_struct_tree(pdf: &[u8]) {
    let objs = objects(pdf);
    let (marks, page_keys) = page_marks(&objs);
    let (owner, objr) = kid_owners(&objs);
    let owned: std::collections::BTreeSet<(u32, u32)> = owner.keys().copied().collect();
    assert_eq!(owned, marks, "struct MCIDs vs content-stream MCIDs");
    let tree = parent_tree(&objs);
    for (page, key) in page_keys {
        let arr = refs(&objs[&tree[&key]]);
        let want: Vec<u32> = (0..arr.len() as u32).map(|m| owner[&(page, m)]).collect();
        assert_eq!(arr, want, "ParentTree of page {page}");
        assert_eq!(arr.len(), marks.iter().filter(|(p, _)| *p == page).count());
    }
    for (annot, el) in objr {
        let key = int_after(&objs[&annot], "/StructParent ").expect("annotation StructParent");
        assert_eq!(
            tree.get(&key),
            Some(&el),
            "ParentTree of annotation {annot}"
        );
    }
}

/// The page objects in page order (`/Kids` of the page tree).
#[cfg(feature = "xref")]
fn page_objects(pdf: &[u8]) -> Vec<u32> {
    let objs = objects(pdf);
    let tree = objs
        .values()
        .find(|b| b.contains("/Type /Pages"))
        .expect("page tree");
    refs(tree.split("/Kids").nth(1).expect("Kids"))
}

/// A `<thead>` holding a link, repeated on every page of a 3+ page table: each page has a
/// clickable Link annotation, the copies are artifacts but their annotations are kids
/// (`OBJR`) of the one `Link` element (PDF/UA-1 7.18.5), the anchor is one destination.
#[cfg(feature = "xref")]
#[test]
fn a_link_in_a_repeated_header_is_on_every_page() {
    let html = format!(
        "<table><thead><tr><th><a href=\"#end\">Go to the end</a></th><th>Value</th></tr></thead>\
         <tbody>{}</tbody></table><p id=\"end\">The end.</p>",
        rows(0, 200)
    );
    let pdf = ua_pdf(&html, "");
    assert_rows_in_tables(&pdf, "turbo_pdf_ua_repeated_link.pdf");
    let objs = objects(&pdf);
    let pages = page_objects(&pdf);
    assert!(pages.len() >= 3, "{} pages", pages.len());
    for p in &pages[..pages.len() - 1] {
        let annots = objs[p]
            .split("/Annots")
            .nth(1)
            .map(refs)
            .unwrap_or_default();
        let links = annots
            .iter()
            .filter(|a| objs[a].contains("/Subtype /Link"))
            .count();
        assert!(links >= 1, "page {p} has its header link");
    }
    let elems = struct_elems(&pdf);
    let link_elems = of_role(&elems, "Link");
    assert_eq!(link_elems.len(), 1, "{elems:?}");
    let objr = kids(&objs[&link_elems[0]])
        .into_iter()
        .filter(|k| matches!(k, Kid::Objr(_)))
        .count();
    assert!(
        objr >= pages.len() - 1,
        "{objr} OBJRs for {} pages",
        pages.len()
    );
    assert_eq!(
        of_role(&elems, "TH").len(),
        2,
        "the header is tagged once: {elems:?}"
    );
    let dests = objs.values().filter(|b| b.contains("/end [")).count();
    assert_eq!(dests, 1, "one named destination");
}

/// A repeated header whose original is cut away: a header row taller than a page is
/// sliced (its original's paint is cut at the page end), and a header placed alone at a
/// page bottom repeats on the next page. Both stay tagged once and pass veraPDF.
#[test]
fn a_repeated_header_whose_original_is_cut_or_stranded() {
    let tall = format!(
        "<table><thead><tr><th style=\"height:1400px\">Tall</th><th>V</th></tr></thead><tbody>{}</tbody></table>",
        rows(0, 60)
    );
    let stranded = format!(
        "<p style=\"height:1000px\">Top</p><table><thead><tr><th>H</th><th>V</th></tr></thead>\
         <tbody>{}</tbody></table>",
        rows(0, 80)
    );
    for (html, name, ths) in [
        (tall, "turbo_pdf_ua_tall_head.pdf", 2),
        (stranded, "turbo_pdf_ua_stranded_head.pdf", 2),
    ] {
        let pdf = ua_pdf(&html, "");
        assert_rows_in_tables(&pdf, name);
        let elems = struct_elems(&pdf);
        assert_eq!(of_role(&elems, "TH").len(), ths, "{name}: {elems:?}");
        assert_eq!(of_role(&elems, "THead").len(), 1, "{name}: {elems:?}");
    }
}
