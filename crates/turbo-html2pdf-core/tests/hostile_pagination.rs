//! Pagination terminates on hostile input: a repeated table header taller than the
//! page leaves no room for a sliced row's continuation (`slice`, #47), which used to
//! loop forever adding pages.

mod common;

use std::sync::mpsc;
use std::time::Duration;

use turbo_html2pdf_core::style::TokenSet;
use turbo_html2pdf_core::{
    build_cascade, compile, render_pages, CompileOptions, Diagnostics, NoImages, RenderInputs,
};

const DOC_TEXT: &str = r#"<!DOCTYPE html><html><head><style>@page{size:600px 400px;margin:40px}body{margin:0;font-family:Evolventa}</style></head><body><table><thead><tr><td style="height:5000px">h</td></tr></thead><tbody><tr><td>x</td></tr></tbody></table></body></html>"#;

fn page_count(doc: &str) -> usize {
    let (program, _) = compile(doc, &CompileOptions::default()).expect("compile");
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
    render_pages(&inputs, &mut Diagnostics::default())
        .expect("render_pages")
        .len()
}

/// The same header over an empty row taller than the page: the row is sliced (#47).
const DOC_SLICED: &str = r#"<!DOCTYPE html><html><head><style>@page{size:600px 400px;margin:40px}body{margin:0;font-family:Evolventa}</style></head><body><table><thead><tr><td style="height:5000px">h</td></tr></thead><tbody><tr><td style="height:900px"></td></tr></tbody></table></body></html>"#;

fn terminates(doc: &'static str) {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || tx.send(page_count(doc)).ok());
    let pages = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("pagination finished");
    assert!((1..=20).contains(&pages), "{pages} pages");
}

#[test]
fn a_header_taller_than_the_page_terminates() {
    terminates(DOC_TEXT);
}

#[test]
fn a_sliced_row_below_a_page_tall_header_terminates() {
    terminates(DOC_SLICED);
}

/// The pages and text lines of `doc`.
fn pages_and_lines(doc: &str) -> (usize, usize) {
    fn lines(fs: &[turbo_html2pdf_core::Fragment]) -> usize {
        fs.iter()
            .map(|f| {
                usize::from(matches!(
                    f.content,
                    turbo_html2pdf_core::layout::fragment::FragmentContent::TextLine { .. }
                )) + lines(&f.children)
            })
            .sum()
    }
    let (program, _) = compile(doc, &CompileOptions::default()).expect("compile");
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
    (pages.len(), pages.iter().map(|p| lines(&p.body)).sum())
}

/// miaskiewicz/turbo-html2pdf#62: a list item whose own text runs past the page breaks
/// between its lines like a paragraph. Its out-of-flow `::marker` box overlaps the first
/// line, which made the item look like a row of side-by-side cells, split in parallel,
/// and every line was lost.
#[test]
fn a_list_item_taller_than_a_page_keeps_its_lines() {
    let text: String = (0..800).map(|i| format!("word{i} ")).collect();
    let page =
        "<style>@page{size:600px 400px;margin:40px}body{margin:0;font-family:Evolventa}</style>";
    let bare = pages_and_lines(&format!("{page}<ul><li>{text}</li></ul>"));
    let wrapped = pages_and_lines(&format!(
        "{page}<ul><li><p style=\"margin:0\">{text}</p></li></ul>"
    ));
    assert!(wrapped.0 > 1 && wrapped.1 > 20, "{wrapped:?}");
    assert_eq!(bare, wrapped, "(pages, lines) of li text vs li>p");
}

/// `body` on 600x400 pages vs one page tall enough to need no break: (pages, lines) of the
/// paged render, and the lines of the unbroken one.
fn paged_vs_unbroken(body: &str) -> ((usize, usize), usize) {
    let page = |h: u32| {
        format!("<style>@page{{size:600px {h}px;margin:40px}}body{{margin:0;font-family:Evolventa}}</style>{body}")
    };
    let paged = pages_and_lines(&page(400));
    let (one, lines) = pages_and_lines(&page(200_000));
    assert_eq!(one, 1, "the tall page holds everything");
    (paged, lines)
}

/// Every line of a split list item lands exactly once (#62: the item's last content is its
/// lowest in-flow child, its `::marker` box aside).
#[test]
fn split_list_items_lose_and_duplicate_no_lines() {
    let text: String = (0..300).map(|i| format!("word{i} ")).collect();
    let cases = [
        (
            "float",
            format!(
                "<ul><li><div style=\"float:left;width:80px;height:60px\">F</div>{text}</li></ul>"
            ),
        ),
        (
            "inside",
            format!("<ul style=\"list-style-position:inside\"><li>{text}</li><li>{text}</li></ul>"),
        ),
        (
            "nested",
            format!("<ul><li>{text}<ul><li>{text}</li><li>{text}</li></ul></li><li>end</li></ul>"),
        ),
        (
            "table",
            format!(
                "<ul><li>{text}<table>{}</table>after</li></ul>",
                (0..30)
                    .map(|i| format!("<tr><td>Row {i}</td></tr>"))
                    .collect::<String>()
            ),
        ),
    ];
    for (name, body) in cases {
        let ((pages, lines), want) = paged_vs_unbroken(&body);
        assert!(pages > 1, "{name}: splits ({pages} pages)");
        assert_eq!(lines, want, "{name}: lines paged vs unbroken");
    }
}
