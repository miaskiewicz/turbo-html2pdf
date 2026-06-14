//! Box generation (§5.1): styled tree -> box tree, anonymous-block wrapping of
//! inline runs (AC-5.1), `display:none` dropping, and `t:` directive markers.

use turbo_pdf_core::layout::boxgen::*;
use turbo_pdf_core::layout::value::Display;
use turbo_pdf_core::node::{TKind, Tag};
use turbo_pdf_core::{ComputedStyle, StyledElement, StyledNode};

fn cs(pairs: &[(&str, &str)]) -> ComputedStyle {
    ComputedStyle::from_pairs(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
}

fn el(tag: &str, pairs: &[(&str, &str)], children: Vec<StyledNode>) -> StyledNode {
    StyledNode::Element(StyledElement {
        tag: Tag::Html(tag.to_string()),
        attrs: vec![],
        style: cs(pairs),
        children,
    })
}

fn dir(kind: TKind, children: Vec<StyledNode>) -> StyledNode {
    StyledNode::Element(StyledElement {
        tag: Tag::Directive(kind),
        attrs: vec![],
        style: cs(&[]),
        children,
    })
}

fn txt(s: &str) -> StyledNode {
    StyledNode::Text(s.to_string())
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
fn text_only_flow_is_inline_context() {
    let root = build_box_tree(&[txt("hello world")]);
    assert_eq!(root.node_id.0, 0);
    assert_eq!(root.display, Display::Block);
    assert_eq!(as_lines(&root.kind).len(), 1);
}

#[test]
fn empty_flow_is_empty_lines() {
    let root = build_box_tree(&[]);
    assert!(as_lines(&root.kind).is_empty());
}

#[test]
fn mixed_block_inline_wraps_anonymous_block() {
    // "before" <span>mid</span> <p>para</p>  -> [anon Lines(before,mid), block p]
    let nodes = vec![
        txt("before"),
        el("span", &[("display", "inline")], vec![txt("mid")]),
        el("p", &[], vec![txt("para")]),
    ];
    let root = build_box_tree(&nodes);
    let boxes = as_block(&root.kind);
    assert_eq!(boxes.len(), 2);
    assert_eq!(as_lines(&boxes[0].kind).len(), 2); // before + mid in one anon block
    assert_eq!(boxes[1].display, Display::Block);
    assert_eq!(as_lines(&boxes[1].kind).len(), 1); // the <p>'s text
}

#[test]
fn display_none_is_dropped() {
    let nodes = vec![
        el("p", &[("display", "none")], vec![txt("hidden")]),
        el("p", &[], vec![txt("shown")]),
    ];
    let boxes = as_block(&build_box_tree(&nodes).kind);
    assert_eq!(boxes.len(), 1);
}

#[test]
fn inline_block_becomes_atomic() {
    let nodes = vec![el("span", &[("display", "inline-block")], vec![txt("x")])];
    let items = as_lines(&build_box_tree(&nodes).kind).to_vec();
    assert!(matches!(items[0], InlineItem::Atomic(_)));
}

#[test]
fn block_nested_in_inline_becomes_atomic() {
    let nodes = vec![el(
        "span",
        &[("display", "inline")],
        vec![el("div", &[], vec![txt("d")])],
    )];
    let items = as_lines(&build_box_tree(&nodes).kind).to_vec();
    assert_eq!(items.len(), 1);
    assert!(matches!(items[0], InlineItem::Atomic(_)));
}

#[test]
fn inline_element_drops_hidden_child() {
    let nodes = vec![el(
        "span",
        &[("display", "inline")],
        vec![
            el("em", &[("display", "none")], vec![txt("hidden")]),
            txt("vis"),
        ],
    )];
    let items = as_lines(&build_box_tree(&nodes).kind).to_vec();
    assert_eq!(items.len(), 1); // only "vis"
}

#[test]
fn inline_directive_is_inline_item() {
    let nodes = vec![txt("a"), dir(TKind::Footnote, vec![txt("note")]), txt("b")];
    let items = as_lines(&build_box_tree(&nodes).kind).to_vec();
    assert_eq!(items.len(), 3);
    assert!(matches!(
        items[1],
        InlineItem::Directive {
            kind: TKind::Footnote,
            ..
        }
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
        BoxKind::Directive(TKind::RunningHeader)
    ));
}

#[test]
fn flex_container_keeps_only_real_items() {
    let nodes = vec![el(
        "div",
        &[("display", "flex")],
        vec![
            el("div", &[], vec![txt("a")]),
            txt("   "),
            el("div", &[("display", "none")], vec![]),
        ],
    )];
    let boxes = as_block(&build_box_tree(&nodes).kind);
    match &boxes[0].kind {
        BoxKind::Flex(items) => assert_eq!(items.len(), 1),
        _ => panic!("expected Flex"),
    }
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
        BoxKind::Flex(items) => assert!(matches!(items[0].kind, BoxKind::Directive(TKind::Anchor))),
        _ => panic!("expected Flex"),
    }
}

#[test]
fn table_structure_preserved() {
    let nodes = vec![el(
        "table",
        &[("display", "table")],
        vec![el(
            "tr",
            &[("display", "table-row")],
            vec![el("td", &[("display", "table-cell")], vec![txt("c")])],
        )],
    )];
    let boxes = as_block(&build_box_tree(&nodes).kind);
    let rows = match &boxes[0].kind {
        BoxKind::Table(rows) => rows,
        _ => panic!("expected Table"),
    };
    assert_eq!(rows[0].display, Display::TableRow);
    let cells = as_block(&rows[0].kind);
    assert_eq!(cells[0].display, Display::TableCell);
}

#[test]
fn whitespace_between_blocks_makes_no_anonymous_box() {
    let nodes = vec![
        el("p", &[], vec![txt("a")]),
        txt("   "),
        el("p", &[], vec![txt("b")]),
    ];
    let boxes = as_block(&build_box_tree(&nodes).kind);
    assert_eq!(boxes.len(), 2); // blank run dropped
}

#[test]
fn nonblank_inline_run_between_blocks_makes_anonymous_box() {
    let nodes = vec![
        el("p", &[], vec![txt("a")]),
        txt("x"),
        el("p", &[], vec![txt("b")]),
    ];
    let boxes = as_block(&build_box_tree(&nodes).kind);
    assert_eq!(boxes.len(), 3);
    assert_eq!(as_lines(&boxes[1].kind).len(), 1); // anon block holding "x"
}

#[test]
fn node_ids_are_preorder() {
    let root = build_box_tree(&[txt("a")]);
    assert_eq!(root.node_id.0, 0);
    match &as_lines(&root.kind)[0] {
        InlineItem::Text { node_id, .. } => assert_eq!(node_id.0, 1),
        _ => panic!("expected text"),
    }
}
