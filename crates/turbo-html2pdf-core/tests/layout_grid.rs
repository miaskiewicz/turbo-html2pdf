//! CSS Grid layout (taffy-backed): `grid-template-columns`/`-rows`, `fr`/`px`/`%`
//! tracks, `repeat()`, gaps, and auto-placement. Driven through boxgen ->
//! layout_tree, mirroring the flex harness.

mod common;

use turbo_html2pdf_core::layout::block::layout_tree;
use turbo_html2pdf_core::layout::boxgen::build_box_tree;
use turbo_html2pdf_core::layout::fragment::Fragment;
use turbo_html2pdf_core::node::Tag;
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

fn grid(container_extra: &[(&str, &str)], items: Vec<StyledNode>, cw: f32) -> Fragment {
    let mut pairs = vec![("display", "grid")];
    pairs.extend_from_slice(container_extra);
    let root = build_box_tree(&[el("div", &pairs, items)]);
    let mut diags = Diagnostics::default();
    layout_tree(&root, cw, &common::registry(), &mut diags)
}

fn items_of(root: &Fragment) -> Vec<Fragment> {
    root.children[0].children.clone()
}

#[test]
fn two_fr_columns_split_width_side_by_side() {
    let root = grid(
        &[("grid-template-columns", "1fr 1fr")],
        vec![item(&[], "a"), item(&[], "b")],
        200.0,
    );
    let its = items_of(&root);
    assert_eq!(its.len(), 2);
    // Same row, equal columns, each ~half the width.
    assert_eq!(its[0].y, its[1].y, "same row");
    assert!(its[1].x > its[0].x, "second column to the right");
    assert!((its[0].width - 100.0).abs() < 1.0, "col1 half width");
    assert!((its[1].width - 100.0).abs() < 1.0, "col2 half width");
}

#[test]
fn third_item_wraps_to_second_row() {
    // Two columns, three items: the third auto-places onto row 2, column 1.
    let root = grid(
        &[("grid-template-columns", "1fr 1fr")],
        vec![item(&[], "a"), item(&[], "b"), item(&[], "c")],
        200.0,
    );
    let its = items_of(&root);
    assert_eq!(its.len(), 3);
    assert!(its[2].y > its[0].y, "third item on a lower row");
    assert!(
        (its[2].x - its[0].x).abs() < 1.0,
        "third item back in column 1"
    );
}

#[test]
fn grid_column_and_row_span_widen_items() {
    // `grid-column:span N` (nike's header logo/nav each span 6 of 12) must place the
    // item across N tracks, not auto-place it one column wide. A 4-col grid: item A
    // spans 3 columns (3/4 width), item B spans the last column (also `grid-row:span
    // 1`, exercising the row placement path).
    let root = grid(
        &[("grid-template-columns", "repeat(4, 1fr)")],
        vec![
            item(&[("grid-column", "span 3")], "a"),
            item(&[("grid-column", "span 1"), ("grid-row", "span 1")], "b"),
        ],
        400.0,
    );
    let its = items_of(&root);
    assert!(
        (its[0].width - 300.0).abs() < 2.0,
        "span 3 -> 3/4 width, got {}",
        its[0].width
    );
    assert!(
        (its[1].width - 100.0).abs() < 2.0,
        "span 1 -> 1/4 width, got {}",
        its[1].width
    );
    assert_eq!(its[0].y, its[1].y, "both on row 1");
}

#[test]
fn fixed_px_and_fr_columns_mix() {
    // `200px 1fr` in a 500px grid: col1 = 200px, col2 = remaining 300px.
    let root = grid(
        &[("grid-template-columns", "200px 1fr")],
        vec![item(&[], "a"), item(&[], "b")],
        500.0,
    );
    let its = items_of(&root);
    assert!((its[0].width - 200.0).abs() < 1.0, "fixed 200px col");
    assert!((its[1].width - 300.0).abs() < 1.0, "fr col takes the rest");
}

#[test]
fn repeat_expands_track_list() {
    // `repeat(3, 1fr)` = three equal columns in a 300px grid → 100px each.
    let root = grid(
        &[("grid-template-columns", "repeat(3, 1fr)")],
        vec![item(&[], "a"), item(&[], "b"), item(&[], "c")],
        300.0,
    );
    let its = items_of(&root);
    assert_eq!(its.len(), 3);
    assert_eq!(its[0].y, its[1].y, "all on row 1");
    assert_eq!(its[1].y, its[2].y);
    for it in &its {
        assert!((it.width - 100.0).abs() < 1.0, "each column ~100px");
    }
}

#[test]
fn named_areas_place_items_by_grid_area() {
    // `grid-template-areas` + `grid-area` place items into named cells regardless
    // of DOM order (this is how Wikipedia's Vector skin lays out sidebar + body).
    let root = grid(
        &[
            ("grid-template-columns", "100px 1fr"),
            ("grid-template-areas", "'side main'"),
        ],
        vec![
            item(&[("grid-area", "main")], "m"), // main first in DOM…
            item(&[("grid-area", "side")], "s"), // …side second
        ],
        400.0,
    );
    let its = items_of(&root);
    let (main, side) = (&its[0], &its[1]);
    assert!((side.x - 0.0).abs() < 1.0, "side lands in column 1");
    assert!(
        (main.x - 100.0).abs() < 1.0,
        "main lands in column 2 despite coming first in the DOM (got {})",
        main.x
    );
    assert_eq!(main.y, side.y, "same row");
}

#[test]
fn minmax_track_fills_remaining_width() {
    // `100px minmax(0, 1fr)`: fixed 100px sidebar, flexible main fills the rest.
    let root = grid(
        &[("grid-template-columns", "100px minmax(0, 1fr)")],
        vec![item(&[], "a"), item(&[], "b")],
        400.0,
    );
    let its = items_of(&root);
    assert!((its[0].width - 100.0).abs() < 1.0, "fixed 100px track");
    assert!(
        (its[1].width - 300.0).abs() < 1.0,
        "minmax(0,1fr) takes the rest"
    );
}

#[test]
fn empty_grid_container_has_no_items() {
    // A grid whose only child is collapsible whitespace produces no grid items, so
    // layout returns early with an empty item list and zero content height.
    let root = grid(
        &[("grid-template-columns", "1fr")],
        vec![StyledNode::Text("   ".into())],
        300.0,
    );
    assert!(items_of(&root).is_empty());
    assert_eq!(root.children[0].height, 0.0);
}

#[test]
fn column_gap_spaces_tracks() {
    // Two 1fr columns with a 20px gap in 220px: each column = 100px, second
    // starts at 120px.
    let root = grid(
        &[("grid-template-columns", "1fr 1fr"), ("column-gap", "20px")],
        vec![item(&[], "a"), item(&[], "b")],
        220.0,
    );
    let its = items_of(&root);
    assert!((its[0].width - 100.0).abs() < 1.0);
    assert!(
        (its[1].x - its[0].x - 120.0).abs() < 1.0,
        "gap between columns"
    );
}
