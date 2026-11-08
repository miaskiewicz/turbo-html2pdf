//! Multi-column pagination (miaskiewicz/turbo-html2pdf#52): re-flow a
//! `turbo-dom-layout::multicol` container's children across pages of the
//! paginator's body capacity.
//!
//! turbo-dom-layout 0.8.1's [`multicol`](../../../../../turbo-dom-wt-081/crates/turbo-dom-layout/src/layout/multicol.rs)
//! lays the container out ONCE at the galley level (balanced per CSS Multicol
//! 1 §7.3): its in-flow children are positioned in source order at
//! `(col_index * (used_width + gap), y_within_col)`. The paginator receives
//! that balanced tree; this module replaces its per-column placement with a
//! page-aware flow — fill the current page's columns top-to-bottom,
//! left-to-right, then start a new page when the columns exhaust — so a
//! `<div style="columns:2">` whose balance height exceeds the page body
//! produces multiple pages, each with up to `used_count` columns of content.
//!
//! The flow currently implements `column-fill: auto` on every page, including
//! the last (CSS Multicol 1 §3 "The multi-column model") — Chrome 156
//! paginates multicol this way for the pages that saturate the body, while
//! `column-fill: balance` on the last page balances content to the tallest
//! filled column. This paginator does NOT re-balance the last page; the only
//! balancing it applies is clipping the column *rule* to the tallest filled
//! column so the rule shrinks on a short last page, matching the raster
//! painter. A child with `break-inside: avoid` that would overrun the current
//! column is pushed to a fresh one (CSS Fragmentation 3 §5.2). A child whose
//! height exceeds the page body's capacity is flagged with
//! [`LintCode::RegionOverflow`] and placed anyway (matching the generic
//! walker's `overflow_leaf` behaviour); pre-layout column-rule fragments are
//! dropped here (their geometry belongs to the one-shot balanced tree, not
//! the paginated one) and the paginator re-emits them per page from the
//! container's `MulticolFacts`.
//!
//! An empty multicol container (no in-flow children, only dropped column-rule
//! fragments) still paints its own box: [`Flow::single_empty_page`] emits one
//! page whose fill height is 0 so the generic walker advances past the
//! container's own box.
//!
//! The entry [`place_multicol`] drives this walk through the shared
//! [`Walker`]/[`PageOps`] interface so emit/new-page/advance semantics stay in
//! lockstep with the rest of pagination (margins at page starts, footnote
//! reservations, repeated headers, out-of-flow placement around the
//! container).
//!
//! **Known limitations** (R1 review, deferred follow-ups):
//! * `break-before: column` / `break-after: column` / `break-inside:
//!   avoid-column` / `column-span: all` are honoured by turbo-dom-layout's
//!   balancer but not re-honoured here when re-flowing across pages — the
//!   paginator reads only the child's `height`. Separating these from
//!   `break-{before,after}: page` would require either a new
//!   `BreakRule::Column` variant in turbo-dom-layout (a turbo-dom change) or
//!   per-child `ComputedStyle` lookup in `flow_boxes.rs`.
//! * `position: absolute` / `position: fixed` descendants inside a multicol
//!   container are placed as in-flow blocks (the `flow_boxes.rs::visit` early
//!   return skips their `BoxFacts`).
//! * Nested multicol: the inner container flows as a plain block instead of
//!   re-entering `place_multicol`.

use crate::layout::fragment::{ColumnRuleStyle, Fragment, FragmentContent, NodeId};
use crate::layout::value::Rgba;

use super::boxes::MulticolFacts;

/// What to paint for a multicol container on one paginated page: the per-page
/// box (its own border-box, background and border), the children placed in
/// their final galley coordinates for THAT page, and any column-rule fragments
/// painted between adjacent columns.
///
/// The paginator emits the box first (so background and border paint under the
/// children), then every child, then the rules. Emission uses the walker's
/// own `put`/`put_on` so the usual page-translation, anchoring and layering
/// (`is_positioned`, `z-index`) stay identical to the rest of the walk; this
/// module never writes directly into the `pages` Vec.
pub struct PagePaint {
    pub container: Fragment,
    pub children: Vec<Fragment>,
    pub rules: Vec<Fragment>,
}

/// A per-child placement choice: which column strip on which page, and the
/// column-local y at which the child sits. Flows in source order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    pub page: u32,
    pub col: u32,
    pub y_in_col: f32,
    pub height: f32,
}

