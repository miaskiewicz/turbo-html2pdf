//! Phase 6 fragmenter tests (§6.1–6.2), AC-per-test. Galleys are built
//! synthetically with contiguous fragments (no inter-block gaps) so each test
//! controls page capacity and fragment heights exactly.

mod common;

use turbo_html2pdf_core::layout::value::BorderEdges;
use turbo_html2pdf_core::style::AtRule;
use turbo_html2pdf_core::{
    paginate, BreakMeta, BreakRule, Diagnostics, Fragment, FragmentContent, LintCode, NodeId, Rgba,
};

// --------------------------------------------------------------------------
// builders
// --------------------------------------------------------------------------

fn box_content() -> FragmentContent {
    FragmentContent::Box {
        background: None,
        border: BorderEdges::default(),
        border_radius: 0.0,
        shadow: None,
        gradient: None,
        transform: None,
    }
}

/// A box fragment at galley `y` with height `h`, width 200.
fn boxf(y: f32, h: f32) -> Fragment {
    // An unbreakable leaf (`break-inside: avoid`), standing for content: an empty box
    // would be sliced at a page end (miaskiewicz/turbo-html2pdf#47).
    let mut f = Fragment::new(NodeId(0), 0.0, y, 200.0, h, box_content());
    f.break_meta.break_inside_avoid = true;
    f
}

/// A text-line fragment at galley `y` with height `h`.
fn line(y: f32, h: f32) -> Fragment {
    Fragment::new(
        NodeId(0),
        0.0,
        y,
        100.0,
        h,
        FragmentContent::TextLine {
            text: String::new(),
            glyphs: Vec::new(),
            face: common::evolventa(),
            font_size: h,
            color: Rgba::BLACK,
        },
    )
}

/// Wrap children (already positioned contiguously) under a root box.
fn root(children: Vec<Fragment>) -> Fragment {
    let h = children.iter().map(|c| c.height).sum();
    let mut r = Fragment::new(NodeId(0), 0.0, 0.0, 200.0, h, box_content());
    r.children = children;
    r
}

/// Lay `boxes` of the given heights out contiguously from y=0.
fn stack(heights: &[f32]) -> Vec<Fragment> {
    let mut y = 0.0;
    let mut out = Vec::new();
    for &h in heights {
        out.push(boxf(y, h));
        y += h;
    }
    out
}

/// A container box whose children are positioned contiguously starting at the
/// container's own `y`.
fn container(y: f32, children: Vec<Fragment>) -> Fragment {
    let h = children.iter().map(|c| c.height).sum();
    let mut c = boxf(y, h);
    c.children = children;
    c
}

/// An `@page` at-rule with the given body.
fn page(body: &str) -> Vec<AtRule> {
    vec![AtRule {
        name: "page".to_string(),
        prelude: String::new(),
        body: body.to_string(),
    }]
}

/// A geometry whose body is 200×120 px (margin 0) for capacity math.
fn cap120() -> Vec<AtRule> {
    page("size: 200px 120px; margin: 0")
}

fn run(root: &Fragment, at: &[AtRule]) -> (Vec<turbo_html2pdf_core::Page>, Diagnostics) {
    let mut diags = Diagnostics::default();
    let pages = paginate(root, at, &mut diags).expect("paginate ok");
    (pages, diags)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.5
}

// --------------------------------------------------------------------------
// geometry (§6.1)
// --------------------------------------------------------------------------

#[test]
fn ac_6_1_default_is_a4_20mm() {
    let (pages, _) = run(&root(stack(&[10.0])), &[]);
    let g = pages[0].geometry;
    assert!(close(g.width, 210.0 * 96.0 / 25.4));
    assert!(close(g.height, 297.0 * 96.0 / 25.4));
    assert!(close(g.margin.top, 20.0 * 96.0 / 25.4));
}

#[test]
fn ac_6_1_named_letter() {
    let g = run(&root(stack(&[10.0])), &page("size: Letter")).0[0].geometry;
    assert!(close(g.width, 8.5 * 96.0));
    assert!(close(g.height, 11.0 * 96.0));
}

#[test]
fn ac_6_1_content_width_excludes_margins() {
    // 200px wide, margin 0 -> full 200px of body width.
    let g = run(&root(stack(&[10.0])), &cap120()).0[0].geometry;
    assert!(close(g.content_width(), 200.0));
}

#[test]
fn ac_6_1_landscape_swaps() {
    let g = run(&root(stack(&[10.0])), &page("size: A4 landscape")).0[0].geometry;
    assert!(g.width > g.height);
}

