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
//! [`extract`] takes the first two kinds out of the galley before the walk; the walk
//! records where each containing block they need starts ([`Anchors`]); [`place`] adds
//! them to the pages afterwards.

use std::collections::{HashMap, HashSet};

use crate::layout::fragment::{Fragment, NodeId};
use crate::layout::value::{LengthPct, Position};

use super::boxes::{BoxFacts, FlowBoxes};
use super::walk::PageSpan;
use super::FootnoteBand;

/// Where a containing block starts: its 0-based page and its border-box top there.
pub(super) type Anchors = HashMap<NodeId, (usize, f32)>;

/// An out-of-flow box taken out of the galley, with its offset from its containing
/// block's top (`cb` = `None`: the initial containing block / the page area).
#[derive(Debug, Clone)]
struct Taken {
    frag: Fragment,
    cb: Option<NodeId>,
    dy: f32,
}

/// The galley without its fixed and inset-anchored absolute boxes, and those boxes.
#[derive(Debug, Clone)]
pub(super) struct Extracted {
    pub root: Fragment,
    fixed: Vec<Taken>,
    absolute: Vec<Taken>,
}

impl Extracted {
    /// The containing blocks whose start the walk must record.
    pub fn needed(&self) -> HashSet<NodeId> {
        self.absolute.iter().filter_map(|t| t.cb).collect()
    }
}

/// Take the fixed boxes and the absolute boxes with a vertical inset out of `root`
/// (`None`: it has none, the galley is walked as is). `area` is the page area height
/// (the fragmenter's base body capacity) a `bottom` inset anchors to.
pub(super) fn extract(root: &Fragment, boxes: &FlowBoxes, area: f32) -> Option<Extracted> {
    if !has_taken(root, boxes) {
        return None;
    }
    let mut root = root.clone();
    let mut taken = Vec::new();
    let top = root.y;
    take(&mut root, (None, top), boxes, &mut taken);
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

/// The facts of `f`'s box when it is taken out of the galley.
fn taken_facts(f: &Fragment, boxes: &FlowBoxes) -> Option<BoxFacts> {
    boxes.get(f.node_id).copied().filter(|b| {
        b.position == Position::Fixed
            || (b.position == Position::Absolute
                && (b.inset_top != LengthPct::Auto || b.inset_bottom != LengthPct::Auto))
    })
}

fn has_taken(f: &Fragment, boxes: &FlowBoxes) -> bool {
    f.children
        .iter()
        .any(|c| taken_facts(c, boxes).is_some() || has_taken(c, boxes))
}

/// Move every taken descendant of `f` into `out` (with its facts). `cb` is the
/// containing block of `f`'s absolute descendants (`(id, galley top)`, `id` = `None`:
/// the initial containing block, at the galley top).
fn take(
    f: &mut Fragment,
    cb: (Option<NodeId>, f32),
    boxes: &FlowBoxes,
    out: &mut Vec<(Taken, BoxFacts)>,
) {
    let children = std::mem::take(&mut f.children);
    for mut child in children {
        if let Some(facts) = taken_facts(&child, boxes) {
            out.push((taken(child, &facts, cb), facts));
            continue;
        }
        let inner = if child.is_positioned {
            (Some(child.node_id), child.y)
        } else {
            cb
        };
        take(&mut child, inner, boxes, out);
        f.children.push(child);
    }
}

/// A taken box with its offset from its containing block (a fixed box's is the page
/// area, at the galley top).
fn taken(frag: Fragment, facts: &BoxFacts, cb: (Option<NodeId>, f32)) -> Taken {
    let (cb, origin) = if facts.position == Position::Fixed {
        (None, 0.0)
    } else {
        cb
    };
    let dy = frag.y - origin;
    Taken { frag, cb, dy }
}

/// Anchor a box of the page area / initial containing block with only `bottom` set to
/// the area's bottom (the galley has no page height to resolve it against).
fn anchor_bottom(mut t: Taken, facts: &BoxFacts, area: f32) -> Taken {
    let insets = (
        facts.inset_top.resolve(area),
        facts.inset_bottom.resolve(area),
    );
    if let (None, Some(bottom), None) = (insets.0, insets.1, t.cb) {
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

/// Add the taken boxes to the pages: each absolute box at its offset from its
/// containing block's start, continued through the page capacities (adding pages), then
/// every fixed box on every page.
pub(super) fn place(extracted: &Extracted, anchors: &Anchors, pages: Paged) {
    for t in &extracted.absolute {
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
    for body in pages.bodies.iter_mut() {
        for t in &extracted.fixed {
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