/// The output of flowing a container's children across pages: one [`Slot`]
/// per in-flow child (empty for a child skipped by the caller — a column-rule
/// fragment the layout emitted, which the paginator re-emits per page) plus
/// the per-page column heights used for column-rule re-emission (CSS Multicol
/// 1 §3 "The multi-column model"). Chrome 156 fills every page BUT the last
/// to its full column height (so the rule runs to the page end on a page
/// whose columns saturate) and balances the last page's columns to their
/// filled content (the rule shrinks to the tallest filled column), which
/// `rule_heights` encodes alongside the content heights `fill_heights`.
#[derive(Debug, Clone, Default)]
pub struct Flow {
    pub slots: Vec<Option<Slot>>,
    /// Content height per page: the tallest filled column on that page.
    pub fill_heights: Vec<f32>,
    /// Rule height per page: `full_cap` (or `first_cap` on page 0) on every
    /// non-last page, the `fill_heights` value on the last page.
    pub rule_heights: Vec<f32>,
}

/// Classify a child by whether the paginator should flow it. A container's
/// pre-layout `FragmentContent::ColumnRule` is dropped because the paginator
/// re-emits rules per page ([`column_rule_fragments`]); every other child is
/// an in-flow block whose fragment feeds into the per-page column fill.
pub fn is_flowable(child: &Fragment) -> bool {
    !matches!(child.content, FragmentContent::ColumnRule { .. })
}

/// Flow `children` of a multicol container into pages, filling each page's
/// `used_count` columns top-to-bottom, left-to-right (CSS `column-fill:
/// auto`). `first_cap` is the body height left on the current page (the page
/// the multicol starts on; may be less than a full page when prior content
/// sits above it); `full_cap` is the body height of a fresh page. Returns a
/// per-child [`Slot`] (None for a child dropped by [`is_flowable`]) and the
/// filled column height of each page. An empty container (no in-flow
/// children, only dropped column rules) produces one page with `fill_h = 0`
/// so the walker can still emit the container's own box and advance past it.
#[must_use]
pub fn flow_children(children: &[Fragment], facts: MulticolFacts, caps: (f32, f32)) -> Flow {
    let mut state = FlowState::start(children.len(), caps);
    for child in children {
        state.step(child, facts);
    }
    state.finish()
}

/// Mutable walker state for [`flow_children`]: tracks the current page's
/// column cursor and the per-page records the final `Flow` reads from.
/// Split out so the step helpers stay under the complexity gate.
struct FlowState {
    slots: Vec<Option<Slot>>,
    fill_heights: Vec<f32>,
    page_caps: Vec<f32>,
    full_cap: f32,
    cap: f32,
    page: u32,
    col: u32,
    y_in_col: f32,
    page_h: f32,
}

impl FlowState {
    fn start(count: usize, caps: (f32, f32)) -> FlowState {
        let (first_cap, full_cap) = caps;
        FlowState {
            slots: Vec::with_capacity(count),
            fill_heights: Vec::new(),
            page_caps: Vec::new(),
            full_cap,
            cap: first_cap,
            page: 0,
            col: 0,
            y_in_col: 0.0,
            page_h: 0.0,
        }
    }

    fn step(&mut self, child: &Fragment, facts: MulticolFacts) {
        if !is_flowable(child) {
            self.slots.push(None);
            return;
        }
        let h = child.height.max(0.0);
        self.maybe_wrap(h, facts);
        self.slots.push(Some(Slot {
            page: self.page,
            col: self.col,
            y_in_col: self.y_in_col,
            height: h,
        }));
        self.y_in_col += h;
        self.page_h = self.page_h.max(self.y_in_col);
    }

    /// Advance the cursor before placing a `h`-tall child: a child that
    /// overruns the current column pushes to the next (CSS Fragmentation 3
    /// §5.2 `break-inside: avoid`), and a run that saturates `used_count`
    /// columns opens a fresh page.
    fn maybe_wrap(&mut self, h: f32, facts: MulticolFacts) {
        if self.y_in_col <= 0.0 || self.y_in_col + h <= self.cap + FIT_EPS {
            return;
        }
        self.col += 1;
        self.y_in_col = 0.0;
        if self.col < facts.used_count {
            return;
        }
        self.fill_heights.push(self.page_h);
        self.page_caps.push(self.cap);
        self.page += 1;
        self.col = 0;
        self.page_h = 0.0;
        self.cap = self.full_cap;
    }

