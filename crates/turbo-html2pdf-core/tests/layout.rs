//! End-to-end layout (§5): the whole Stage 0–3 pipeline — compile a template,
//! render it to nodes, cascade into a styled tree, and lay it out into the
//! galley. Exercises the UA defaults (real HTML tables and inline elements) and
//! the `NodeId` round-trip (AC-5.11).

mod common;

use std::collections::HashSet;

use turbo_html2pdf_core::layout::boxgen::build_box_tree;
use turbo_html2pdf_core::layout::fragment::{Fragment, FragmentContent, NodeId};
use turbo_html2pdf_core::style::TokenSet;
use turbo_html2pdf_core::{
    build_cascade, compile, layout, style_tree, CompileOptions, Diagnostics, StyledNode,
};

fn render(template: &str) -> Vec<StyledNode> {
    let (program, _) = compile(template, &CompileOptions::default()).expect("compile");
    let (nodes, _) = program
        .render_nodes(&serde_json::json!({"name": "World"}), None)
        .expect("render");
    let cascade = build_cascade("", "", TokenSet::default());
    style_tree(&nodes, &cascade)
}

fn galley(template: &str) -> Fragment {
    let styled = render(template);
    let mut diags = Diagnostics::default();
    layout(&styled, 600.0, &common::registry(), &mut diags)
}

fn walk<'a>(f: &'a Fragment, out: &mut Vec<&'a Fragment>) {
    out.push(f);
    for c in &f.children {
        walk(c, out);
    }
}

fn all(f: &Fragment) -> Vec<&Fragment> {
    let mut v = Vec::new();
    walk(f, &mut v);
    v
}

fn count_text(f: &Fragment) -> usize {
    all(f)
        .iter()
        .filter(|g| matches!(g.content, FragmentContent::TextLine { .. }))
        .count()
}

#[test]
fn paragraph_renders_text() {
    let g = galley("<p>Hello {{ name }}</p>");
    assert_eq!(g.node_id, NodeId(0)); // root
    assert!(count_text(&g) >= 1);
}

#[test]
fn real_table_lays_out_via_ua_defaults() {
    // No author CSS: the UA sheet alone must make <table>/<tr>/<td> a table.
    let g = galley("<table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>");
    let cells: Vec<&Fragment> = all(&g)
        .into_iter()
        .filter(|f| matches!(f.content, FragmentContent::TextLine { .. }))
        .collect();
    assert_eq!(cells.len(), 4); // four cells of text
                                // the two columns sit at different x; the two rows at different y.
    assert!(
        cells
            .iter()
            .map(|c| c.x as i32)
            .collect::<HashSet<_>>()
            .len()
            >= 2
    );
    assert!(
        cells
            .iter()
            .map(|c| c.y as i32)
            .collect::<HashSet<_>>()
            .len()
            >= 2
    );
}

#[test]
fn inline_elements_stay_on_one_line() {
    // <b>/<span> are inline via the UA sheet, so the runs share one line (same y),
    // rather than stacking as separate blocks.
    let g = galley("<p>one <b>two</b> <span>three</span> four</p>");
    let ys: HashSet<i32> = all(&g)
        .into_iter()
        .filter(|f| matches!(f.content, FragmentContent::TextLine { .. }))
        .map(|f| f.y as i32)
        .collect();
    assert!(count_text(&g) >= 2); // multiple runs (regular + bold)
    assert_eq!(ys.len(), 1); // all on a single line
}

#[test]
fn block_elements_stack() {
    let g = galley("<div><p>first</p><p>second</p></div>");
    let texts: Vec<&Fragment> = all(&g)
        .into_iter()
        .filter(|f| matches!(f.content, FragmentContent::TextLine { .. }))
        .collect();
    assert_eq!(texts.len(), 2);
    assert!(texts[1].y > texts[0].y); // stacked vertically
}

#[test]
fn node_ids_round_trip_to_the_box_tree() {
    // Every galley fragment's NodeId is one assigned by box generation, so a
    // fragment maps back to its source node (AC-5.11).
    let template = "<div><p>hi</p><table><tr><td>x</td></tr></table></div>";
    let styled = render(template);
    let tree = build_box_tree(&styled);

    let mut box_ids = HashSet::new();
    collect_box_ids(&tree, &mut box_ids);

    let mut diags = Diagnostics::default();
    let g = layout(&styled, 600.0, &common::registry(), &mut diags);
    for frag in all(&g) {
        assert!(
            box_ids.contains(&frag.node_id),
            "fragment id {:?} not in box tree",
            frag.node_id
        );
    }
}

#[test]
fn layout_is_deterministic() {
    let a = galley("<p>determinism {{ name }}</p>");
    let b = galley("<p>determinism {{ name }}</p>");
    let ids_a: Vec<NodeId> = all(&a).iter().map(|f| f.node_id).collect();
    let ids_b: Vec<NodeId> = all(&b).iter().map(|f| f.node_id).collect();
    assert_eq!(ids_a, ids_b);
}

fn collect_box_ids(b: &turbo_html2pdf_core::layout::boxgen::LayoutBox, out: &mut HashSet<NodeId>) {
    use turbo_html2pdf_core::layout::boxgen::BoxKind;
    out.insert(b.node_id);
    match &b.kind {
        BoxKind::Block(kids) | BoxKind::Flex(kids) | BoxKind::Grid(kids) | BoxKind::Table(kids) => {
            for k in kids {
                collect_box_ids(k, out);
            }
        }
        BoxKind::Lines(items) => {
            for it in items {
                collect_inline_ids(it, out);
            }
        }
        BoxKind::Directive(_) => {}
    }
}

fn collect_inline_ids(
    it: &turbo_html2pdf_core::layout::boxgen::InlineItem,
    out: &mut HashSet<NodeId>,
) {
    use turbo_html2pdf_core::layout::boxgen::InlineItem;
    match it {
        InlineItem::Text { node_id, .. } | InlineItem::Directive { node_id, .. } => {
            out.insert(*node_id);
        }
        InlineItem::Atomic(b) => collect_box_ids(b, out),
    }
}
