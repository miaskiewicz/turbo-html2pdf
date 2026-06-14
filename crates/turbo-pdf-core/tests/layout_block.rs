//! Block layout (§5.3, AC-5.5): width resolution, margin collapsing (sibling +
//! empty-block collapse-through), Lines/atomic/directive content, and the
//! Flex/Table block-flow fallback. Driven through the real boxgen -> layout pipe.

mod common;

use turbo_pdf_core::layout::block::layout_tree;
use turbo_pdf_core::layout::boxgen::build_box_tree;
use turbo_pdf_core::layout::fragment::{Fragment, FragmentContent};
use turbo_pdf_core::layout::value::BreakRule;
use turbo_pdf_core::node::{TKind, Tag};
use turbo_pdf_core::text::FontRegistry;
use turbo_pdf_core::{ComputedStyle, Diagnostics, StyledElement, StyledNode};

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

fn lay(nodes: &[StyledNode], cb_width: f32) -> Fragment {
    let root = build_box_tree(nodes);
    let mut diags = Diagnostics::default();
    layout_tree(&root, cb_width, &common::registry(), &mut diags)
}

fn collect<'a>(f: &'a Fragment, out: &mut Vec<&'a Fragment>) {
    out.push(f);
    for c in &f.children {
        collect(c, out);
    }
}

fn all(f: &Fragment) -> Vec<&Fragment> {
    let mut v = Vec::new();
    collect(f, &mut v);
    v
}

fn text_lines(f: &Fragment) -> usize {
    all(f)
        .iter()
        .filter(|g| matches!(g.content, FragmentContent::TextLine { .. }))
        .count()
}

fn gap(prev: &Fragment, next: &Fragment) -> f32 {
    next.y - (prev.y + prev.height)
}

#[test]
fn auto_width_fills_and_padding_offsets_content() {
    let root = lay(&[el("div", &[("padding", "10px")], vec![txt("hi")])], 500.0);
    assert_eq!(root.width, 500.0); // root fills cb
    let div = &root.children[0];
    assert_eq!(div.width, 500.0); // auto fills cb
    let line = &div.children[0];
    assert_eq!((line.x, line.y), (10.0, 10.0)); // padding offset
    assert!(matches!(line.content, FragmentContent::TextLine { .. }));
}

#[test]
fn content_box_width_adds_padding() {
    let root = lay(
        &[el(
            "div",
            &[("width", "100px"), ("padding", "10px")],
            vec![],
        )],
        500.0,
    );
    assert_eq!(root.children[0].width, 120.0); // 100 content + 2*10 padding
}

#[test]
fn border_box_width_includes_padding() {
    let root = lay(
        &[el(
            "div",
            &[
                ("width", "100px"),
                ("padding", "10px"),
                ("box-sizing", "border-box"),
            ],
            vec![],
        )],
        500.0,
    );
    assert_eq!(root.children[0].width, 100.0);
}

#[test]
fn min_and_max_width_clamp() {
    let small = lay(
        &[el(
            "div",
            &[("width", "50px"), ("min-width", "80px")],
            vec![],
        )],
        500.0,
    );
    assert_eq!(small.children[0].width, 80.0);
    let big = lay(
        &[el(
            "div",
            &[("width", "200px"), ("max-width", "120px")],
            vec![],
        )],
        500.0,
    );
    assert_eq!(big.children[0].width, 120.0);
}

#[test]
fn explicit_height_is_honored() {
    let root = lay(&[el("div", &[("height", "200px")], vec![])], 500.0);
    assert_eq!(root.children[0].height, 200.0);
}

#[test]
fn background_and_border_become_box_content() {
    let root = lay(
        &[el(
            "div",
            &[("background-color", "red"), ("border", "2px solid blue")],
            vec![],
        )],
        500.0,
    );
    match &root.children[0].content {
        FragmentContent::Box { background, border } => {
            assert!(background.is_some());
            assert_eq!(border.top.width, 2);
        }
        _ => panic!("expected box"),
    }
}

#[test]
fn sibling_margins_collapse_to_max() {
    let root = lay(
        &[
            el("p", &[("margin", "20px")], vec![txt("a")]),
            el("p", &[("margin", "20px")], vec![txt("b")]),
        ],
        500.0,
    );
    // collapsed gap is max(20, 20) = 20, not the 40 of summing.
    assert_eq!(gap(&root.children[0], &root.children[1]), 20.0);
}

#[test]
fn empty_block_collapses_through() {
    let root = lay(
        &[
            el("p", &[("margin", "20px")], vec![txt("a")]),
            el("div", &[("margin", "30px")], vec![]), // empty, height 0
            el("p", &[("margin", "10px")], vec![txt("b")]),
        ],
        500.0,
    );
    assert_eq!(root.children[1].height, 0.0); // empty div
                                              // gap between the two paragraphs = max(20, 30, 30, 10) = 30.
    assert_eq!(gap(&root.children[0], &root.children[2]), 30.0);
}

#[test]
fn flex_and_table_fall_back_to_block_flow() {
    let flex = lay(
        &[el(
            "div",
            &[("display", "flex")],
            vec![el("div", &[], vec![txt("a")])],
        )],
        500.0,
    );
    assert!(text_lines(&flex) >= 1);
    let table = lay(
        &[el(
            "table",
            &[("display", "table")],
            vec![el(
                "tr",
                &[("display", "table-row")],
                vec![el("td", &[("display", "table-cell")], vec![txt("c")])],
            )],
        )],
        500.0,
    );
    assert!(text_lines(&table) >= 1);
}

#[test]
fn block_directive_is_zero_size_marker() {
    let root = lay(&[dir(TKind::RunningHeader, vec![])], 500.0);
    let d = &root.children[0];
    assert_eq!(d.height, 0.0);
    assert!(matches!(
        d.content,
        FragmentContent::Directive(TKind::RunningHeader)
    ));
}

#[test]
fn lines_handle_text_directive_and_atomic() {
    let root = lay(
        &[el(
            "p",
            &[],
            vec![
                txt("a"),
                dir(TKind::Footnote, vec![txt("note")]),
                el("span", &[("display", "inline-block")], vec![txt("b")]),
            ],
        )],
        500.0,
    );
    let frags = all(&root);
    assert!(frags
        .iter()
        .any(|f| matches!(f.content, FragmentContent::TextLine { .. })));
    assert!(frags
        .iter()
        .any(|f| matches!(f.content, FragmentContent::Directive(TKind::Footnote))));
}

#[test]
fn empty_registry_renders_no_text_lines() {
    let root = build_box_tree(&[el("p", &[], vec![txt("hi")])]);
    let mut diags = Diagnostics::default();
    let frag = layout_tree(&root, 500.0, &FontRegistry::new(), &mut diags);
    assert_eq!(text_lines(&frag), 0); // no face selectable -> runs dropped
}

#[test]
fn break_properties_propagate_to_meta() {
    let root = lay(
        &[el(
            "div",
            &[
                ("break-before", "page"),
                ("break-inside", "avoid"),
                ("orphans", "3"),
            ],
            vec![],
        )],
        500.0,
    );
    let m = &root.children[0].break_meta;
    assert_eq!(m.break_before, BreakRule::Page);
    assert!(m.break_inside_avoid);
    assert_eq!(m.orphans, 3);
}
