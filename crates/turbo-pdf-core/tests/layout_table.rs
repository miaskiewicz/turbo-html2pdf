//! Table layout (§5.4, AC-5.7–5.10): grid with colspan/rowspan, fixed + auto
//! column widths, per-cell vertical-align, and thead/tfoot repeatable marking.

mod common;

use turbo_pdf_core::layout::block::layout_tree;
use turbo_pdf_core::layout::boxgen::build_box_tree;
use turbo_pdf_core::layout::fragment::{Fragment, FragmentContent, RepeatKind};
use turbo_pdf_core::node::{Attr, Tag};
use turbo_pdf_core::{ComputedStyle, Diagnostics, StyledElement, StyledNode};

fn cs(pairs: &[(&str, &str)]) -> ComputedStyle {
    ComputedStyle::from_pairs(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
}

fn ela(
    tag: &str,
    attrs: &[(&str, &str)],
    pairs: &[(&str, &str)],
    children: Vec<StyledNode>,
) -> StyledNode {
    StyledNode::Element(StyledElement {
        tag: Tag::Html(tag.to_string()),
        attrs: attrs
            .iter()
            .map(|(n, v)| Attr {
                name: n.to_string(),
                value: v.to_string(),
            })
            .collect(),
        style: cs(pairs),
        children,
    })
}

fn td(attrs: &[(&str, &str)], text: &str) -> StyledNode {
    ela(
        "td",
        attrs,
        &[("display", "table-cell")],
        vec![StyledNode::Text(text.to_string())],
    )
}

fn tr(cells: Vec<StyledNode>) -> StyledNode {
    ela("tr", &[], &[("display", "table-row")], cells)
}

fn group(display: &str, rows: Vec<StyledNode>) -> StyledNode {
    ela("g", &[], &[("display", display)], rows)
}

fn table(rows: Vec<StyledNode>, table_pairs: &[(&str, &str)], cw: f32) -> Fragment {
    let mut tp = vec![("display", "table")];
    tp.extend_from_slice(table_pairs);
    let root = build_box_tree(&[ela("table", &[], &tp, rows)]);
    let mut d = Diagnostics::default();
    layout_tree(&root, cw, &common::registry(), &mut d).children[0].clone()
}

fn rows(t: &Fragment) -> Vec<Fragment> {
    t.children.clone()
}

fn cells(row: &Fragment) -> Vec<Fragment> {
    row.children.clone()
}

fn find_text_y(f: &Fragment) -> Option<f32> {
    if matches!(f.content, FragmentContent::TextLine { .. }) {
        return Some(f.y);
    }
    f.children.iter().find_map(find_text_y)
}

#[test]
fn simple_two_by_two_grid() {
    let t = table(
        vec![
            tr(vec![td(&[], "a"), td(&[], "b")]),
            tr(vec![td(&[], "c"), td(&[], "d")]),
        ],
        &[],
        500.0,
    );
    assert_eq!(rows(&t).len(), 2);
    let r0 = rows(&t);
    assert_eq!(cells(&r0[0]).len(), 2);
    // second column is to the right; second row is below.
    assert!(cells(&r0[0])[1].x > cells(&r0[0])[0].x);
    assert!(r0[1].y > r0[0].y);
}

#[test]
fn colspan_widens_cell() {
    let t = table(
        vec![
            tr(vec![td(&[("colspan", "2")], "wide")]),
            tr(vec![td(&[], "a"), td(&[], "b")]),
        ],
        &[("table-layout", "fixed"), ("width", "300px")],
        500.0,
    );
    let wide = cells(&rows(&t)[0])[0].width;
    let narrow = cells(&rows(&t)[1])[0].width;
    assert!(wide > narrow * 1.5); // spans ~2 columns
}

#[test]
fn rowspan_occupies_next_row_column() {
    // r0: [A rowspan2][B] ; r1: [C] -> C must land in column 1 (col 0 taken by A)
    let t = table(
        vec![
            tr(vec![td(&[("rowspan", "2")], "A"), td(&[], "B")]),
            tr(vec![td(&[], "C")]),
        ],
        &[],
        500.0,
    );
    let r = rows(&t);
    let c_x = cells(&r[1])[0].x;
    let b_x = cells(&r[0])[1].x;
    assert!((c_x - b_x).abs() < 1.0); // C aligns under B (column 1)
}

#[test]
fn thead_tfoot_rows_are_repeatable() {
    let t = table(
        vec![
            group("table-header-group", vec![tr(vec![td(&[], "h")])]),
            tr(vec![td(&[], "body")]),
            group("table-footer-group", vec![tr(vec![td(&[], "f")])]),
        ],
        &[],
        500.0,
    );
    let r = rows(&t);
    assert_eq!(r[0].break_meta.repeatable, Some(RepeatKind::Header));
    assert_eq!(r[1].break_meta.repeatable, None);
    assert_eq!(r[2].break_meta.repeatable, Some(RepeatKind::Footer));
}

#[test]
fn fixed_layout_splits_equally() {
    let t = table(
        vec![tr(vec![td(&[], "a"), td(&[], "b"), td(&[], "c")])],
        &[("table-layout", "fixed"), ("width", "300px")],
        500.0,
    );
    for c in cells(&rows(&t)[0]) {
        assert!((c.width - 100.0).abs() < 0.5);
    }
}

#[test]
fn fixed_layout_honors_explicit_then_shares() {
    // the first cell gets an explicit width; the rest share the remaining space.
    let first = ela(
        "td",
        &[],
        &[("display", "table-cell"), ("width", "120px")],
        vec![StyledNode::Text("a".into())],
    );
    let t = table(
        vec![tr(vec![first, td(&[], "b"), td(&[], "c")])],
        &[("table-layout", "fixed"), ("width", "300px")],
        500.0,
    );
    let cs = cells(&rows(&t)[0]);
    assert_eq!(cs[0].width, 120.0);
    assert!((cs[1].width - 90.0).abs() < 0.5); // (300-120)/2
}

#[test]
fn auto_layout_shrinks_to_content() {
    let t = table(vec![tr(vec![td(&[], "hi"), td(&[], "yo")])], &[], 500.0);
    let total: f32 = cells(&rows(&t)[0]).iter().map(|c| c.width).sum();
    assert!(total > 0.0 && total < 500.0);
}

#[test]
fn auto_layout_scales_down_on_overflow() {
    // narrow container forces the content-sized columns to scale to fit
    let t = table(
        vec![tr(vec![
            td(&[], "a long stretch of words here"),
            td(&[], "another long stretch of words"),
        ])],
        &[],
        80.0,
    );
    let total: f32 = cells(&rows(&t)[0]).iter().map(|c| c.width).sum();
    assert!((total - 80.0).abs() < 1.0);
}

#[test]
fn explicit_table_width_scales_columns() {
    let t = table(
        vec![tr(vec![td(&[], "a"), td(&[], "b")])],
        &[("width", "200px")],
        500.0,
    );
    let total: f32 = cells(&rows(&t)[0]).iter().map(|c| c.width).sum();
    assert!((total - 200.0).abs() < 1.0);
}

#[test]
fn vertical_align_shifts_cell_content() {
    fn build(valign: &str) -> f32 {
        // a tall cell (narrow + long text) next to a short cell with the valign
        let tall = ela(
            "td",
            &[],
            &[("display", "table-cell"), ("width", "40px")],
            vec![StyledNode::Text("word word word word word word".into())],
        );
        let short = ela(
            "td",
            &[],
            &[("display", "table-cell"), ("vertical-align", valign)],
            vec![StyledNode::Text("x".into())],
        );
        let t = table(vec![tr(vec![tall, short])], &[], 500.0);
        let row = &rows(&t)[0];
        find_text_y(&cells(row)[1]).unwrap()
    }
    assert!(build("bottom") > build("top"));
}

#[test]
fn empty_table_has_no_rows() {
    let t = table(vec![tr(vec![StyledNode::Text("   ".into())])], &[], 500.0);
    // the row has no cells -> no columns -> table content empty, height 0
    assert!(t.children.is_empty());
}

#[test]
fn text_only_group_contributes_no_cells() {
    // thead whose content is bare text -> rows_of yields none; a real row follows
    let t = table(
        vec![
            group("table-header-group", vec![StyledNode::Text("stray".into())]),
            tr(vec![td(&[], "real")]),
        ],
        &[],
        500.0,
    );
    // the text-only header group is a Lines box, so it yields no rows.
    assert_eq!(rows(&t).len(), 1);
}

#[test]
fn plain_row_group_is_not_repeatable() {
    // a tbody-style group (display not header/footer) yields non-repeatable rows
    let t = table(
        vec![group("table-row-group", vec![tr(vec![td(&[], "a")])])],
        &[],
        500.0,
    );
    let r = rows(&t);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].break_meta.repeatable, None);
}

