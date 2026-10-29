//! A box whose children are only text lines (a "paragraph" to the fragmenter) keeps the
//! height its box has beyond its lines when it is split across pages: a definite
//! `height`, a `min-height`, or bottom padding runs on through the following pages, as
//! for any other block (#33). Page counts are Chrome 156 `page.pdf()` on `600px x 400px`
//! pages with 40px margins and the Evolventa fixture font.

mod common;

use std::sync::mpsc;
use std::time::Duration;

use turbo_html2pdf_core::style::TokenSet;
use turbo_html2pdf_core::{
    build_cascade, compile, render_pages, CompileOptions, Diagnostics, NoImages, RenderInputs,
};

fn page_count(body: &'static str) -> usize {
    let doc = format!(
        "<!DOCTYPE html><html><head><style>@page{{size:600px 400px;margin:40px}}body{{margin:0;font-family:Evolventa}}</style></head><body>{body}<p>t</p></body></html>"
    );
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let (program, _) = compile(&doc, &CompileOptions::default()).expect("compile");
        let cascade = build_cascade("", "", TokenSet::default());
        let fonts = common::registry();
        let inputs = RenderInputs {
            program: &program,
            data: &serde_json::Value::Null,
            cascade: &cascade,
            at_rules: &[],
            fonts: &fonts,
            images: &NoImages,
            now: Some(0),
        };
        let pages = render_pages(&inputs, &mut Diagnostics::default()).expect("render_pages");
        tx.send(pages.len()).ok();
    });
    rx.recv_timeout(Duration::from_secs(60))
        .expect("pagination finished")
}

#[test]
fn a_definite_height_runs_on_past_its_lines() {
    assert_eq!(page_count(r#"<div style="height:1000px">a</div>"#), 4);
    assert_eq!(page_count(r#"<div style="height:3000px">a</div>"#), 10);
}

#[test]
fn a_definite_height_runs_on_past_several_lines() {
    assert_eq!(page_count(r#"<div style="height:1000px">a<br>b<br>c</div>"#), 4);
}

#[test]
fn a_min_height_runs_on_past_its_lines() {
    assert_eq!(page_count(r#"<div style="min-height:1000px">a</div>"#), 4);
}

#[test]
fn bottom_padding_runs_on_past_its_lines() {
    assert_eq!(page_count(r#"<div style="padding-bottom:1000px">a</div>"#), 5);
}

#[test]
fn a_tall_table_cell_runs_on_past_its_lines() {
    assert_eq!(
        page_count(r#"<table><tr><td style="height:1000px">a</td></tr></table>"#),
        4
    );
}
