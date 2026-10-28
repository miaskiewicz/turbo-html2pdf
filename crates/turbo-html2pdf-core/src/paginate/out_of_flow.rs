//! Out-of-flow boxes in print (miaskiewicz/turbo-html2pdf#32 item 1), as Chrome 156
//! paginates them. Layout places `absolute`/`fixed` boxes in the continuous galley; the
//! fragmenter must not stack them as flow content:
//!
//! - A `fixed` box repeats on every page, at its galley position relative to the page
//!   area (the print initial containing block); with only `bottom` set it is anchored to
//!   the page area's bottom.
//! - An `absolute` box with a vertical inset (`top`/`bottom`) sits at its offset from its
//!   containing block — the nearest positioned ancestor, else the initial containing
//!   block (the first page area) — measured from where that block starts and continued
//!   through the page capacities (a box past the content lands on new pages).
//! - An `absolute` box with neither inset stays in the flow at its static position (the
//!   walker places it there without advancing).
//!
//! [`extract`] takes the fixed boxes and the inset absolute boxes of the initial
//! containing block out of the galley before the walk. An absolute box whose containing
//! block is a positioned box stays its descendant: placed whole, the containing block
//! carries it (its paint and tagged structure with it); split, the walker defers it from
//! that block's start ([`Anchors`], `Walker::place_out_of_flow`). [`place`] adds the taken
//! and deferred boxes to the pages after the walk, after each page's flow.

use std::collections::HashMap;

use crate::layout::fragment::{Fragment, NodeId};
use crate::layout::value::Position;

use super::boxes::{BoxFacts, FlowBoxes};
use super::walk::PageSpan;
use super::FootnoteBand;

/// Where a containing block starts: its 0-based page and its border-box top there.
pub(super) type Anchors = HashMap<NodeId, (usize, f32)>;

/// An out-of-flow box placed apart from the flow, with its offset from its containing
/// block's top (`cb` = `None`: the initial containing block / the page area).
#[derive(Debug, Clone)]
pub(super) struct Taken {
    pub frag: Fragment,
    pub cb: Option<NodeId>,
    pub dy: f32,
}

/// The galley without its fixed boxes and the inset absolute boxes of the initial
/// containing block, and those boxes.
#[derive(Debug, Clone)]
pub(super) struct Extracted {
    pub root: Fragment,
    fixed: Vec<Taken>,
    absolute: Vec<Taken>,
}

/// Take the fixed boxes and the absolute boxes with a vertical inset out of `root`
/// (`None`: it has none, the galley is walked as is). `area` is the page area height
/// (the fragmenter's base body capacity) a `bottom` inset anchors to.
pub(super) fn extract(root: &Fragment, boxes: &FlowBoxes, area: f32) -> Option<Extracted> {
    if !has_taken(root, boxes, false) {
        return None;
    }
    let mut root = root.clone();
    let mut taken = Vec::new();
    take(&mut root, false, boxes, &mut taken);
    let (fixed, absolute): (Vec<_>, Vec<_>) = taken
        .into_iter()
        .map(|(t, facts)| (anchor_bottom(t, &facts, area), facts.position))
        .partition(|(_, position)| *position == Position::Fixed);
    Some(Extracted {
        root,
        fixed: fixed.into_iter().map(|(t, _)| t).collect(),
        absolute: absolute.into_iter().map(|(t, _)| t).collect(),
    })
}

/// The facts of `f`'s box when it is taken out of the galley (`positioned`: it has a
/// positioned ancestor, its containing block).
fn taken_facts(f: &Fragment, boxes: &FlowBoxes, positioned: bool) -> Option<BoxFacts> {
    boxes.get(f.node_id).copied().filter(|b| {
        b.position == Position::Fixed
            || (b.position == Position::Absolute && b.anchored() && !positioned)
    })
}

fn has_taken(f: &Fragment, boxes: &FlowBoxes, positioned: bool) -> bool {
    f.children.iter().any(|c| {
        taken_facts(c, boxes, positioned).is_some()
            || has_taken(c, boxes, positioned || c.is_positioned)
    })
}

/// Move every taken descendant of `f` into `out` (with its facts); `positioned`: `f` is
/// (inside) a positioned box, the containing block of its absolute descendants. A taken
/// box's offset is from the galley top (the page area / the initial containing block).
fn take(f: &mut Fragment, positioned: bool, boxes: &FlowBoxes, out: &mut Vec<(Taken, BoxFacts)>) {
    let children = std::mem::take(&mut f.children);
    for mut child in children {
        if let Some(facts) = taken_facts(&child, boxes, positioned) {
            let dy = child.y;
            out.push((
                Taken {
                    frag: child,
                    cb: None,
                    dy,
                },
                facts,
            ));
            continue;
        }
        let inner = positioned || child.is_positioned;
        take(&mut child, inner, boxes, out);
        f.children.push(child);
    }
}