#[test]
fn fixed_layout_all_columns_explicit() {
    // every column specified -> nothing left to share (share count == 0)
    let only = ela(
        "td",
        &[],
        &[("display", "table-cell"), ("width", "300px")],
        vec![StyledNode::Text("a".into())],
    );
    let t = table(
        vec![tr(vec![only])],
        &[("table-layout", "fixed"), ("width", "300px")],
        500.0,
    );
    assert_eq!(cells(&rows(&t)[0])[0].width, 300.0);
}

#[test]
fn rowspan_cell_taller_than_rows_expands() {
    // a tall rowspan cell forces the spanned rows to grow to contain it
    let tall = ela(
        "td",
        &[("rowspan", "2")],
        &[("display", "table-cell"), ("width", "40px")],
        vec![StyledNode::Text(
            "word word word word word word word".into(),
        )],
    );
    let t = table(
        vec![tr(vec![tall, td(&[], "B")]), tr(vec![td(&[], "C")])],
        &[],
        500.0,
    );
    let r = rows(&t);
    assert_eq!(r.len(), 2);
    // the two rows together must be at least as tall as the spanning cell.
    let total = r[0].height + r[1].height;
    assert!(total >= cells(&r[0])[0].height - 0.01);
}

#[test]
fn span_attrs_tolerate_garbage() {
    // colspan="abc" and rowspan="0" both fall back to 1
    let t = table(
        vec![tr(vec![
            td(&[("colspan", "abc"), ("rowspan", "0")], "a"),
            td(&[], "b"),
        ])],
        &[],
        500.0,
    );
    assert_eq!(cells(&rows(&t)[0]).len(), 2);
}

#[test]
fn colspan_cell_can_exceed_column_sum() {
    // a wide colspan cell whose content exceeds the two columns' natural widths
    let t = table(
        vec![
            tr(vec![td(
                &[("colspan", "2")],
                "an extremely wide spanning header cell",
            )]),
            tr(vec![td(&[], "a"), td(&[], "b")]),
        ],
        &[],
        500.0,
    );
    assert_eq!(rows(&t).len(), 2);
    assert!(cells(&rows(&t)[0])[0].width > 0.0);
}
