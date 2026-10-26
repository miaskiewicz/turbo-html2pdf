//! Block layout (§5.3, AC-5.5): width resolution, margin collapsing (sibling +
//! empty-block collapse-through), Lines/atomic/directive content, and the
//! Flex/Table block-flow fallback. Driven through the real boxgen -> layout pipe.
//!
//! Split at the S1 relocation: the layout-only cases moved to turbo-dom-layout's
//! `tests/layout_block.rs`; the ones here need TKind directives, templating or the PDF
//! emitter.

mod common;

use turbo_html2pdf_core::layout::block::layout_tree;
use turbo_html2pdf_core::layout::boxgen::build_box_tree;
use turbo_html2pdf_core::layout::fragment::{Fragment, FragmentContent};
use turbo_html2pdf_core::tag::Tag;
use turbo_html2pdf_core::TKind;
use turbo_html2pdf_core::{ComputedStyle, Diagnostics, StyledElement, StyledNode};

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

#[test]
fn block_directive_is_zero_size_marker() {
    let root = lay(&[dir(TKind::RunningHeader, vec![])], 500.0);
    let d = &root.children[0];
    assert_eq!(d.height, 0.0);
    assert!(matches!(
        d.content,
        FragmentContent::Marker(k) if TKind::from_marker(k) == Some(TKind::RunningHeader)
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
        .any(|f| matches!(f.content, FragmentContent::Marker(k) if TKind::from_marker(k) == Some(TKind::Footnote))));
}