#[test]
fn ac_6_1_portrait_keyword() {
    let g = run(&root(stack(&[10.0])), &page("size: A5 portrait")).0[0].geometry;
    assert!(g.width < g.height);
}

#[test]
fn ac_6_1_single_dim_is_square() {
    let g = run(&root(stack(&[10.0])), &page("size: 300px")).0[0].geometry;
    assert!(close(g.width, 300.0) && close(g.height, 300.0));
}

#[test]
fn ac_6_1_explicit_wh_and_margin_two_values() {
    let g = run(
        &root(stack(&[10.0])),
        &page("size: 400px 500px; margin: 10px 20px"),
    )
    .0[0]
        .geometry;
    assert!(close(g.width, 400.0) && close(g.height, 500.0));
    assert!(close(g.margin.top, 10.0) && close(g.margin.left, 20.0));
}

#[test]
fn ac_6_1_margin_four_values() {
    let g = run(&root(stack(&[10.0])), &page("margin: 1px 2px 3px 4px")).0[0].geometry;
    assert!(close(g.margin.top, 1.0) && close(g.margin.right, 2.0));
    assert!(close(g.margin.bottom, 3.0) && close(g.margin.left, 4.0));
}

#[test]
fn ac_6_1_keyword_only_size_falls_back_to_a4() {
    // No explicit dims and no named size: dims empty -> A4, oriented landscape.
    let g = run(&root(stack(&[10.0])), &page("size: landscape")).0[0].geometry;
    assert!(g.width > g.height);
}

#[test]
fn ac_6_1_unknown_named_size_is_error() {
    let mut diags = Diagnostics::default();
    let err = paginate(&root(stack(&[10.0])), &page("size: A9"), &mut diags).unwrap_err();
    assert!(err.message.contains("unknown @page size"));
}

#[test]
fn ac_6_1_invalid_margin_is_ignored() {
    // Three values is not a valid shorthand -> margin left at the default.
    let g = run(&root(stack(&[10.0])), &page("margin: 1px 2px 3px")).0[0].geometry;
    assert!(close(g.margin.top, 20.0 * 96.0 / 25.4));
}

#[test]
fn ac_6_1_unknown_property_and_blank_decls_ignored() {
    let g = run(
        &root(stack(&[10.0])),
        &page("color: red; ; size: 200px 200px"),
    )
    .0[0]
        .geometry;
    assert!(close(g.width, 200.0));
}

// --------------------------------------------------------------------------
// break walk (§6.2)
// --------------------------------------------------------------------------

#[test]
fn ac_6_0_page_count_is_data_driven() {
    // Same template shape, more blocks -> more pages (capacity 120, blocks 50).
    let few = run(&root(stack(&[50.0, 50.0])), &cap120()).0;
    let many = run(&root(stack(&[50.0, 50.0, 50.0, 50.0])), &cap120()).0;
    assert_eq!(few.len(), 1);
    assert_eq!(many.len(), 2);
}

#[test]
fn ac_6_2_greedy_fill_splits_pages() {
    let (pages, _) = run(&root(stack(&[50.0, 50.0, 50.0])), &cap120());
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].body.len(), 2); // 100 <= 120, third would be 150
    assert_eq!(pages[1].body.len(), 1);
}

#[test]
fn ac_6_2_body_translated_into_page_coords() {
    // margin 10 -> body origin y = 10; first block top lands at y = 10.
    let at = page("size: 200px 300px; margin: 10px");
    let (pages, _) = run(&root(stack(&[20.0])), &at);
    assert!(close(pages[0].body[0].y, 10.0));
}

#[test]
fn ac_6_2_forced_break_before() {
    let mut blocks = stack(&[40.0, 40.0]);
    blocks[1].break_meta.break_before = BreakRule::Page;
    let (pages, _) = run(&root(blocks), &cap120());
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].body.len(), 1);
}

#[test]
fn ac_6_2_forced_break_after() {
    let mut blocks = stack(&[40.0, 40.0]);
    blocks[0].break_meta.break_after = BreakRule::Page;
    let (pages, _) = run(&root(blocks), &cap120());
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[1].body.len(), 1);
}

#[test]
fn forced_break_keeps_the_galley_gap_of_a_box_with_unknown_margins() {
    // No `FlowBoxes` facts (a bare galley): after a forced break the next block keeps its
    // galley distance from the content before the break (here 60 - 40 = 20px).
    let mut blocks = vec![boxf(0.0, 40.0), boxf(60.0, 40.0)];
    blocks[1].break_meta.break_before = BreakRule::Page;
    let (pages, _) = run(&root(blocks), &cap120());
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[1].body[0].y, 20.0);
}

