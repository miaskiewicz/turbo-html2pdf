//! Flex layout (§5.3, AC-5.6): taffy-backed grow/shrink/basis, direction, wrap,
//! gap, justify/align, and content sizing. Driven through boxgen -> layout_tree.
//!
//! Split at the S1 relocation: the layout-only cases moved to turbo-dom-layout's
//! `tests/layout_flex.rs`; the ones here need TKind directives, templating or the PDF
//! emitter.

mod common;

use turbo_html2pdf_core::layout::block::layout_tree;
use turbo_html2pdf_core::layout::boxgen::build_box_tree;
use turbo_html2pdf_core::layout::fragment::Fragment;
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

fn flex(container_extra: &[(&str, &str)], items: Vec<StyledNode>, cw: f32) -> Fragment {
    let mut pairs = vec![("display", "flex")];
    pairs.extend_from_slice(container_extra);
    let root = build_box_tree(&[el("div", &pairs, items)]);
    let mut diags = Diagnostics::default();
    layout_tree(&root, cw, &common::registry(), &mut diags)
}

/// The laid-out flex item fragments.
fn items_of(root: &Fragment) -> Vec<Fragment> {
    root.children[0].children.clone()
}

#[test]
fn directive_item_has_zero_natural_width() {
    let d = StyledNode::Element(StyledElement {
        tag: Tag::Marker(TKind::Anchor.marker()),
        attrs: vec![],
        style: cs(&[]),
        children: vec![],
        source: None,
    });
    let root = flex(&[], vec![d], 500.0);
    assert_eq!(items_of(&root).len(), 1);
    assert_eq!(items_of(&root)[0].width, 0.0);
}
