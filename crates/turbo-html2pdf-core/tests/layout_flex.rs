//! Flex layout (§5.3, AC-5.6): taffy-backed grow/shrink/basis, direction, wrap,
//! gap, justify/align, and content sizing. Driven through boxgen -> layout_tree.

mod common;

use turbo_html2pdf_core::layout::block::layout_tree;
use turbo_html2pdf_core::layout::boxgen::build_box_tree;
use turbo_html2pdf_core::layout::fragment::Fragment;
use turbo_html2pdf_core::node::{TKind, Tag};
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
    })
}

fn item(pairs: &[(&str, &str)], text: &str) -> StyledNode {
    el("div", pairs, vec![StyledNode::Text(text.to_string())])
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
fn grow_splits_free_space_equally() {
    let root = flex(
        &[],
        vec![
            item(&[("flex-grow", "1")], "a"),
            item(&[("flex-grow", "1")], "a"),
        ],
        200.0,
    );
    let its = items_of(&root);
    assert_eq!(its.len(), 2);
    assert!((its[0].width - its[1].width).abs() < 1.0);
    assert!((its[0].width + its[1].width - 200.0).abs() < 1.0);
    // row: laid side by side
    assert!(its[1].x > its[0].x);
    assert_eq!(its[0].y, its[1].y);
}

#[test]
fn explicit_basis_sizes_items() {
    let root = flex(
        &[],
        vec![
            item(&[("flex-basis", "80px"), ("flex-shrink", "0")], "a"),
            item(&[("flex-basis", "40px"), ("flex-shrink", "0")], "a"),
        ],
        500.0,
    );
    let its = items_of(&root);
    assert_eq!(its[0].width, 80.0);
    assert_eq!(its[1].width, 40.0);
}

#[test]
fn auto_basis_keyword_is_accepted() {
    let root = flex(&[], vec![item(&[("flex-basis", "auto")], "hello")], 500.0);
    assert_eq!(items_of(&root).len(), 1);
}

#[test]
fn explicit_width_item_sizes_to_width() {
    let root = flex(
        &[],
        vec![item(&[("width", "60px"), ("flex-shrink", "0")], "a")],
        500.0,
    );
    assert_eq!(items_of(&root)[0].width, 60.0);
}

#[test]
fn content_sized_item_uses_natural_width() {
    let root = flex(&[], vec![item(&[], "hello world")], 500.0);
    let w = items_of(&root)[0].width;
    assert!(w > 0.0 && w < 500.0); // shrink-to-content, not filling
}

#[test]
fn percent_width_item_fills_via_flex_basis() {
    // `flex-basis:auto` (initial) defers to `width`, so a `width:100%` flex item
    // fills the row instead of shrink-wrapping. Wikipedia's `.mw-header` is a
    // `width:100%` grid that is itself a flex child; without this it collapsed to
    // its content width and the whole top bar was narrow + centered.
    let root = flex(
        &[],
        vec![item(&[("width", "100%"), ("flex-shrink", "0")], "hi")],
        500.0,
    );
    assert!((items_of(&root)[0].width - 500.0).abs() < 1.0);
}

#[test]
fn nested_block_item_measures_children() {
    let inner = el(
        "div",
        &[],
        vec![el("p", &[], vec![StyledNode::Text("hello".into())])],
    );
    let root = flex(&[], vec![inner], 500.0);
    assert!(items_of(&root)[0].width > 0.0);
}

#[test]
fn directive_item_has_zero_natural_width() {
    let d = StyledNode::Element(StyledElement {
        tag: Tag::Directive(TKind::Anchor),
        attrs: vec![],
        style: cs(&[]),
        children: vec![],
    });
    let root = flex(&[], vec![d], 500.0);
    assert_eq!(items_of(&root).len(), 1);
    assert_eq!(items_of(&root)[0].width, 0.0);
}

#[test]
fn direction_keywords_render() {
    for dir in ["row", "row-reverse", "column", "column-reverse"] {
        let root = flex(
            &[("flex-direction", dir)],
            vec![item(&[], "a"), item(&[], "b")],
            300.0,
        );
        assert_eq!(items_of(&root).len(), 2);
    }
    // column stacks vertically
    let col = flex(
        &[("flex-direction", "column")],
        vec![item(&[], "a"), item(&[], "b")],
        300.0,
    );
    let its = items_of(&col);
    assert!(its[1].y > its[0].y);
}

#[test]
fn wrap_keywords_render_and_wrap() {
    for w in ["nowrap", "wrap", "wrap-reverse"] {
        let root = flex(
            &[("flex-wrap", w)],
            vec![
                item(&[("flex-basis", "100px"), ("flex-shrink", "0")], "a"),
                item(&[("flex-basis", "100px"), ("flex-shrink", "0")], "b"),
                item(&[("flex-basis", "100px"), ("flex-shrink", "0")], "c"),
            ],
            150.0,
        );
        assert_eq!(items_of(&root).len(), 3);
    }
    // wrap pushes the overflowing item to a new line (y increases)
    let wrapped = flex(
        &[("flex-wrap", "wrap")],
        vec![
            item(&[("flex-basis", "100px"), ("flex-shrink", "0")], "a"),
            item(&[("flex-basis", "100px"), ("flex-shrink", "0")], "b"),
        ],
        150.0,
    );
    let its = items_of(&wrapped);
    assert!(its[1].y > its[0].y);
}

#[test]
fn justify_keywords_render() {
    for j in [
        "flex-start",
        "flex-end",
        "center",
        "space-between",
        "space-around",
        "space-evenly",
    ] {
        let root = flex(
            &[("justify-content", j)],
            vec![item(&[], "a"), item(&[], "b")],
            400.0,
        );
        assert_eq!(items_of(&root).len(), 2);
    }
    // flex-end pushes content to the right vs flex-start
    let start = flex(
        &[("justify-content", "flex-start")],
        vec![item(&[], "a")],
        400.0,
    );
    let end = flex(
        &[("justify-content", "flex-end")],
        vec![item(&[], "a")],
        400.0,
    );
    assert!(items_of(&end)[0].x > items_of(&start)[0].x);
}

#[test]
fn align_keywords_render() {
    for a in ["stretch", "flex-start", "flex-end", "center", "baseline"] {
        let root = flex(
            &[("align-items", a)],
            vec![item(&[], "a"), item(&[], "b")],
            300.0,
        );
        assert_eq!(items_of(&root).len(), 2);
    }
}

#[test]
fn gap_adds_space_between_items() {
    let root = flex(
        &[("gap", "20px")],
        vec![
            item(&[("flex-basis", "50px"), ("flex-shrink", "0")], "a"),
            item(&[("flex-basis", "50px"), ("flex-shrink", "0")], "b"),
        ],
        500.0,
    );
    let its = items_of(&root);
    assert!(its[1].x >= its[0].x + 50.0 + 20.0 - 0.5);
}

#[test]
fn empty_flex_container_has_no_items() {
    let root = flex(&[], vec![StyledNode::Text("   ".into())], 500.0);
    assert!(items_of(&root).is_empty());
    assert_eq!(root.children[0].height, 0.0);
}

#[test]
fn deeply_nested_flex_is_not_exponential() {
    // Each flex container's items are probed by taffy several times per solve, and
    // each probe recurses a full sub-layout, so nested flex was exponential in depth
    // (~8 levels stopped completing). The per-box measure cache collapses the repeat
    // probes; a depth-7 tree must lay out — this test completing is the assertion.
    use turbo_html2pdf_core::{layout_html, Diagnostics, FontRegistry};
    let mut html = String::from("<span>leaf</span>");
    for _ in 0..7 {
        html = format!("<div style=\"display:flex\"><div>a</div>{html}<div>b</div></div>");
    }
    let mut d = Diagnostics::default();
    let f = layout_html(
        &format!("<body>{html}</body>"),
        "",
        800.0,
        &FontRegistry::new(),
        &mut d,
    )
    .expect("layout");
    assert!(f.height > 0.0);
}