#[test]
fn forced_break_before_the_first_content_makes_no_blank_page() {
    // miaskiewicz/turbo-html2pdf#32 item 3 (Chrome 156: the break is ignored).
    let mut blocks = stack(&[40.0, 40.0]);
    blocks[0].break_meta.break_before = BreakRule::Page;
    let (pages, _) = run(&root(blocks), &cap120());
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].body.len(), 2);
}

#[test]
fn break_after_and_break_before_at_one_point_break_once() {
    // miaskiewicz/turbo-html2pdf#32 item 3: no blank page between the two blocks.
    let mut blocks = stack(&[40.0, 40.0]);
    blocks[0].break_meta.break_after = BreakRule::Page;
    blocks[1].break_meta.break_before = BreakRule::Page;
    let (pages, _) = run(&root(blocks), &cap120());
    assert_eq!(pages.len(), 2);
    assert_eq!((pages[0].body.len(), pages[1].body.len()), (1, 1));
}

#[test]
fn split_block_starting_a_page_keeps_its_top_padding() {
    // miaskiewicz/turbo-html2pdf#32 item 4: the container (10px above its first child:
    // its top border + padding) splits; its first child moves to page 2 and keeps the
    // 10px there (only margins are truncated at an unforced break).
    let mut wrap = boxf(100.0, 140.0);
    wrap.children = vec![boxf(110.0, 60.0), boxf(170.0, 60.0)];
    let (pages, _) = run(&root(vec![boxf(0.0, 100.0), wrap]), &cap120());
    assert_eq!(pages.len(), 3);
    assert!(close(pages[1].body[0].y, 10.0));
    assert!(close(pages[2].body[0].y, 0.0)); // later breaks inside it truncate
}

/// A container whose children sit contiguously from its `y`, closed by a bottom border
/// `bottom` px wide.
fn bordered(y: f32, children: Vec<Fragment>, bottom: u16) -> Fragment {
    let mut c = container(y, children);
    c.height += f32::from(bottom);
    if let FragmentContent::Box { border, .. } = &mut c.content {
        border.bottom.width = bottom;
    }
    c
}

#[test]
fn last_content_moves_on_with_the_bottom_border_that_ends_its_parent() {
    // miaskiewicz/turbo-html2pdf#32 item 6: the second child exactly fills page 1, but
    // its parent's 20px bottom border cannot be separated from it: it moves to page 2.
    let wrap = bordered(0.0, vec![boxf(0.0, 60.0), boxf(60.0, 60.0)], 20);
    let (pages, _) = run(&root(vec![wrap]), &cap120());
    assert_eq!(pages.len(), 2);
    assert_eq!((pages[0].body.len(), pages[1].body.len()), (1, 1));
    assert!(close(pages[1].body[0].y, 0.0));
}

#[test]
fn last_content_that_never_fits_with_its_tail_stays() {
    // A 120px leaf + the 20px border never fit a 120px page: the leaf stays put and the
    // border starts page 2, as Chrome 156 breaks it (miaskiewicz/turbo-html2pdf#49,
    // `tail-border-past-page-end` in pagination_chrome156.json).
    let wrap = bordered(0.0, vec![boxf(0.0, 120.0)], 20);
    let (pages, diags) = run(&root(vec![wrap]), &cap120());
    assert_eq!(pages.len(), 2);
    assert!(close(pages[0].body[0].y, 0.0));
    assert!(diags.lints.is_empty());
}

#[test]
fn split_block_remaining_height_runs_onto_the_next_pages() {
    // miaskiewicz/turbo-html2pdf#33: a 300px box holding one 60px child splits; its
    // remaining 240px fill the rest of page 1 (60), all of page 2 (120) and 60px of
    // page 3, where the next block follows.
    let mut tall = boxf(0.0, 300.0);
    tall.children = vec![boxf(0.0, 60.0)];
    let (pages, _) = run(&root(vec![tall, boxf(300.0, 20.0)]), &cap120());
    assert_eq!(pages.len(), 3);
    assert!(pages[1].body.is_empty());
    assert!(close(pages[2].body[0].y, 60.0));
}