    fn finish(mut self) -> Flow {
        // A multicol with ONLY non-flowable children (e.g. `<div
        // style="columns:2"></div>` with no in-flow kids, or a container whose
        // only emitted child was a `ColumnRule` fragment the paginator drops)
        // still owns a container box to paint. Emit one zero-height page so
        // `place_multicol` has somewhere to put the container's own frame and
        // advance the cursor past it (MEDIUM-1).
        let any_placed = self.slots.iter().any(Option::is_some);
        if any_placed || self.slots.is_empty() {
            self.fill_heights.push(self.page_h);
            self.page_caps.push(self.cap);
        } else {
            // All slots are `None` (every child was a dropped column rule) —
            // still a container-paint opportunity.
            self.fill_heights.push(0.0);
            self.page_caps.push(self.cap);
        }
        debug_assert_eq!(
            self.fill_heights.len(),
            self.page_caps.len(),
            "fill_heights / page_caps lengths diverge; `rule_heights_from` zip would mis-pair pages",
        );
        let rule_heights = rule_heights_from(&self.fill_heights, &self.page_caps);
        Flow {
            slots: self.slots,
            fill_heights: self.fill_heights,
            rule_heights,
        }
    }
}

/// Per-page rule height (CSS Multicol 1 §3, Chrome 156): every page but the
/// last fills its column strips to the page body height, so the rule runs
/// full-height — `page_caps[page]`. The last page balances to the tallest
/// filled column (`fill_heights[last]`), so the rule shrinks to that content.
fn rule_heights_from(fills: &[f32], caps: &[f32]) -> Vec<f32> {
    let last = fills.len().saturating_sub(1);
    fills
        .iter()
        .zip(caps)
        .enumerate()
        .map(|(i, (fill, cap))| if i == last { *fill } else { *cap })
        .collect()
}

/// Chrome's 1/64-px LayoutUnit slop — a column height that differs by at
/// most this from the capacity is treated as a WHOLE column, so a 1/64 px
/// rounding slop at balance does not spawn a zero-height tail piece.
const FIT_EPS: f32 = 0.5;

/// A multicol container's `column-rule-*` style resolved from the
/// pre-layout fragments the layout emitted: the painter re-emits one rule
/// per adjacent column pair per page using this style and the page's
/// filled column height ([`Flow::col_heights`]). The layout emits a
/// `FragmentContent::ColumnRule` for each gap, so the first one (if any)
/// carries the shared style.
pub fn rule_style(children: &[Fragment]) -> Option<(ColumnRuleStyle, Rgba, f32)> {
    children.iter().find_map(|c| match c.content {
        FragmentContent::ColumnRule { style, color } => Some((style, color, c.width)),
        _ => None,
    })
}

/// Build one page's paint plan for a paginated multicol container: the
/// per-page box (whose height is this page's filled column height `col_h`),
/// the children whose [`Slot`] lands on `page`, laid out at their column
/// positions in galley coordinates (the container's y + `y_in_col`), and the
/// column-rule fragments between adjacent columns. The caller places each
/// one through the walker's `put`/`put_on` so page translation stays shared
/// with the rest of the walk.
#[must_use]
pub fn page_paint(
    container: &Fragment,
    children: &[Fragment],
    facts: MulticolFacts,
    flow: &Flow,
    page: u32,
    heights: (f32, f32),
    content_origin: (f32, f32),
) -> PagePaint {
    let (fill_h, rule_h) = heights;
    let (cx, cy) = content_origin;
    let mut per_page = container.clone();
    per_page.height = fill_h;
    per_page.children.clear();
    let stride = facts.used_width + facts.gap;
    let mut kids: Vec<Fragment> = Vec::new();
    for (i, slot) in flow.slots.iter().enumerate() {
        let Some(s) = slot else { continue };
        if s.page != page {
            continue;
        }
        let mut child = children[i].clone();
        // Replace the layout's balanced position with this page's slot.
        let child_x = cx + s.col as f32 * stride;
        let child_y = cy + s.y_in_col;
        let (dx, dy) = (child_x - child.x, child_y - child.y);
        child.translate(dx, dy);
        kids.push(child);
    }
    let rule = rule_style(children);
    let rules = rule.map_or_else(Vec::new, |r| {
        column_rule_fragments(container.node_id, (cx, cy), facts, r, rule_h)
    });
    PagePaint {
        container: per_page,
        children: kids,
        rules,
    }
}

