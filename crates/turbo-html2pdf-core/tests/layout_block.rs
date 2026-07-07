//! Block layout (§5.3, AC-5.5): width resolution, margin collapsing (sibling +
//! empty-block collapse-through), Lines/atomic/directive content, and the
//! Flex/Table block-flow fallback. Driven through the real boxgen -> layout pipe.

mod common;

use turbo_html2pdf_core::layout::block::layout_tree;
use turbo_html2pdf_core::layout::boxgen::build_box_tree;
use turbo_html2pdf_core::layout::fragment::{Fragment, FragmentContent};
use turbo_html2pdf_core::layout::value::BreakRule;
use turbo_html2pdf_core::node::{TKind, Tag};
use turbo_html2pdf_core::text::FontRegistry;
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
        FragmentContent::Box {
            background, border, ..
        } => {
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

/// The `Box` fragments carrying a background (the coloured probe boxes), in
/// document order.
fn bg_boxes(root: &Fragment) -> Vec<&Fragment> {
    all(root)
        .into_iter()
        .filter(|f| {
            matches!(
                &f.content,
                FragmentContent::Box {
                    background: Some(_),
                    ..
                }
            )
        })
        .collect()
}

/// An `inline-block` probe: distinct background, optional explicit size + text.
fn ib(pairs: &[(&str, &str)], text: &str) -> StyledNode {
    let mut p = vec![("display", "inline-block")];
    p.extend_from_slice(pairs);
    let kids = if text.is_empty() {
        vec![]
    } else {
        vec![txt(text)]
    };
    el("span", &p, kids)
}

#[test]
fn inline_blocks_flow_horizontally() {
    // Two inline-blocks sit side by side on one row (not stacked vertically).
    let root = lay(
        &[el(
            "div",
            &[],
            vec![
                ib(
                    &[
                        ("width", "60px"),
                        ("height", "20px"),
                        ("background-color", "#ff0000"),
                    ],
                    "",
                ),
                ib(
                    &[
                        ("width", "60px"),
                        ("height", "20px"),
                        ("background-color", "#00ff00"),
                    ],
                    "",
                ),
            ],
        )],
        500.0,
    );
    let bx = bg_boxes(&root);
    assert_eq!(bx.len(), 2);
    assert_eq!(bx[0].y, bx[1].y, "same row");
    assert!(
        (bx[1].x - bx[0].x - 60.0).abs() < 1.0,
        "packed side by side"
    );
}

#[test]
fn inline_blocks_wrap_when_row_full() {
    // Three 80px inline-blocks in a 200px box: two fit on row 1, the third wraps.
    let mk = |c: &str| {
        ib(
            &[
                ("width", "80px"),
                ("height", "20px"),
                ("background-color", c),
            ],
            "",
        )
    };
    let root = lay(
        &[el(
            "div",
            &[],
            vec![mk("#ff0000"), mk("#00ff00"), mk("#0000ff")],
        )],
        200.0,
    );
    let bx = bg_boxes(&root);
    assert_eq!(bx.len(), 3);
    assert_eq!(bx[0].y, bx[1].y, "first two on row 1");
    assert!(bx[2].y > bx[0].y, "third wraps to row 2");
    assert!(
        (bx[2].x - bx[0].x).abs() < 1.0,
        "third back at the row start"
    );
}

#[test]
fn auto_width_inline_block_shrinks_to_content() {
    // An auto-width inline-block shrinks to its content instead of filling the
    // 500px line, so two of them share a row.
    let root = lay(
        &[el(
            "div",
            &[],
            vec![
                ib(&[("background-color", "#ff0000")], "hi"),
                ib(&[("background-color", "#00ff00")], "yo"),
            ],
        )],
        500.0,
    );
    let bx = bg_boxes(&root);
    assert_eq!(bx.len(), 2);
    assert!(
        bx[0].width < 200.0,
        "shrinks to content, not the full 500px line (got {})",
        bx[0].width
    );
    assert_eq!(bx[0].y, bx[1].y, "same row");
    assert!(
        bx[1].x > bx[0].x + 1.0,
        "second sits to the right of the first"
    );
}

/// A floated probe box: distinct background + explicit size.
fn fl(side: &str, w: &str, c: &str) -> StyledNode {
    el(
        "div",
        &[
            ("float", side),
            ("width", w),
            ("height", "30px"),
            ("background-color", c),
        ],
        vec![],
    )
}

#[test]
fn left_floats_pack_side_by_side() {
    // Two `float:left` boxes sit on one row at the left edge (not stacked).
    let root = lay(
        &[el(
            "div",
            &[],
            vec![fl("left", "60px", "#ff0000"), fl("left", "60px", "#00ff00")],
        )],
        500.0,
    );
    let bx = bg_boxes(&root);
    assert_eq!(bx.len(), 2);
    assert_eq!(bx[0].y, bx[1].y, "same row");
    assert!((bx[0].x - 0.0).abs() < 1.0, "first at the left edge");
    assert!((bx[1].x - 60.0).abs() < 1.0, "second packed right after it");
}

#[test]
fn left_and_right_floats_go_to_opposite_edges() {
    let root = lay(
        &[el(
            "div",
            &[],
            vec![
                fl("left", "80px", "#ff0000"),
                fl("right", "80px", "#00ff00"),
            ],
        )],
        500.0,
    );
    let bx = bg_boxes(&root);
    assert!((bx[0].x - 0.0).abs() < 1.0, "left float at x=0");
    assert!(
        (bx[1].x - (500.0 - 80.0)).abs() < 1.0,
        "right float at the right edge (got {})",
        bx[1].x
    );
    assert_eq!(bx[0].y, bx[1].y, "same band row");
}

#[test]
fn in_flow_content_flows_beside_floats() {
    // A `float:left` box, then an in-flow paragraph: the paragraph flows *beside*
    // the float (to its right, within the float's height band), not below it.
    let root = lay(
        &[el(
            "div",
            &[],
            vec![
                fl("left", "80px", "#ff0000"),
                el("p", &[], vec![txt("after")]),
            ],
        )],
        500.0,
    );
    let floatbox = &bg_boxes(&root)[0];
    let line = all(&root)
        .into_iter()
        .find(|f| matches!(f.content, FragmentContent::TextLine { .. }))
        .expect("paragraph line");
    assert!(
        line.x >= floatbox.x + floatbox.width - 1.0,
        "in-flow text sits to the right of the float (float right {}, text x {})",
        floatbox.x + floatbox.width,
        line.x
    );
    assert!(
        line.y < floatbox.y + floatbox.height - 0.5,
        "in-flow text is beside the float, not cleared below it (float bottom {}, text y {})",
        floatbox.y + floatbox.height,
        line.y
    );
}

#[test]
fn empty_registry_renders_no_text_lines() {
    let root = build_box_tree(&[el("p", &[], vec![txt("hi")])]);
    let mut diags = Diagnostics::default();
    // `default()` is a genuinely empty registry (no caller *and* no bundled
    // faces, even with the `bundled-fonts` feature on, which `new()` would
    // populate); with no selectable face the runs are dropped.
    let frag = layout_tree(&root, 500.0, &FontRegistry::default(), &mut diags);
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

#[test]
fn visibility_hidden_and_opacity_zero_drop_the_box() {
    // visibility:hidden / opacity:0 boxes are not rendered (this is what keeps
    // Wikipedia's click/hover-revealed nav dropdowns hidden in a static shot).
    let vis = lay(
        &[el(
            "div",
            &[("visibility", "hidden"), ("background-color", "#ff0000")],
            vec![txt("x")],
        )],
        200.0,
    );
    assert!(bg_boxes(&vis).is_empty(), "visibility:hidden box dropped");
    let op = lay(
        &[el(
            "div",
            &[("opacity", "0"), ("background-color", "#00ff00")],
            vec![txt("y")],
        )],
        200.0,
    );
    assert!(bg_boxes(&op).is_empty(), "opacity:0 box dropped");
    // A normal box still renders.
    let visible = lay(
        &[el("div", &[("background-color", "#0000ff")], vec![])],
        200.0,
    );
    assert_eq!(bg_boxes(&visible).len(), 1);
}

#[test]
fn text_align_center_and_margin_auto_center_width_constrained_blocks() {
    // A `text-align:center` container (like `<center>`) centers a narrow block
    // child; a full-width child is untouched.
    let centered = lay(
        &[el(
            "div",
            &[("text-align", "center")],
            vec![el(
                "div",
                &[("width", "100px"), ("background-color", "#ff0000")],
                vec![],
            )],
        )],
        500.0,
    );
    let inner = &bg_boxes(&centered)[0];
    assert!(
        (inner.x - 200.0).abs() < 1.0,
        "100px block centered in 500 (got x={})",
        inner.x
    );

    // `margin: 0 auto` centers regardless of container align.
    let mauto = lay(
        &[el(
            "div",
            &[
                ("width", "100px"),
                ("margin", "0 auto"),
                ("background-color", "#00ff00"),
            ],
            vec![],
        )],
        500.0,
    );
    assert!(
        (bg_boxes(&mauto)[0].x - 200.0).abs() < 1.0,
        "margin:auto centers"
    );
}

#[test]
fn inline_block_flows_within_the_line_next_to_text() {
    // An inline-block after text sits on the SAME line, to the right of the text
    // (not stacked below) — this is what puts HN's footer search box next to
    // "Search:" and its nav logo inline with the title.
    let root = lay(
        &[el(
            "div",
            &[],
            vec![
                txt("Search: "),
                ib(
                    &[
                        ("width", "40px"),
                        ("height", "16px"),
                        ("background-color", "#ff0000"),
                    ],
                    "",
                ),
            ],
        )],
        500.0,
    );
    let atom = &bg_boxes(&root)[0];
    let line = all(&root)
        .into_iter()
        .find(|f| matches!(f.content, FragmentContent::TextLine { .. }))
        .expect("text line");
    assert!(atom.x > 20.0, "atom sits after the text (got x={})", atom.x);
    assert!(
        (atom.y - line.y).abs() < 20.0,
        "atom on the same line as the text (atom y={}, line y={})",
        atom.y,
        line.y
    );
}