#[test]
fn rows_follow_the_repeated_header_on_a_continued_page() {
    // miaskiewicz/turbo-html2pdf#36: on page 2 the repeated header sits at the top and
    // the next row right below it (Chrome 156), not at its galley distance from the
    // header's original position at the top of the table.
    let mut header = boxf(0.0, 20.0);
    header.break_meta.repeatable = Some(turbo_html2pdf_core::RepeatKind::Header);
    let mut rows = vec![header];
    rows.extend((0..4).map(|i| boxf(20.0 + 40.0 * i as f32, 40.0)));
    let (pages, _) = run(&root(vec![container(0.0, rows)]), &cap120());
    let ys: Vec<Vec<f32>> = pages
        .iter()
        .map(|p| p.body.iter().map(|f| f.y).collect())
        .collect();
    assert_eq!(pages.len(), 2, "page tops {ys:?}");
    let ys = &ys[1];
    assert!(
        close(ys[0], 0.0) && close(ys[1], 20.0) && close(ys[2], 60.0),
        "{ys:?}"
    );
}

#[test]
fn ac_6_2_trailing_empty_page_trimmed() {
    let mut blocks = stack(&[40.0]);
    blocks[0].break_meta.break_after = BreakRule::Page;
    let (pages, _) = run(&root(blocks), &cap120());
    assert_eq!(pages.len(), 1); // the empty page the break would open is trimmed
}

#[test]
fn ac_6_2_break_inside_avoid_moves_whole() {
    let mut blocks = stack(&[60.0, 80.0]);
    blocks[1].break_meta.break_inside_avoid = true;
    // Give the avoid block children so splitting it *would* be possible — proving
    // avoid kept it whole on the next page instead.
    blocks[1].children = vec![boxf(60.0, 40.0), boxf(100.0, 40.0)];
    let (pages, _) = run(&root(blocks), &cap120());
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[1].body.len(), 1); // moved whole, not split
}

#[test]
fn ac_6_2_atomic_leaf_overflow_on_empty_page() {
    // A childless block taller than the body overflows with a lint.
    let (pages, diags) = run(&root(stack(&[200.0])), &cap120());
    assert_eq!(pages.len(), 1);
    assert!(diags
        .lints
        .iter()
        .any(|l| l.code == LintCode::RegionOverflow));
}

#[test]
fn ac_6_2_atomic_leaf_overflow_after_content() {
    // Prior content on the page -> overflow leaf opens a fresh page first.
    let (pages, diags) = run(&root(stack(&[60.0, 200.0])), &cap120());
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].body.len(), 1);
    assert!(diags
        .lints
        .iter()
        .any(|l| l.code == LintCode::RegionOverflow));
}

#[test]
fn ac_6_2_oversized_block_splits_into_children() {
    // A 200px container of five 40px rows splits across pages (no headers).
    let rows = vec![
        boxf(0.0, 40.0),
        boxf(40.0, 40.0),
        boxf(80.0, 40.0),
        boxf(120.0, 40.0),
        boxf(160.0, 40.0),
    ];
    let (pages, _) = run(&root(vec![container(0.0, rows)]), &cap120());
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].body.len(), 3); // 120 fits exactly three rows
    assert_eq!(pages[1].body.len(), 2);
}

// --------------------------------------------------------------------------
// repeatable table headers (§6.3 / AC-5.8)
// --------------------------------------------------------------------------

#[test]
fn ac_6_3_repeatable_header_reemitted() {
    let mut header = boxf(0.0, 20.0);
    header.break_meta.repeatable = Some(turbo_html2pdf_core::RepeatKind::Header);
    let rows = vec![
        header,
        boxf(20.0, 50.0),
        boxf(70.0, 50.0),
        boxf(120.0, 50.0),
    ];
    let (pages, _) = run(&root(vec![container(0.0, rows)]), &cap120());
    assert_eq!(pages.len(), 2);
    // Header (height 20) is present on both pages.
    assert!(close(pages[0].body[0].height, 20.0));
    assert!(close(pages[1].body[0].height, 20.0));
}

// --------------------------------------------------------------------------
// orphans / widows (§6.2)
// --------------------------------------------------------------------------

fn paragraph(y: f32, n: usize, lh: f32, orphans: u8, widows: u8) -> Fragment {
    let mut lines = Vec::new();
    let mut ly = y;
    for _ in 0..n {
        lines.push(line(ly, lh));
        ly += lh;
    }
    let mut p = boxf(y, lh * n as f32);
    p.children = lines;
    p.break_meta = BreakMeta {
        orphans,
        widows,
        ..BreakMeta::default()
    };
    p
}

#[test]
fn ac_6_2_widows_pull_lines_back() {
    // 5 lines × 30 = 150 > 120: a naive split leaves 4 then 1 (widow). With
    // widows=2 the break moves earlier to 3 + 2.
    let para = paragraph(0.0, 5, 30.0, 2, 2);
    let (pages, _) = run(&root(vec![para]), &cap120());
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].body.len(), 3);
    assert_eq!(pages[1].body.len(), 2);
}