/// Build per-page column-rule fragments ([`FragmentContent::ColumnRule`])
/// for one paginated multicol segment: rule `k` sits centered in the gap
/// after column `k` at the page's column origin `(cx, cy)`, is `rule_w`
/// px wide (`rule_w <= gap`, matching Chrome 156's Blink clamp) and as
/// tall as the page's filled column height `col_h`.
#[must_use]
pub fn column_rule_fragments(
    node_id: NodeId,
    origin: (f32, f32),
    facts: MulticolFacts,
    rule: (ColumnRuleStyle, Rgba, f32),
    col_h: f32,
) -> Vec<Fragment> {
    if !rule_visible(facts, rule, col_h) {
        return Vec::new();
    }
    let (style, color, rule_w) = rule;
    let clamped = rule_w.min(facts.gap);
    (0..(facts.used_count - 1))
        .map(|k| one_rule(node_id, origin, facts, (style, color, clamped), col_h, k))
        .collect()
}

/// Whether the multicol container paints a column rule under `facts` with `rule`
/// at `col_h`: non-zero track count beyond 1, positive gap, a non-zero rule
/// width and a non-transparent colour (CSS Multicol 1 §5 "The column-rule
/// properties"). Used by [`column_rule_fragments`] to short-circuit.
fn rule_visible(facts: MulticolFacts, rule: (ColumnRuleStyle, Rgba, f32), col_h: f32) -> bool {
    let (_, color, rule_w) = rule;
    facts.used_count > 1 && col_h > 0.0 && facts.gap > 0.0 && rule_w > 0.0 && color.a > 0
}

/// Build one column rule's fragment: `k` is the gap index (`0..used_count-1`),
/// the rule is centered in the `k`-th gap at `(origin.x + (k+1) * used_width +
/// (k+0.5) * gap, origin.y)`, `clamped.width` wide (clamped to the gap by the
/// caller) and `col_h` tall. `clamped <= facts.gap` holds by construction
/// ([`column_rule_fragments`] clamps `rule_w` to the gap), so the fragment
/// never crosses the far edge of the last gap and no outer fence is needed.
fn one_rule(
    node_id: NodeId,
    (cx, cy): (f32, f32),
    facts: MulticolFacts,
    rule: (ColumnRuleStyle, Rgba, f32),
    col_h: f32,
    k: u32,
) -> Fragment {
    let (style, color, clamped) = rule;
    let k_f = k as f32;
    let gap_center = cx + (k_f + 1.0) * facts.used_width + (k_f + 0.5) * facts.gap;
    Fragment::new(
        node_id,
        gap_center - clamped / 2.0,
        cy,
        clamped,
        col_h,
        FragmentContent::ColumnRule { style, color },
    )
}

