//! The html2pdf-mode HTML drive (root `layout_html` / `layout_html_with_images` /
//! `layout_boxes` / `parse_html`): raw HTML with the `t:` directives classified as in
//! a template render, `<t:…/>` expanded and unknown `t:` names rejected — unlike
//! turbo-dom-layout's directive-free `html_layout::*`.

mod common;

use turbo_html2pdf_core::layout::fragment::{Fragment, FragmentContent};
use turbo_html2pdf_core::{
    layout_boxes, layout_html, layout_html_with_images, parse_html, Diagnostics, ErrorCode,
    ImageCtx, NoImages, TKind, Tag,
};

const DOC: &str = r#"<body><p>a<t:page/>b</p><div data-cid="d" style="height:10px">x</div></body>"#;

fn markers(frag: &Fragment, out: &mut Vec<TKind>) {
    out.extend(TKind::of_fragment(&frag.content));
    for child in &frag.children {
        markers(child, out);
    }
}

#[test]
fn directives_are_markers_on_the_html_drive() {
    let fonts = common::registry();
    let mut diags = Diagnostics::default();
    let galley = layout_html(DOC, "", 400.0, &fonts, &mut diags).expect("layout");
    let mut found = Vec::new();
    markers(&galley, &mut found);
    assert_eq!(found, vec![TKind::Page]);
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    let with_images =
        layout_html_with_images(DOC, "", 400.0, &fonts, &images, &mut diags).expect("layout");
    assert_eq!(format!("{with_images:?}"), format!("{galley:?}"));
    assert!(matches!(
        with_images.children[0].content,
        FragmentContent::Box { .. }
    ));
    let boxes = layout_boxes(DOC, "", 400.0, 300.0, &fonts, &mut diags).expect("boxes");
    assert_eq!(boxes.len(), 1);
    assert_eq!(boxes[0].cid, "d");
}

#[test]
fn unknown_t_elements_are_rejected_everywhere() {
    let fonts = common::registry();
    let mut diags = Diagnostics::default();
    let bad = "<p><t:bogus/></p>";
    let code = |r: Result<(), turbo_html2pdf_core::RenderError>| r.unwrap_err().code;
    assert_eq!(
        code(layout_html(bad, "", 100.0, &fonts, &mut diags).map(drop)),
        ErrorCode::UnknownElement
    );
    assert_eq!(
        code(layout_boxes(bad, "", 100.0, 100.0, &fonts, &mut diags).map(drop)),
        ErrorCode::UnknownElement
    );
    assert_eq!(code(parse_html(bad).map(drop)), ErrorCode::UnknownElement);
}

#[test]
fn parse_html_returns_the_flow_with_directives_as_markers() {
    let flow = parse_html("<p>x<t:footnote>n</t:footnote></p>").expect("parse");
    let view = flow.view();
    let p = flow.roots()[0];
    assert_eq!(view.tag(p), Tag::Html("p".into()));
    assert_eq!(
        view.tag(view.children(p)[1]),
        Tag::Marker(TKind::Footnote.marker())
    );
}

#[test]
fn flow_debug_shows_the_markup() {
    let (program, _) =
        turbo_html2pdf_core::compile("<p>dbg</p>", &Default::default()).expect("compile");
    let (flow, _) = program
        .render_nodes(&serde_json::json!({}), None)
        .expect("render");
    let shown = format!("{flow:?}");
    assert!(
        shown.starts_with("Flow {") && shown.contains("dbg"),
        "{shown}"
    );
}
