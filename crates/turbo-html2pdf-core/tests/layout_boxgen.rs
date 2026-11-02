//! Box generation (§5.1): styled tree -> box tree, anonymous-block wrapping of
//! inline runs (AC-5.1), `display:none` dropping, and `t:` directive markers.
//!
//! Split at the S1 relocation: the layout-only cases moved to turbo-dom-layout's
//! `tests/layout_boxgen.rs`; the ones here need TKind directives, templating or the PDF
//! emitter.

use turbo_html2pdf_core::layout::boxgen::*;
use turbo_html2pdf_core::tag::Tag;
use turbo_html2pdf_core::TKind;
use turbo_html2pdf_core::{ComputedStyle, StyledElement, StyledNode};

fn cs(pairs: &[(&str, &str)]) -> ComputedStyle {
    ComputedStyle::from_pairs(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
}

fn el(tag: &str, pairs: &[(&str, &str)], children: Vec<StyledNode>) -> StyledNode {
    StyledNode::Element(StyledElement {
        tag: Tag::Html(tag.to_string()),
        attrs: vec![],
        style: cs(pairs),
        children,
        source: None,
    })
}

fn dir(kind: TKind, children: Vec<StyledNode>) -> StyledNode {
    StyledNode::Element(StyledElement {
        tag: Tag::Marker(kind.marker()),
        attrs: vec![],
        style: cs(&[]),
        children,
        source: None,
    })
}

fn txt(s: &str) -> StyledNode {
    StyledNode::Text(s.into())
}

fn as_block(kind: &BoxKind) -> Vec<LayoutBox> {
    match kind {
        BoxKind::Block(b) => b.clone(),
        _ => panic!("expected Block"),
    }
}

fn as_lines(kind: &BoxKind) -> Vec<InlineItem> {
    match kind {
        BoxKind::Lines(items) => items.clone(),
        _ => panic!("expected Lines"),
    }
}

#[test]
fn inline_directive_is_inline_item() {
    let nodes = vec![txt("a"), dir(TKind::Footnote, vec![txt("note")]), txt("b")];
    let items = as_lines(&build_box_tree(&nodes).kind).to_vec();
    assert_eq!(items.len(), 3);
    assert!(matches!(
        items[1],
        InlineItem::Marker { kind, .. } if TKind::from_marker(kind) == Some(TKind::Footnote)
    ));
}

#[test]
fn block_directive_is_opaque_marker() {
    let nodes = vec![
        dir(TKind::RunningHeader, vec![]),
        el("p", &[], vec![txt("x")]),
    ];
    let boxes = as_block(&build_box_tree(&nodes).kind);
    assert!(matches!(
        boxes[0].kind,
        BoxKind::Marker(k) if TKind::from_marker(k) == Some(TKind::RunningHeader)
    ));
}

#[test]
fn flex_keeps_directive_child() {
    let nodes = vec![el(
        "div",
        &[("display", "flex")],
        vec![dir(TKind::Anchor, vec![])],
    )];
    let boxes = as_block(&build_box_tree(&nodes).kind);
    match &boxes[0].kind {
        BoxKind::Flex(items) => assert!(
            matches!(items[0].kind, BoxKind::Marker(k) if TKind::from_marker(k) == Some(TKind::Anchor))
        ),
        _ => panic!("expected Flex"),
    }
}
