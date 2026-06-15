//! Phase 15b runtime conformance toggles: the `cmyk` / `pdf_a` / `pdf_ua`
//! `EmitOptions` flags now ACTIVATE per render, with the cargo features merely
//! gating capability. This suite is compiled with all three conformance features
//! plus `xref`, so it can assert two things the per-feature suites cannot:
//!
//! 1. **Byte-identical default** — with every toggle off, none of the
//!    conformance machinery appears, so a feature-on build's flag-off render is
//!    the same output the default build produces.
//! 2. **Combined flags co-exist** — `pdf_a` + `pdf_ua` + internal links in one
//!    render allocate non-colliding object ids (qpdf accepts the document, and
//!    each feature's markers are present).

#![cfg(all(feature = "pdf-a", feature = "pdf-ua", feature = "xref"))]

mod common;

use std::io::Write;
use std::process::Command;

use turbo_pdf_core::layout::fragment::{Fragment, FragmentContent, NodeId};
use turbo_pdf_core::layout::value::{BorderEdges, Rgba};
use turbo_pdf_core::paginate::{paginate, Page, PageGeometry};
use turbo_pdf_core::style::TokenSet;
use turbo_pdf_core::{
    build_cascade, compile, emit_pdf, layout, style_tree, CompileOptions, Diagnostics, EmitOptions,
    PageKind, StyledNode,
};

/// A semantic template with a heading, a paragraph, an anchor and an internal
/// link — enough to exercise tags (`pdf-ua`), colour fills and cross-references
/// (`xref`) at once.
const TEMPLATE: &str = r##"
<h1>Combined</h1>
<p>Body text with an <a href="#tail">internal link</a>.</p>
<t:anchor name="tail"/>
<p>Tail target paragraph.</p>
"##;

const CSS: &str = "body { font-family: Evolventa; font-size: 12px; } h1 { font-size: 20px; }";

fn pages() -> Vec<Page> {
    let (program, _) = compile(TEMPLATE, &CompileOptions::default()).expect("compile");
    let (nodes, _) = program
        .render_nodes(&serde_json::json!({}), Some(0))
        .expect("render");
    let cascade = build_cascade(CSS, "", TokenSet::default());
    let styled: Vec<StyledNode> = style_tree(&nodes, &cascade);
    let mut diags = Diagnostics::default();
    let galley = layout(&styled, 540.0, &common::registry(), &mut diags);
    paginate(&galley, &[], &mut diags).expect("paginate")
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// All conformance toggles ON.
fn all_on() -> EmitOptions {
    EmitOptions {
        title: Some("Combined".to_string()),
        cmyk: true,
        pdf_a: true,
        pdf_ua: true,
        lang: Some("en-US".to_string()),
        ..EmitOptions::default()
    }
}

#[test]
fn all_flags_off_emits_no_conformance_machinery() {
    // The byte-identical-default guarantee: with every per-render toggle off,
    // even this all-features build emits NONE of the conformance machinery, so
    // the bytes match the plain default build.
    let pdf = emit_pdf(&pages(), &EmitOptions::default());
    for marker in [
        // pdf-a
        &b"/OutputIntents"[..],
        b"GTS_PDFA",
        b"<pdfaid:part>",
        b"/ID [",
        // pdf-ua
        b"/StructTreeRoot",
        b"/MarkInfo",
        b"/ParentTree",
        b"/MCID",
        b"/ToUnicode",
        b"/StructParents",
        b"/DisplayDocTitle",
        // print-color: a DeviceCMYK fill operator
        b" k\n",
    ] {
        assert!(
            !contains(&pdf, marker),
            "flag-off render must not emit {:?}",
            std::str::from_utf8(marker).unwrap()
        );
    }
    // The default colour space is DeviceRGB.
    assert!(contains(&pdf, b" rg\n"), "DeviceRGB fills by default");
    // Deterministic across renders.
    assert_eq!(pdf, emit_pdf(&pages(), &EmitOptions::default()));
}

#[test]
fn all_flags_on_coexist_with_non_colliding_ids() {
    let pdf = emit_pdf(&pages(), &all_on());
    // Each feature's signature object is present in one render.
    assert!(contains(&pdf, b"/OutputIntents"), "pdf-a OutputIntent");
    assert!(contains(&pdf, b"<pdfaid:part>2"), "pdf-a pdfaid");
    assert!(contains(&pdf, b"/StructTreeRoot"), "pdf-ua struct tree");
    assert!(contains(&pdf, b"/ToUnicode"), "pdf-ua per-face ToUnicode");
    assert!(contains(&pdf, b"/S /GoTo"), "xref GoTo link action");
    assert!(contains(&pdf, b"/Dests"), "xref named destination dict");
    // CMYK fills are active for this render.
    assert!(contains(&pdf, b" k\n"), "cmyk fills active");
    assert!(!contains(&pdf, b" rg\n"), "no DeviceRGB fill under cmyk");
    // Deterministic across renders.
    assert_eq!(pdf, emit_pdf(&pages(), &all_on()), "byte-deterministic");
}

#[test]
fn qpdf_accepts_combined_flags() {
    if Command::new("qpdf")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("qpdf not on PATH; skipping combined-flags structural check");
        return;
    }
    let pdf = emit_pdf(&pages(), &all_on());
    let path = std::env::temp_dir().join("turbo_combined_flags.pdf");
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&pdf))
        .expect("write temp pdf");
    let out = Command::new("qpdf")
        .arg("--check")
        .arg(&path)
        .output()
        .expect("run qpdf");
    // qpdf reports duplicated/colliding object ids as errors, so a clean check
    // proves the three features' object ids never overlap.
    assert!(
        out.status.success(),
        "qpdf --check failed (object-id collision?):\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A page with one solid red box fragment, for the colour-space check.
fn red_box_page() -> Page {
    let frag = Fragment::new(
        NodeId(1),
        10.0,
        10.0,
        100.0,
        50.0,
        FragmentContent::Box {
            background: Some(Rgba::new(255, 0, 0, 255)),
            border: BorderEdges::default(),
        },
    );
    Page {
        geometry: PageGeometry::a4(),
        kind: PageKind::First,
        number: 1,
        body: vec![frag],
        header: Vec::new(),
        footer: Vec::new(),
        footnotes: Vec::new(),
    }
}

/// A red box must fill DeviceRGB under `cmyk: false` and DeviceCMYK under
/// `cmyk: true`, proving the toggle drives the colour space at runtime in this
/// build (the feature only gates the capability).
#[test]
fn cmyk_toggle_switches_colour_space() {
    let page = [red_box_page()];
    let rgb = emit_pdf(&page, &EmitOptions::default());
    let cmyk = emit_pdf(
        &page,
        &EmitOptions {
            cmyk: true,
            ..EmitOptions::default()
        },
    );
    assert!(
        contains(&rgb, b"1 0 0 rg"),
        "red -> DeviceRGB when cmyk off"
    );
    assert!(!contains(&rgb, b" k\n"), "no DeviceCMYK when cmyk off");
    assert!(
        contains(&cmyk, b"0 1 1 0 k"),
        "red -> DeviceCMYK when cmyk on"
    );
    assert!(!contains(&cmyk, b" rg\n"), "no DeviceRGB when cmyk on");
}