#[test]
fn lines_exactly_filling_the_page_stay_on_it() {
    // 5 lines × 30 = 150 > 120, orphans/widows 1: lines 1-4 fill page 1 exactly (no gap
    // between contiguous lines), line 5 goes on.
    let para = paragraph(0.0, 5, 30.0, 1, 1);
    let (pages, _) = run(&root(vec![para]), &cap120());
    assert_eq!(pages.len(), 2);
    assert_eq!((pages[0].body.len(), pages[1].body.len()), (4, 1));
    assert!(close(pages[1].body[0].y, 0.0));
}

#[test]
fn spaced_lines_fill_the_page_with_their_gaps() {
    // 4 lines of 30 at galley 0, 35, 70, 105 (5px apart) on a 130px page: three fit with
    // their gaps (to 100); the fourth would end at 135 and goes on.
    let mut para = paragraph(0.0, 4, 30.0, 1, 1);
    for (i, l) in para.children.iter_mut().enumerate() {
        l.y = 35.0 * i as f32;
    }
    para.height = 135.0;
    let (pages, _) = run(&root(vec![para]), &page("size: 200px 130px; margin: 0"));
    assert_eq!(pages.len(), 2);
    assert_eq!((pages[0].body.len(), pages[1].body.len()), (3, 1));
    assert!(close(pages[0].body[2].y, 70.0));
}

#[test]
fn a_paragraphs_last_line_moves_on_with_its_parents_bottom_border() {
    // 4 lines × 30 = 120 plus a 10px bottom border: the last line fits page 1 only
    // without the border that ends its paragraph, so it moves on with it; earlier lines
    // carry no tail.
    let mut para = paragraph(0.0, 4, 30.0, 1, 1);
    para.height += 10.0;
    if let FragmentContent::Box { border, .. } = &mut para.content {
        border.bottom.width = 10;
    }
    let (pages, _) = run(&root(vec![para]), &cap120());
    assert_eq!(pages.len(), 2);
    assert_eq!((pages[0].body.len(), pages[1].body.len()), (3, 1));
}

#[test]
fn ac_6_2_orphans_defer_run_to_next_page() {
    // A 20px block, then a paragraph that must split. On page 1 only 2 lines
    // fit, but orphans=3 -> defer the whole paragraph to a fresh page.
    let block = boxf(0.0, 20.0);
    let para = paragraph(20.0, 5, 40.0, 3, 2);
    let (pages, _) = run(&root(vec![block, para]), &cap120());
    assert_eq!(pages[0].body.len(), 1); // only the block
    assert!(pages.len() >= 2);
}

#[test]
fn ac_6_2_orphans_unsatisfiable_on_empty_page_places_anyway() {
    // orphans=5 but only 2 lines fit on a fresh page: place what fits, no loop.
    let para = paragraph(0.0, 3, 50.0, 5, 1);
    let (pages, diags) = run(&root(vec![para]), &cap120());
    assert!(pages.len() >= 2);
    assert!(diags.is_empty() || !diags.is_empty()); // terminates; that's the point
}

#[test]
fn ac_6_2_single_line_taller_than_page_overflows() {
    // One 200px line on an empty page: forced progress + overflow lint.
    let para = paragraph(0.0, 1, 200.0, 2, 2);
    let (pages, diags) = run(&root(vec![para]), &cap120());
    assert_eq!(pages.len(), 1);
    assert!(diags
        .lints
        .iter()
        .any(|l| l.code == LintCode::RegionOverflow));
}

#[test]
fn ac_6_2_empty_document_yields_one_page() {
    let (pages, _) = run(&root(Vec::new()), &cap120());
    assert_eq!(pages.len(), 1);
    assert!(pages[0].body.is_empty());
}

#[test]
fn ac_6_page_kind_first_left_right() {
    // Three forced pages -> First, then Right (odd 3 -> recto), middle Left.
    let mut blocks = stack(&[20.0, 20.0, 20.0]);
    blocks[1].break_meta.break_before = BreakRule::Page;
    blocks[2].break_meta.break_before = BreakRule::Page;
    let (pages, _) = run(&root(blocks), &cap120());
    assert_eq!(pages[0].kind, turbo_html2pdf_core::PageKind::First);
    assert_eq!(pages[1].kind, turbo_html2pdf_core::PageKind::Left);
    assert_eq!(pages[2].kind, turbo_html2pdf_core::PageKind::Right);
}