/// The galley node ids of the multicol children that would overflow the given
/// page capacity if placed as the first child in a fresh column (their height
/// alone exceeds `full_cap`). The caller lints each one as
/// [`crate::error::LintCode::RegionOverflow`]: the paginator still places them
/// (matching the generic walker's `overflow_leaf`), it just flags that the
/// clipped content exists.
#[must_use]
pub fn overflowing_children(children: &[Fragment], full_cap: f32) -> Vec<NodeId> {
    if full_cap <= 0.0 {
        return Vec::new();
    }
    children
        .iter()
        .filter(|c| is_flowable(c) && c.height > full_cap)
        .map(|c| c.node_id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::value::BorderEdges;
    use turbo_dom_layout::BorderRadii;

    fn block(id: u32, y: f32, height: f32) -> Fragment {
        let content = FragmentContent::Box {
            background: None,
            border: BorderEdges::default(),
            border_radius: BorderRadii::ZERO,
            shadow: None,
            gradient: None,
            transform: None,
        };
        Fragment::new(NodeId(id), 0.0, y, 100.0, height, content)
    }

    fn facts2(used_width: f32, gap: f32) -> MulticolFacts {
        MulticolFacts {
            used_count: 2,
            used_width,
            gap,
        }
    }

    #[test]
    fn six_200px_blocks_in_two_columns_on_a_320px_body_fill_three_pages() {
        // r3-multicol: break-inside:avoid, used_count=2, page body 320.
        let kids: Vec<Fragment> = (0..6).map(|i| block(i as u32 + 1, 0.0, 200.0)).collect();
        let flow = flow_children(&kids, facts2(300.0, 0.0), (320.0, 320.0));
        let slot = |i: usize| flow.slots[i].unwrap();
        assert_eq!(
            slot(0),
            Slot {
                page: 0,
                col: 0,
                y_in_col: 0.0,
                height: 200.0
            }
        );
        assert_eq!(
            slot(1),
            Slot {
                page: 0,
                col: 1,
                y_in_col: 0.0,
                height: 200.0
            }
        );
        assert_eq!(
            slot(2),
            Slot {
                page: 1,
                col: 0,
                y_in_col: 0.0,
                height: 200.0
            }
        );
        assert_eq!(
            slot(3),
            Slot {
                page: 1,
                col: 1,
                y_in_col: 0.0,
                height: 200.0
            }
        );
        assert_eq!(
            slot(4),
            Slot {
                page: 2,
                col: 0,
                y_in_col: 0.0,
                height: 200.0
            }
        );
        assert_eq!(
            slot(5),
            Slot {
                page: 2,
                col: 1,
                y_in_col: 0.0,
                height: 200.0
            }
        );
        assert_eq!(flow.fill_heights, vec![200.0, 200.0, 200.0]);
        // Non-last pages fill to the page cap; the last page rule height
        // matches the filled content (CSS Multicol 1 §3).
        assert_eq!(flow.rule_heights, vec![320.0, 320.0, 200.0]);
    }

    #[test]
    fn seven_110px_blocks_in_three_columns_pack_six_on_page_one() {
        // r3-multicol-3cols: 7 blocks of 110px, 3 columns, 320 cap.
        let kids: Vec<Fragment> = (0..7).map(|i| block(i as u32 + 1, 0.0, 110.0)).collect();
        let three = MulticolFacts {
            used_count: 3,
            used_width: 173.0,
            gap: 0.0,
        };
        let flow = flow_children(&kids, three, (320.0, 320.0));
        let placed_pages: Vec<u32> = flow.slots.iter().map(|s| s.unwrap().page).collect();
        assert_eq!(placed_pages, [0, 0, 0, 0, 0, 0, 1]);
        // Each col holds two blocks (220px) on page 1 (3 cols * 2 = 6 blocks); the 7th
        // starts page 2 at col 0.
        assert_eq!(flow.slots[0].unwrap().col, 0);
        assert_eq!(flow.slots[1].unwrap().col, 0);
        assert_eq!(flow.slots[2].unwrap().col, 1);
        assert_eq!(flow.slots[5].unwrap().col, 2);
        assert_eq!(flow.slots[6].unwrap().col, 0);
    }

    #[test]
    fn a_shallow_first_page_cap_pushes_a_tall_child_to_the_next_full_cap() {
        // When the multicol sits mid-page, the first page's cap is smaller. A 200px
        // block fits a 320px full page but not a 120px first-page remainder, so after
        // one 100px block in column 0 the 200px block pushes to column 1 (where 200 >
        // 120 still overruns: push again), saturating used_count and spilling to a
        // fresh page at the full cap, where it fits whole in column 0.
        let kids = vec![block(1, 0.0, 100.0), block(2, 0.0, 200.0)];
        let flow = flow_children(&kids, facts2(200.0, 0.0), (120.0, 320.0));
        assert_eq!(
            flow.slots[0].unwrap(),
            Slot {
                page: 0,
                col: 0,
                y_in_col: 0.0,
                height: 100.0
            }
        );
        assert_eq!(
            flow.slots[1].unwrap(),
            Slot {
                page: 0,
                col: 1,
                y_in_col: 0.0,
                height: 200.0
            }
        );
    }

    #[test]
    fn column_rules_skipped_in_flow_restored_per_page() {
        let (style, color) = (ColumnRuleStyle::Solid, Rgba::BLACK);
        let mut rule = block(99, 0.0, 300.0);
        rule.content = FragmentContent::ColumnRule { style, color };
        rule.width = 4.0;
        let kids = vec![
            block(1, 0.0, 180.0),
            rule.clone(),
            block(2, 0.0, 180.0),
            block(3, 0.0, 180.0),
            block(4, 0.0, 180.0),
        ];
        let flow = flow_children(&kids, facts2(260.0, 16.0), (320.0, 320.0));
        assert!(flow.slots[1].is_none(), "rule not flowed");
        assert_eq!(flow.slots[0].unwrap().page, 0);
        assert_eq!(flow.slots[2].unwrap().page, 0);
        assert_eq!(flow.slots[3].unwrap().page, 1);
        let two = MulticolFacts {
            used_count: 2,
            used_width: 260.0,
            gap: 16.0,
        };
        let rules = column_rule_fragments(
            NodeId(1),
            (0.0, 0.0),
            two,
            (style, color, rule.width),
            180.0,
        );
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].height, 180.0);
        assert_eq!(rules[0].width, 4.0);
    }

    #[test]
    fn column_rules_skipped_when_gap_is_zero_or_used_count_is_one() {
        let two = MulticolFacts {
            used_count: 2,
            used_width: 300.0,
            gap: 0.0,
        };
        let one = MulticolFacts {
            used_count: 1,
            used_width: 600.0,
            gap: 10.0,
        };
        let r = (ColumnRuleStyle::Solid, Rgba::BLACK, 4.0);
        assert!(column_rule_fragments(NodeId(1), (0.0, 0.0), two, r, 100.0).is_empty());
        assert!(column_rule_fragments(NodeId(1), (0.0, 0.0), one, r, 100.0).is_empty());
    }

    #[test]
    fn an_empty_multicol_still_emits_one_container_page() {
        // R1 MEDIUM-1: `<div style="columns:2"></div>` must produce one page so
        // the walker can paint the container's own box and advance past it.
        let flow = flow_children(&[], facts2(300.0, 0.0), (320.0, 320.0));
        assert_eq!(flow.fill_heights, vec![0.0]);
        assert_eq!(flow.rule_heights, vec![0.0]);
        assert!(flow.slots.is_empty());
    }

    #[test]
    fn a_container_whose_only_children_are_dropped_rules_still_emits_one_page() {
        // R1 MEDIUM-1: a multicol that holds only a `ColumnRule` fragment the
        // paginator drops leaves `slots` all-None but still needs one page so
        // the container box gets painted.
        let mut rule = block(99, 0.0, 300.0);
        rule.content = FragmentContent::ColumnRule {
            style: ColumnRuleStyle::Solid,
            color: Rgba::BLACK,
        };
        rule.width = 4.0;
        let flow = flow_children(&[rule], facts2(260.0, 16.0), (320.0, 320.0));
        assert_eq!(flow.slots.len(), 1);
        assert!(flow.slots[0].is_none());
        assert_eq!(flow.fill_heights, vec![0.0]);
    }

    #[test]
    fn overflowing_children_flags_only_oversize_in_flow_kids() {
        // R1 MEDIUM-3: a multicol child that overruns the page body is listed
        // for a `LintCode::RegionOverflow` lint; column-rule fragments and
        // children that fit are skipped.
        let (style, color) = (ColumnRuleStyle::Solid, Rgba::BLACK);
        let mut rule = block(99, 0.0, 900.0);
        rule.content = FragmentContent::ColumnRule { style, color };
        rule.width = 4.0;
        let kids = vec![
            block(1, 0.0, 200.0), // fits a 320 cap
            block(2, 0.0, 500.0), // exceeds 320 → listed
            rule.clone(),         // non-flowable, always skipped
            block(3, 0.0, 321.0), // exceeds 320 → listed
        ];
        let flagged = overflowing_children(&kids, 320.0);
        assert_eq!(flagged, vec![NodeId(2), NodeId(3)]);
    }

    #[test]
    fn overflowing_children_short_circuits_on_a_non_positive_cap() {
        let kids = vec![block(1, 0.0, 10.0)];
        assert!(overflowing_children(&kids, 0.0).is_empty());
        assert!(overflowing_children(&kids, -5.0).is_empty());
    }

    #[test]
    fn rule_style_reads_the_first_column_rule_fragment() {
        let kids = vec![
            block(1, 0.0, 180.0),
            {
                let mut f = block(99, 0.0, 300.0);
                f.width = 3.0;
                f.content = FragmentContent::ColumnRule {
                    style: ColumnRuleStyle::Dashed,
                    color: Rgba {
                        r: 1,
                        g: 2,
                        b: 3,
                        a: 255,
                    },
                };
                f
            },
            block(2, 0.0, 180.0),
        ];
        let rule = rule_style(&kids).unwrap();
        assert_eq!(rule.0, ColumnRuleStyle::Dashed);
        assert_eq!(rule.1.r, 1);
        assert_eq!(rule.2, 3.0);
        assert!(rule_style(&[block(1, 0.0, 10.0)]).is_none());
    }
}
