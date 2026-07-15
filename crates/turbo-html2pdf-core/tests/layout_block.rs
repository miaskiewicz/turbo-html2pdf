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
fn border_box_min_height_includes_padding_and_border() {
    // Under `box-sizing:border-box` a `min-height` is the BORDER-box height, so the
    // padding+border live inside it (google's "Sign in" pill:
    // min-height:40;padding:10px;border:1px). The box is 40px tall — not 40 + 20 + 2.
    let root = lay(
        &[el(
            "div",
            &[
                ("min-height", "40px"),
                ("padding", "10px"),
                ("border", "1px solid black"),
                ("box-sizing", "border-box"),
            ],
            vec![],
        )],
        500.0,
    );
    assert!(
        (root.children[0].height - 40.0).abs() < 1.0,
        "border-box min-height 40 -> 40px tall, got {}",
        root.children[0].height
    );
    // The same box as content-box grows by the padding+border (40 + 20 + 2 = 62).
    let cbox = lay(
        &[el(
            "div",
            &[
                ("min-height", "40px"),
                ("padding", "10px"),
                ("border", "1px solid black"),
            ],
            vec![],
        )],
        500.0,
    );
    assert!(
        (cbox.children[0].height - 62.0).abs() < 1.0,
        "content-box min-height 40 -> 62px tall, got {}",
        cbox.children[0].height
    );
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
fn inline_block_auto_width_honors_min_width() {
    // An auto-width (shrink-to-fit) inline-block whose content is far narrower than
    // its `min-width` must widen to the min-width — google's header "Sign in" pill
    // (min-width:85px) otherwise shrank to its text and rendered round, not a pill.
    let root = lay(
        &[el(
            "div",
            &[],
            vec![ib(
                &[("min-width", "100px"), ("background-color", "#123456")],
                "x",
            )],
        )],
        500.0,
    );
    let bx = bg_boxes(&root);
    assert_eq!(bx.len(), 1);
    assert!(
        bx[0].width >= 100.0,
        "auto-width inline-block should honor min-width, got {}",
        bx[0].width
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

/// The `border-radius` (px) of a box fragment, or `None` for a non-box.
fn box_radius(f: &Fragment) -> Option<f32> {
    match &f.content {
        FragmentContent::Box { border_radius, .. } => Some(*border_radius),
        _ => None,
    }
}

#[test]
fn absolute_right_inset_anchors_to_right_edge() {
    // `position:absolute` with a `right` inset but no `left`: the box's right edge
    // is placed `right` px in from the containing block's right edge
    // (out_of_flow_origin's `(None, Some(r))` arm).
    let root = lay(
        &[el(
            "div",
            &[
                ("position", "relative"),
                ("width", "300px"),
                ("height", "100px"),
            ],
            vec![el(
                "div",
                &[
                    ("position", "absolute"),
                    ("right", "10px"),
                    ("width", "50px"),
                    ("height", "20px"),
                    ("background-color", "#ff0000"),
                ],
                vec![],
            )],
        )],
        500.0,
    );
    let inner = &bg_boxes(&root)[0];
    // cb = the relative div's content box (x=0, width=300). right:10, bbw=50 →
    // x = 0 + 300 - 50 - 10 = 240.
    assert!((inner.x - 240.0).abs() < 1.0, "got x={}", inner.x);
}

#[test]
fn percent_height_positioned_box_resolves_against_definite_ancestor() {
    // A positioned box with a `%` height inside a positioned ancestor of definite
    // (px) height exercises `definite_content_height`'s `Pct` + border-box-inset
    // arms (the height is exposed to the box's own `%`-height descendants).
    let root = lay(
        &[el(
            "div",
            &[
                ("position", "relative"),
                ("width", "300px"),
                ("height", "200px"),
            ],
            vec![el(
                "div",
                &[
                    ("position", "absolute"),
                    ("top", "0"),
                    ("left", "0"),
                    ("width", "100px"),
                    ("height", "50%"),
                    ("box-sizing", "border-box"),
                    ("padding", "10px"),
                    ("background-color", "#00ff00"),
                ],
                vec![],
            )],
        )],
        500.0,
    );
    let inner = &bg_boxes(&root)[0];
    assert!((inner.width - 100.0).abs() < 1.0, "got w={}", inner.width);
}

#[test]
fn max_height_clamps_taller_content() {
    // A `max-height` shorter than the content clamps the box height
    // (`content_box_height`'s max-height arm).
    let root = lay(
        &[el(
            "div",
            &[("max-height", "10px"), ("background-color", "#ff0000")],
            vec![el("div", &[("height", "100px")], vec![])],
        )],
        500.0,
    );
    let outer = &bg_boxes(&root)[0];
    assert!((outer.height - 10.0).abs() < 0.5, "got h={}", outer.height);
}

#[test]
fn shrink_float_with_inline_block_measures_content() {
    // An auto-width float shrinks to its content (shrink-to-fit float sizing); its
    // inline content includes an inline-block, so the natural-width measurement
    // walks a Lines box that mixes text and a non-text (atomic) item.
    let root = lay(
        &[el(
            "div",
            &[],
            vec![
                el(
                    "div",
                    &[("float", "left"), ("background-color", "#ff0000")],
                    vec![
                        txt("Hi"),
                        ib(
                            &[
                                ("width", "20px"),
                                ("height", "10px"),
                                ("background-color", "#00ff00"),
                            ],
                            "",
                        ),
                    ],
                ),
                el("p", &[], vec![txt("after the float")]),
            ],
        )],
        500.0,
    );
    let floatbox = &bg_boxes(&root)[0];
    assert!(
        floatbox.width > 0.0 && floatbox.width < 400.0,
        "shrink-to-fit float width {}",
        floatbox.width
    );
}

#[test]
fn floats_narrow_region_and_clear_drops_below() {
    // Text flows in the region narrowed by BOTH a left and a right float, then
    // `clear:left`/`clear:right`/`clear:both` blocks drop below the cleared floats.
    let root = lay(
        &[el(
            "div",
            &[],
            vec![
                fl("left", "80px", "#ff0000"),
                fl("right", "80px", "#00ff00"),
                el(
                    "p",
                    &[],
                    vec![txt("text flows between the two floats in the narrow band")],
                ),
                el(
                    "div",
                    &[("clear", "left"), ("background-color", "#0000ff")],
                    vec![txt("cl")],
                ),
                el(
                    "div",
                    &[("clear", "right"), ("background-color", "#ff00ff")],
                    vec![txt("cr")],
                ),
                el(
                    "div",
                    &[("clear", "both"), ("background-color", "#00ffff")],
                    vec![txt("cb")],
                ),
            ],
        )],
        300.0,
    );
    let line = all(&root)
        .into_iter()
        .find(|f| matches!(f.content, FragmentContent::TextLine { .. }))
        .expect("paragraph line");
    // left float (80px) pushes the text region's start to x>=80 (right float pulls
    // its end in — the right-float branch of `inline_region_from`).
    assert!(
        line.x >= 80.0 - 1.0,
        "text right of left float, x={}",
        line.x
    );
    // clear:left box is bg_boxes[2] (after the two floats); it drops to the left
    // float's bottom (y=30).
    let cleared = &bg_boxes(&root)[2];
    assert!(
        cleared.y >= 30.0 - 0.5,
        "clear:left dropped below float, y={}",
        cleared.y
    );
}

#[test]
fn floats_drop_below_when_row_full() {
    // Three 120px floats in a 300px box: two fit on row 1, the third can't so it
    // drops past the nearest float bottom (`float_drop_y`'s finite-drop retry).
    let root = lay(
        &[el(
            "div",
            &[],
            vec![
                fl("left", "120px", "#ff0000"),
                fl("left", "120px", "#00ff00"),
                fl("left", "120px", "#0000ff"),
            ],
        )],
        300.0,
    );
    let bx = bg_boxes(&root);
    assert_eq!(bx.len(), 3);
    assert!(
        bx[0].y.abs() < 0.5 && bx[1].y.abs() < 0.5,
        "first two on row 1"
    );
    assert!((bx[2].y - 30.0).abs() < 0.5, "third dropped, y={}", bx[2].y);
}

#[test]
fn oversized_float_stays_put_no_finite_drop() {
    // A float wider than the whole container never fits any row, so `float_drop_y`
    // finds no lower float to drop past (`next` is infinite) and leaves it at y0.
    let root = lay(
        &[el(
            "div",
            &[],
            vec![el(
                "div",
                &[
                    ("float", "left"),
                    ("width", "400px"),
                    ("height", "20px"),
                    ("background-color", "#ff0000"),
                ],
                vec![],
            )],
        )],
        200.0,
    );
    let bx = &bg_boxes(&root)[0];
    assert!(bx.y.abs() < 0.5, "oversized float stays at y0, y={}", bx.y);
    assert!((bx.width - 400.0).abs() < 1.0, "width={}", bx.width);
}

#[test]
fn text_align_right_pushes_block_child_to_right() {
    // A `text-align:right` container right-aligns a width-constrained block child
    // (`block_h_offset`'s Align::Right arm).
    let root = lay(
        &[el(
            "div",
            &[("text-align", "right")],
            vec![el(
                "div",
                &[("width", "100px"), ("background-color", "#ff0000")],
                vec![],
            )],
        )],
        500.0,
    );
    let inner = &bg_boxes(&root)[0];
    assert!(
        (inner.x - 400.0).abs() < 1.0,
        "100px block right-aligned in 500, got x={}",
        inner.x
    );
}

#[test]
fn right_float_table_reanchors_to_right_edge() {
    // A right-floated table declared narrower than its content grows past the
    // reserved width during layout; `reanchor_right_float` re-pins its right edge
    // to the container's right edge.
    let root = lay(
        &[el(
            "div",
            &[],
            vec![el(
                "table",
                &[
                    ("float", "right"),
                    ("display", "table"),
                    ("width", "20px"),
                    ("background-color", "#ff0000"),
                ],
                vec![el(
                    "tr",
                    &[("display", "table-row")],
                    vec![el(
                        "td",
                        &[("display", "table-cell")],
                        vec![txt("this is wide table content")],
                    )],
                )],
            )],
        )],
        400.0,
    );
    let t = &bg_boxes(&root)[0];
    assert!(
        t.width > 20.0,
        "table grew to its content, width={}",
        t.width
    );
    assert!(
        (t.x + t.width - 400.0).abs() < 2.0,
        "right edge pinned to container right, right={}",
        t.x + t.width
    );
}

#[test]
fn mask_image_without_resolver_skips_tint() {
    // A `mask-image` box laid with no image resolver: the mask source can't be
    // probed, so no tinted image fragment is prepended (the probe-fail return).
    let root = lay(
        &[el(
            "div",
            &[
                ("mask-image", "url(icon.svg)"),
                ("width", "20px"),
                ("height", "20px"),
                ("background-color", "#ff0000"),
            ],
            vec![],
        )],
        200.0,
    );
    let div = &root.children[0];
    assert!((div.height - 20.0).abs() < 0.5, "h={}", div.height);
    assert!(div.children.is_empty(), "no mask tint fragment inserted");
}

#[test]
fn percent_border_radius_resolves_against_box() {
    // `border-radius:50%` resolves against the shorter box side and clamps to half
    // (a 40px square → a 20px radius circle).
    let root = lay(
        &[el(
            "div",
            &[
                ("width", "40px"),
                ("height", "40px"),
                ("border-radius", "50%"),
                ("background-color", "#ff0000"),
            ],
            vec![],
        )],
        200.0,
    );
    let r = box_radius(&root.children[0]).expect("box");
    assert!((r - 20.0).abs() < 0.5, "50% of 40 → 20, got {}", r);
}

#[test]
fn tall_float_forces_clear_block_to_drop_below() {
    // A 100px-tall left float with only a tiny sibling before the `clear:left`
    // block: the cursor is still near the top when the clear is reached, so the
    // clear actually drops it past the float bottom (the `cleared > base` arm).
    let root = lay(
        &[el(
            "div",
            &[],
            vec![
                el(
                    "div",
                    &[
                        ("float", "left"),
                        ("width", "60px"),
                        ("height", "100px"),
                        ("background-color", "#ff0000"),
                    ],
                    vec![],
                ),
                el(
                    "div",
                    &[("clear", "left"), ("background-color", "#0000ff")],
                    vec![txt("after")],
                ),
            ],
        )],
        300.0,
    );
    // The cleared block must sit below the 100px float, not beside it near y=0.
    let cleared = all(&root)
        .into_iter()
        .filter(|f| matches!(f.content, FragmentContent::Box { .. }))
        .find(|f| f.y >= 100.0);
    assert!(
        cleared.is_some(),
        "clear:left block should drop below the 100px float"
    );
}

#[test]
fn right_float_narrows_line_region_from_the_right() {
    // A tall `float:right` overlapping the paragraph's first line: the line's
    // region end is pulled in from the right (the `Float::Right` region arm).
    let root = lay(
        &[el(
            "div",
            &[],
            vec![
                el(
                    "div",
                    &[
                        ("float", "right"),
                        ("width", "120px"),
                        ("height", "80px"),
                        ("background-color", "#00ff00"),
                    ],
                    vec![],
                ),
                el("p", &[], vec![txt("x")]),
            ],
        )],
        300.0,
    );
    let line = all(&root)
        .into_iter()
        .find(|f| matches!(f.content, FragmentContent::TextLine { .. }))
        .expect("paragraph line");
    // The 120px right float (x0=180) pulls the line's usable width below the full
    // 300px container width.
    assert!(
        line.width <= 180.5,
        "right float should narrow the line, got {}",
        line.width
    );
}

#[test]
fn border_radius_auto_resolves_to_zero() {
    // `border-radius:auto` (parsed to `LengthPct::Auto`) resolves to a 0 radius.
    let root = lay(
        &[el(
            "div",
            &[
                ("width", "40px"),
                ("height", "40px"),
                ("border-radius", "auto"),
                ("background-color", "#ff0000"),
            ],
            vec![],
        )],
        200.0,
    );
    let r = box_radius(&root.children[0]).expect("box");
    assert_eq!(r, 0.0, "border-radius:auto → 0");
}

/// The bg overlay is the first child of the `relative` card whose `background`
/// paints — i.e. the second card child (index 1). Return its fragment.
fn overlay_of(root: &Fragment) -> &Fragment {
    &root.children[0].children[1]
}

#[test]
fn absolute_percent_height_child_uses_relative_parent_content_height() {
    // A `relative` card sized by a 300px in-flow block; a `bottom:0; height:50%`
    // overlay must be 150px tall (half the card) and sit in the card's lower half
    // (y 150..300), NOT collapse to its text and drop below the card.
    let root = lay(
        &[el(
            "div",
            &[("position", "relative")],
            vec![
                el("div", &[("height", "300px")], vec![]),
                el(
                    "div",
                    &[
                        ("position", "absolute"),
                        ("height", "50%"),
                        ("bottom", "0"),
                        ("background-color", "#f00"),
                    ],
                    vec![],
                ),
            ],
        )],
        400.0,
    );
    let overlay = overlay_of(&root);
    assert!(
        (overlay.height - 150.0).abs() < 1.0,
        "50% of 300, got {}",
        overlay.height
    );
    assert!(
        (overlay.y - 150.0).abs() < 1.0,
        "bottom:0 anchors to card bottom (y=150), got {}",
        overlay.y
    );
}

#[test]
fn absolute_flex_overlay_fills_its_band_at_card_bottom() {
    // The nike editorial-card shape: a flex overlay with `bottom:0; height:33.33%`
    // over an auto-height card. It must occupy the card's bottom third with its
    // text un-clipped inside, not render below the card image.
    let root = lay(
        &[el(
            "div",
            &[("position", "relative")],
            vec![
                el("div", &[("height", "300px")], vec![]),
                el(
                    "div",
                    &[
                        ("position", "absolute"),
                        ("display", "flex"),
                        ("height", "33.33333%"),
                        ("bottom", "0"),
                        ("background-color", "#f00"),
                    ],
                    vec![el("div", &[], vec![txt("Fast Sprints?")])],
                ),
            ],
        )],
        400.0,
    );
    let overlay = overlay_of(&root);
    assert!(
        (overlay.height - 100.0).abs() < 1.0,
        "33.33% of 300, got {}",
        overlay.height
    );
    assert!(
        (overlay.y - 200.0).abs() < 1.0,
        "sits at card bottom (y=200), got {}",
        overlay.y
    );
}

#[test]
fn deferred_absolute_with_top_inset_anchors_from_the_top() {
    // A deferred `absolute` child with an explicit `top` (no `bottom`) keeps its
    // top anchor — `anchor_bottom` must leave it alone.
    let root = lay(
        &[el(
            "div",
            &[("position", "relative")],
            vec![
                el("div", &[("height", "300px")], vec![]),
                el(
                    "div",
                    &[
                        ("position", "absolute"),
                        ("top", "40px"),
                        ("background-color", "#0f0"),
                    ],
                    vec![el("div", &[("height", "20px")], vec![])],
                ),
            ],
        )],
        400.0,
    );
    let overlay = overlay_of(&root);
    assert!(
        (overlay.y - 40.0).abs() < 1.0,
        "top:40px anchors from the top, got {}",
        overlay.y
    );
}

#[test]
fn deferred_absolute_without_vertical_inset_keeps_static_y() {
    // A deferred `absolute` child with neither `top` nor `bottom` stays at its
    // static y — `anchor_bottom` sees no `bottom` and does not move it.
    let root = lay(
        &[el(
            "div",
            &[("position", "relative")],
            vec![
                el("div", &[("height", "80px")], vec![]),
                el(
                    "div",
                    &[
                        ("position", "absolute"),
                        ("left", "0"),
                        ("background-color", "#00f"),
                    ],
                    vec![el("div", &[("height", "10px")], vec![])],
                ),
            ],
        )],
        400.0,
    );
    let overlay = overlay_of(&root);
    assert!(
        (overlay.y - 80.0).abs() < 1.0,
        "no top/bottom → static y (after the 80px block), got {}",
        overlay.y
    );
}

#[test]
fn in_flow_percent_height_stays_content_derived() {
    // An in-flow `%` height has no definite basis in the flow model, so the box
    // still sizes to its content (the `positioned_pct_height` guard is in-flow →
    // `None`), not to the percentage.
    let root = lay(
        &[el(
            "div",
            &[("height", "50%")],
            vec![el("div", &[("height", "30px")], vec![])],
        )],
        400.0,
    );
    assert!(
        (root.children[0].height - 30.0).abs() < 1.0,
        "in-flow 50% → content height (30), got {}",
        root.children[0].height
    );
}

#[test]
fn aspect_ratio_sizes_auto_height_from_width() {
    // A `aspect-ratio:1` box 200px wide with auto height (and no in-flow content)
    // is a 200px square, not collapsed — overriding a smaller `min-height`.
    let root = lay(
        &[el(
            "div",
            &[
                ("width", "200px"),
                ("aspect-ratio", "1"),
                ("min-height", "40px"),
            ],
            vec![],
        )],
        400.0,
    );
    assert!(
        (root.children[0].height - 200.0).abs() < 1.0,
        "square 200, got {}",
        root.children[0].height
    );
}

#[test]
fn aspect_ratio_ignored_when_height_is_explicit() {
    // An explicit height wins; the ratio does not override it.
    let root = lay(
        &[el(
            "div",
            &[
                ("width", "200px"),
                ("height", "60px"),
                ("aspect-ratio", "1"),
            ],
            vec![],
        )],
        400.0,
    );
    assert!(
        (root.children[0].height - 60.0).abs() < 1.0,
        "explicit 60, got {}",
        root.children[0].height
    );
}

#[test]
fn absolute_calc_percent_minus_px_top_positions_inside_parent() {
    // A `top: calc(50% - 10px)` absolute box in a 200px-tall relative parent sits at
    // y = 100 - 10 = 90 (calc mixing `%` and px must resolve, not fall to `auto`).
    let root = lay(
        &[el(
            "div",
            &[("position", "relative"), ("height", "200px")],
            vec![el(
                "div",
                &[
                    ("position", "absolute"),
                    ("top", "calc(50% - 10px)"),
                    ("height", "20px"),
                    ("background-color", "#f00"),
                ],
                vec![],
            )],
        )],
        400.0,
    );
    let overlay = &root.children[0].children[0];
    assert!(
        (overlay.y - 90.0).abs() < 1.0,
        "calc(50% - 10px) of 200 = 90, got {}",
        overlay.y
    );
}

#[test]
fn margin_inline_start_maps_to_left_offset() {
    // A flow-relative `margin-inline-start` (LTR) offsets the box from the left, so
    // Google's search-bar "AI Mode" label clears its icon instead of overprinting it.
    let root = lay(
        &[el(
            "div",
            &[
                ("margin-inline-start", "20px"),
                ("width", "50px"),
                ("height", "10px"),
            ],
            vec![],
        )],
        400.0,
    );
    assert!(
        (root.children[0].x - 20.0).abs() < 0.5,
        "margin-inline-start:20px -> x=20, got {}",
        root.children[0].x
    );
}