/// Anchor a box of the page area / initial containing block with only `bottom` set to
/// the area's bottom (the galley has no page height to resolve it against).
fn anchor_bottom(mut t: Taken, facts: &BoxFacts, area: f32) -> Taken {
    let insets = (
        facts.inset_top.resolve(area),
        facts.inset_bottom.resolve(area),
    );
    if let (None, Some(bottom)) = insets {
        t.dy = area - bottom - facts.margin_bottom - t.frag.height;
    }
    t
}

/// The walked pages a [`place`] adds the taken boxes to (aligned vectors).
pub(super) struct Paged<'a> {
    pub bodies: &'a mut Vec<Vec<Fragment>>,
    pub spans: &'a mut Vec<PageSpan>,
    pub bands: &'a mut Vec<FootnoteBand>,
    /// The base page capacity of pages added past the walked ones.
    pub capacity: f32,
}

/// Add the taken boxes and the walker's `deferred` ones to the pages: each absolute box at
/// its offset from its containing block's start, continued through the page capacities
/// (adding pages), then every fixed box on every page.
pub(super) fn place(
    extracted: Option<&Extracted>,
    deferred: &[Taken],
    anchors: &Anchors,
    pages: Paged,
) {
    let taken = extracted.map_or(&[][..], |e| &e.absolute);
    for t in taken.iter().chain(deferred) {
        let start = t.cb.and_then(|id| anchors.get(&id)).copied();
        let (page, y) = start.unwrap_or((0, 0.0));
        put(
            t,
            (page, y + t.dy),
            &mut *pages.bodies,
            pages.spans,
            pages.bands,
            pages.capacity,
        );
    }
    let fixed = extracted.map_or(&[][..], |e| &e.fixed);
    for body in pages.bodies.iter_mut() {
        for t in fixed {
            body.push(moved(&t.frag, t.dy));
        }
    }
}

/// Put `t` at page-local `y` of `page`, moving on a page for every page capacity it runs
/// past.
fn put(
    t: &Taken,
    (mut page, mut y): (usize, f32),
    bodies: &mut Vec<Vec<Fragment>>,
    spans: &mut Vec<PageSpan>,
    bands: &mut Vec<FootnoteBand>,
    capacity: f32,
) {
    let cap = |spans: &[PageSpan], page: usize| spans.get(page).map_or(capacity, |s| s.capacity);
    while y >= cap(spans, page) && cap(spans, page) > 0.0 {
        y -= cap(spans, page);
        page += 1;
    }
    while bodies.len() <= page {
        bodies.push(Vec::new());
        spans.push(PageSpan {
            capacity,
            last: None,
        });
        bands.push(FootnoteBand::default());
    }
    bodies[page].push(moved(&t.frag, y));
}

