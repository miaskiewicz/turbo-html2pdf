//! miaskiewicz/turbo-dom#537: a grid row with an `fr` track beside an `auto` track and
//! `align-items: baseline` keeps its wrappable text on the solved column width. turbo-dom
//! 0.8.5 placed the baseline-aligned item as if the `fr` cell's text had wrapped at its
//! min-content width, so each label/value row grew by several lines and a one-page
//! payslip rendered on two pages.
//!
//! Chrome for Testing 156.0.8078.47 control (752×992 viewport, `font-size: 8px;
//! line-height: 1.4`): every row below is one line, 11.19px tall, with both items at
//! the same y, exactly as with `align-items: start`. The payslip-shaped document prints
//! on one Letter page.

mod common;

use turbo_html2pdf_core::style::TokenSet;
use turbo_html2pdf_core::{
    build_cascade, compile, layout_boxes, render_pages, CidBox, CompileOptions, Diagnostics, Page,
    RenderInputs,
};

const ROW: &str = r#"<div data-cid="row" style="display:grid;grid-template-columns:COLS;column-gap:8px;align-items:ALIGN;font-size:8px;line-height:1.4"><span data-cid="label" style="LABEL">Total percepciones</span><span data-cid="value">$1,000.00</span></div>"#;

fn row(cols: &str, align: &str, label: &str) -> Vec<CidBox> {
    let html = ROW
        .replace("COLS", cols)
        .replace("ALIGN", align)
        .replace("LABEL", label);
    let fonts = common::registry();
    let mut diags = Diagnostics::default();
    layout_boxes(&html, "", 752.0, 992.0, &fonts, &mut diags).expect("boxes")
}

fn assert_one_line(cols: &str, label: &str) {
    let baseline = row(cols, "baseline", label);
    let start = row(cols, "start", label);
    let (row_box, first, second) = (&baseline[0], &baseline[1], &baseline[2]);
    assert_eq!(
        row_box.height, start[0].height,
        "{cols} / {label:?}: baseline row is as tall as the one-line start row"
    );
    assert_eq!(
        first.height, row_box.height,
        "{cols} / {label:?}: label stays on one line"
    );
    assert_eq!(
        second.y, first.y,
        "{cols} / {label:?}: same-size items share one baseline row"
    );
}

#[test]
fn baseline_rows_with_an_auto_track_stay_on_one_line() {
    assert_one_line("1fr auto", "");
    assert_one_line("1fr auto", "overflow-wrap:break-word");
    assert_one_line("auto 1fr", "overflow-wrap:break-word");
}

const PAYSLIP_CSS: &str = "@page { size: LETTER; margin: 32px } \
    body { margin: 0; font-family: sans-serif; font-size: 8px; line-height: 1.4 } \
    .breakdown { display: grid; grid-template-columns: 1fr auto; column-gap: 8px; \
    align-items: baseline; padding: 1px 0 } \
    .label { overflow-wrap: break-word }";

const BREAKDOWN: &str = r#"<div class="breakdown"><span class="label">Cuotas de seguridad social</span><span>$50.00</span></div>"#;

fn pages(template: &str, css: &str) -> Vec<Page> {
    let (program, _) = compile(template, &CompileOptions::default()).expect("compile");
    let cascade = build_cascade(css, "", TokenSet::default());
    let fonts = common::registry();
    let rules = turbo_html2pdf_core::style::parse_stylesheet(css).at_rules;
    let data = serde_json::Value::Null;
    let inputs = RenderInputs {
        program: &program,
        data: &data,
        cascade: &cascade,
        at_rules: &rules,
        fonts: &fonts,
        images: &turbo_html2pdf_core::NoImages,
        now: Some(0),
    };
    render_pages(&inputs, &mut Diagnostics::default()).expect("render_pages")
}

#[test]
fn a_payslip_of_baseline_breakdown_rows_fits_one_letter_page() {
    // Twenty one-line rows are about 270px; Letter's page area is 992px tall.
    let template = BREAKDOWN.repeat(20);
    assert_eq!(pages(&template, PAYSLIP_CSS).len(), 1);
}