/// A copy of `f` whose top is at `y`.
fn moved(f: &Fragment, y: f32) -> Fragment {
    let mut clone = f.clone();
    clone.translate(0.0, y - f.y);
    clone
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::fragment::FragmentContent;
    use crate::layout::value::{BorderEdges, LengthPct};

    fn boxed(id: u32, y: f32, children: Vec<Fragment>) -> Fragment {
        let content = FragmentContent::Box {
            background: None,
            border: BorderEdges::default(),
            border_radius: 0.0,
            shadow: None,
            gradient: None,
            transform: None,
        };
        let mut f = Fragment::new(NodeId(id), 0.0, y, 100.0, 50.0, content);
        f.children = children;
        f
    }

    fn facts(position: Position, inset_top: LengthPct) -> BoxFacts {
        BoxFacts {
            position,
            margin_top: 0.0,
            margin_bottom: 0.0,
            padding_top: 0.0,
            padding_bottom: 0.0,
            inset_top,
            inset_bottom: LengthPct::Auto,
            shift: 0.0,
            definite_height: false,
            avoid_break_inside: false,
        }
    }

    #[test]
    fn a_galley_without_taken_boxes_is_walked_as_is() {
        let mut boxes = FlowBoxes::default();
        // A static-position absolute box stays in the flow.
        boxes.insert(NodeId(3), facts(Position::Absolute, LengthPct::Auto));
        let root = boxed(1, 0.0, vec![boxed(2, 0.0, vec![boxed(3, 10.0, vec![])])]);
        assert!(extract(&root, &boxes, 300.0).is_none());
    }

    #[test]
    fn a_nested_fixed_box_is_taken_out_of_the_galley() {
        let mut boxes = FlowBoxes::default();
        boxes.insert(NodeId(3), facts(Position::Fixed, LengthPct::Px(5.0)));
        let root = boxed(1, 0.0, vec![boxed(2, 0.0, vec![boxed(3, 40.0, vec![])])]);
        let extracted = extract(&root, &boxes, 300.0).expect("a fixed box to take");
        assert!(extracted.root.children[0].children.is_empty());
        assert_eq!(extracted.fixed.len(), 1);
        assert_eq!(extracted.fixed[0].frag.node_id, NodeId(3));
        assert_eq!(extracted.fixed[0].dy, 40.0);
        assert!(extracted.absolute.is_empty());
    }

    #[test]
    fn an_inset_absolute_box_in_a_positioned_box_stays_in_the_galley() {
        let mut boxes = FlowBoxes::default();
        boxes.insert(NodeId(2), facts(Position::Fixed, LengthPct::Px(0.0)));
        boxes.insert(NodeId(4), facts(Position::Absolute, LengthPct::Px(5.0)));
        boxes.insert(NodeId(5), facts(Position::Absolute, LengthPct::Px(5.0)));
        let mut relative = boxed(3, 60.0, vec![boxed(4, 65.0, vec![])]);
        relative.is_positioned = true;
        let root = boxed(
            1,
            0.0,
            vec![boxed(2, 0.0, vec![]), relative, boxed(5, 120.0, vec![])],
        );
        let extracted = extract(&root, &boxes, 300.0).expect("boxes to take");
        let kept: Vec<NodeId> = extracted.root.children.iter().map(|c| c.node_id).collect();
        assert_eq!(kept, [NodeId(3)]);
        assert_eq!(extracted.root.children[0].children[0].node_id, NodeId(4));
        let taken: Vec<NodeId> = extracted.absolute.iter().map(|t| t.frag.node_id).collect();
        assert_eq!(taken, [NodeId(5)], "only the initial containing block's");
    }

    #[test]
    fn a_bottom_anchored_box_ends_its_margin_box_at_the_inset() {
        let mut boxes = FlowBoxes::default();
        let bottom = BoxFacts {
            inset_bottom: LengthPct::Px(10.0),
            margin_bottom: 5.0,
            ..facts(Position::Fixed, LengthPct::Auto)
        };
        boxes.insert(NodeId(2), bottom);
        let root = boxed(1, 0.0, vec![boxed(2, 0.0, vec![])]);
        let extracted = extract(&root, &boxes, 300.0).expect("a fixed box to take");
        // 300 area - 10 bottom - 5 margin - 50 height.
        assert_eq!(extracted.fixed[0].dy, 235.0);
    }

    #[test]
    fn a_box_runs_on_through_page_capacities_but_stays_on_a_zero_capacity_page() {
        let t = |dy: f32| Taken {
            frag: boxed(9, 0.0, vec![]),
            cb: None,
            dy,
        };
        let run = |spans: Vec<PageSpan>, dy: f32| {
            let mut bodies = vec![Vec::new(); spans.len()];
            let mut bands = vec![FootnoteBand::default(); spans.len()];
            let mut spans = spans;
            put(&t(dy), (0, dy), &mut bodies, &mut spans, &mut bands, 300.0);
            let at: Vec<(usize, f32)> = bodies
                .iter()
                .enumerate()
                .flat_map(|(p, b)| b.iter().map(move |f| (p, f.y)))
                .collect();
            (at, spans.len(), bands.len())
        };
        let span = |capacity: f32| PageSpan {
            capacity,
            last: None,
        };
        // 250px past a 200px page: 50px down the next, an added page of base capacity.
        assert_eq!(run(vec![span(200.0)], 250.0), (vec![(1, 50.0)], 2, 2));
        // Exactly at a page's end: the top of the next.
        assert_eq!(run(vec![span(200.0)], 200.0), (vec![(1, 0.0)], 2, 2));
        // A page with no capacity cannot be run through: the box stays on it.
        assert_eq!(run(vec![span(0.0)], 10.0), (vec![(0, 10.0)], 1, 1));
    }

    #[test]
    fn a_deep_inset_absolute_box_of_a_positioned_box_takes_nothing_out() {
        let mut boxes = FlowBoxes::default();
        boxes.insert(NodeId(4), facts(Position::Absolute, LengthPct::Px(5.0)));
        let mut relative = boxed(2, 0.0, vec![boxed(3, 0.0, vec![boxed(4, 5.0, vec![])])]);
        relative.is_positioned = true;
        let root = boxed(1, 0.0, vec![relative]);
        assert!(extract(&root, &boxes, 300.0).is_none());
    }
}
