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
//! The flow implements Chrome 156's `column-fill: auto` for every page BUT
//! the last (saturating `used_count` columns at the page body cap) and
//! `column-fill: balance` on the last (CSS Multicol 1 §3 "The multi-column
//! model") — after the forward pass assigns kids to pages,
//! [`rebalance_last_page`] binary-searches the smallest `col_h` at which the
//! last page's kids fit in `used_count` columns and replaces their slots
//! accordingly, shrinking the column rule and the paginator's cursor
//! advance alike on a short last page. A child with `break-inside: avoid`
//! that would overrun the current column is pushed to a fresh one (CSS
//! Fragmentation 3 §5.2); a child with `break-before: column` or
//! `break-after: column` forces a column break around it (§3.2), honoured
//! by both the forward pass and the balance pass's fitness check so the
//! balanced `col_h` always fits with the break constraints. A child whose
//! height exceeds the page body's capacity is flagged with
//! [`LintCode::RegionOverflow`] and placed anyway (matching the generic
//! walker's `overflow_leaf` behaviour); pre-layout column-rule fragments are
//! dropped here (their geometry belongs to the one-shot balanced tree, not
//! the paginated one) and the paginator re-emits them per page from the
//! container's `MulticolFacts`. Out-of-flow descendants (`position:
//! absolute` / `fixed`) are filtered from the column flow by [`is_flowable`]
//! so the paginator can route them through
//! [`super::walk::Walker::place_out_of_flow`] (R1 HIGH-1).
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
//! **Supported in 0.8.1** (R1 landed): `break-before: column`,
//! `break-after: column` (R1 HIGH-2 via turbo-dom 0.8.1's new
//! [`BreakRule::Column`] variant), last-page `column-fill: balance`
//! (R1 HIGH-3) and OOF descendants (R1 HIGH-1) are all honoured.
//!
//! **Known limitations** (R1 review, deferred follow-ups):
//! * `column-span: all`: turbo-dom-layout 0.8.1 emits the band at the
//!   container's full inline size, but the paginator flows it as one column
//!   kid — closing the current sub-flow and opening a fresh one below needs
//!   a per-slot "is span-all" flag the paginator does not read yet.
//! * `break-inside: avoid-column`: a tall kid is flagged via
//!   `break_inside_avoid` (collapsed with plain `avoid`) in `BreakMeta`; the
//!   paginator wraps it to a fresh column via geometry overflow, but the
//!   layout stripe size for an over-tall kid stays at the pre-paginator
//!   balanced width instead of being resized to the paginator's `col_h`.
//! * Nested multicol: the inner container flows as a plain block inside the
//!   outer's column (its layout-side balanced placement is preserved, which
//!   matches Chrome 156 for the simple cases); a split of the inner across
//!   outer pages is not re-entered through `place_multicol`.
//! * `turbo-html2pdf-core`'s own `multicol_facts` resolves `used_count`
//!   differently from turbo-dom-layout's shared `resolve_tracks` for the
//!   `columns: <count> <width>` shorthand: our count beats a narrower
//!   content box, Chrome drops to 1 column. A follow-up routes
//!   `multicol_facts` through the shared resolver.

use crate::layout::fragment::{ColumnRuleStyle, Fragment, FragmentContent, NodeId};
use crate::layout::value::{BreakRule, Rgba};

use super::boxes::{FlowBoxes, MulticolFacts};

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

/// Classify a child by whether the paginator should flow it as a multicol
/// column kid. A container's pre-layout `FragmentContent::ColumnRule` is
/// dropped because the paginator re-emits rules per page ([`column_rule_fragments`]).
/// An out-of-flow descendant (`position: absolute` / `fixed`) whose [`super::boxes::BoxFacts`]
/// flags it [`super::boxes::BoxFacts::position::is_out_of_flow`] (R1 HIGH-1,
/// miaskiewicz/turbo-html2pdf#52) is skipped from the column flow too — the
/// paginator places those through [`super::walk::Walker::place_out_of_flow`]
/// so an abspos box with a top/left inset lands at its specified position,
/// not as an in-flow block in the first available column. Every other child
/// is an in-flow block whose fragment feeds into the per-page column fill.
pub fn is_flowable(child: &Fragment, boxes: &FlowBoxes) -> bool {
    if matches!(child.content, FragmentContent::ColumnRule { .. }) {
        return false;
    }
    !boxes
        .get(child.node_id)
        .is_some_and(|b| b.position.is_out_of_flow())
}

/// Flow `children` of a multicol container into pages, filling each page's
/// `used_count` columns top-to-bottom, left-to-right (CSS `column-fill:
/// auto`). `first_cap` is the body height left on the current page (the page
/// the multicol starts on; may be less than a full page when prior content
/// sits above it); `full_cap` is the body height of a fresh page. Returns a
/// per-child [`Slot`] (None for a child dropped by [`is_flowable`]) and the
/// filled column height of each page. An empty container (no in-flow
/// children, only dropped column rules or out-of-flow descendants) produces
/// one page with `fill_h = 0` so the walker can still emit the container's
/// own box and advance past it. R1 HIGH-2: a kid with `break-before: column`
/// or `break-after: column` forces a column break around it; without a
/// BreakRule::Column reader the paginator would place the kid at the current
/// column cursor like any other block.
#[must_use]
pub fn flow_children(
    children: &[Fragment],
    facts: MulticolFacts,
    caps: (f32, f32),
    boxes: &FlowBoxes,
) -> Flow {
    let mut state = FlowState::start(children.len(), caps);
    for child in children {
        state.step(child, facts, boxes);
    }
    rebalance_last_page(state.finish(), facts, children)
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

    fn step(&mut self, child: &Fragment, facts: MulticolFacts, boxes: &FlowBoxes) {
        if !is_flowable(child, boxes) {
            self.slots.push(None);
            return;
        }
        // R1 HIGH-2: honour break-before: column by forcing a column break
        // before the child when any content already sits in the current
        // column. CSS Fragmentation 3 §3.2.
        if matches!(child.break_meta.break_before, BreakRule::Column) {
            self.force_column_break(facts);
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
        // R1 HIGH-2: honour break-after: column by forcing a column break
        // after the child so the next sibling opens a fresh column.
        if matches!(child.break_meta.break_after, BreakRule::Column) {
            self.force_column_break(facts);
        }
    }

    /// Advance the cursor before placing a `h`-tall child: a child that
    /// overruns the current column pushes to the next (CSS Fragmentation 3
    /// §5.2 `break-inside: avoid`), and a run that saturates `used_count`
    /// columns opens a fresh page.
    fn maybe_wrap(&mut self, h: f32, facts: MulticolFacts) {
        if self.y_in_col <= 0.0 || self.y_in_col + h <= self.cap + FIT_EPS {
            return;
        }
        self.advance_column(facts);
    }

    /// Force a column break at the current cursor: move to the next column,
    /// or open a fresh page when all `used_count` columns are filled. A
    /// no-op when the current column is still empty (no content yet to break
    /// away from; CSS Fragmentation 3 §3.5 "class A breaks: at the start of
    /// a fragmentainer the break is suppressed").
    fn force_column_break(&mut self, facts: MulticolFacts) {
        if self.y_in_col <= 0.0 && self.col == 0 {
            return;
        }
        self.advance_column(facts);
    }

    /// Advance to the next column strip, opening a new page when `used_count`
    /// is exhausted.
    fn advance_column(&mut self, facts: MulticolFacts) {
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

/// Re-balance the slots on `flow`'s last page (R1 HIGH-3, column-fill:
/// balance): the smallest `col_h` at which the page's in-flow kids fit in
/// `used_count` columns, re-assigned column by column. Non-last pages are
/// left alone — they already fill `column-fill: auto` to the page cap, which
/// is also what Chrome 156 does for every page but the last on a paginated
/// multicol (CSS Multicol 1 §3 "The multi-column model"). The forward pass
/// already produced an auto-fill layout; this post-pass shrinks the last
/// page's filled column height to the balanced minimum and reshuffles its
/// kids into the resulting balanced columns.
#[must_use]
pub fn rebalance_last_page(flow: Flow, facts: MulticolFacts, kids: &[Fragment]) -> Flow {
    let Flow {
        mut slots,
        mut fill_heights,
        mut rule_heights,
    } = flow;
    let last_page = match fill_heights.len().checked_sub(1) {
        Some(p) => p as u32,
        None => {
            return Flow {
                slots,
                fill_heights,
                rule_heights,
            }
        }
    };
    let indices = last_page_indices(&slots, last_page);
    if indices.is_empty() || facts.used_count <= 1 {
        return Flow {
            slots,
            fill_heights,
            rule_heights,
        };
    }
    let col_h = balance_col_h(&indices, kids, facts.used_count);
    let fill_h = reflow_last_page(&mut slots, &indices, col_h, facts, last_page, kids);
    fill_heights[last_page as usize] = fill_h;
    // The rule on the last page always matches its filled column height
    // (CSS Multicol 1 §3). Non-last entries keep their auto-fill cap.
    if let Some(last_rule) = rule_heights.last_mut() {
        *last_rule = fill_h;
    }
    Flow {
        slots,
        fill_heights,
        rule_heights,
    }
}

/// Collect the slot indices of the kids placed on `last_page`, in source
/// order. Non-last-page slots are left alone; `None` slots (OOF descendants,
/// dropped column rules) are skipped.
fn last_page_indices(slots: &[Option<Slot>], last_page: u32) -> Vec<usize> {
    slots
        .iter()
        .enumerate()
        .filter_map(|(i, slot)| slot.and_then(|s| (s.page == last_page).then_some(i)))
        .collect()
}

/// Smallest `col_h` at which `kids[indices]` fits in `used_count` columns
/// under the paginator's atomic-kid flow (honouring `BreakRule::Column`
/// before and after a kid). Lower bound is `max(max_h, total / used_count)`;
/// upper bound is `total` (every kid in column 0). Thirty bisections give
/// 1 part in a billion, well under [`FIT_EPS`].
fn balance_col_h(indices: &[usize], kids: &[Fragment], used_count: u32) -> f32 {
    let heights: Vec<f32> = indices.iter().map(|&i| kids[i].height.max(0.0)).collect();
    let total: f32 = heights.iter().sum();
    let max_h = heights.iter().copied().fold(0.0_f32, f32::max);
    let n = used_count.max(1) as f32;
    let lo_init = max_h.max(total / n).max(0.0);
    let hi_init = total.max(lo_init);
    if fits_in_cols(indices, kids, lo_init, used_count) {
        return lo_init;
    }
    if !fits_in_cols(indices, kids, hi_init, used_count) {
        return hi_init;
    }
    let (mut lo, mut hi) = (lo_init, hi_init);
    for _ in 0..30 {
        let mid = 0.5 * (lo + hi);
        if fits_in_cols(indices, kids, mid, used_count) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// Mutable column cursor shared by [`fits_in_cols`] and
/// [`reflow_last_page`] so the fitness check and the placement run exactly
/// the same step logic.
struct ColCursor {
    col: u32,
    cur: f32,
    max_fill: f32,
}

impl ColCursor {
    fn new() -> ColCursor {
        ColCursor {
            col: 0,
            cur: 0.0,
            max_fill: 0.0,
        }
    }

    /// Apply `break-before: column` before placing a kid: advance one column
    /// unless we are at the fragmentainer start (col 0, cursor 0), where
    /// CSS Fragmentation 3 §3.5 "class A breaks" suppresses the break.
    fn pre_break(&mut self, kid: &Fragment) {
        if matches!(kid.break_meta.break_before, BreakRule::Column)
            && !(self.cur <= 0.0 && self.col == 0)
        {
            self.col += 1;
            self.cur = 0.0;
        }
    }

    /// Wrap to the next column when the kid would overrun the current one.
    fn wrap_if_overflows(&mut self, h: f32, col_h: f32) {
        if self.cur > 0.0 && self.cur + h > col_h + FIT_EPS {
            self.col += 1;
            self.cur = 0.0;
        }
    }

    /// Place a kid of height `h` at the cursor, updating `max_fill`.
    fn place(&mut self, h: f32) {
        self.cur += h;
        self.max_fill = self.max_fill.max(self.cur);
    }

    /// Apply `break-after: column` after placing a kid.
    fn post_break(&mut self, kid: &Fragment) {
        if matches!(kid.break_meta.break_after, BreakRule::Column) {
            self.col += 1;
            self.cur = 0.0;
        }
    }
}

/// Whether `kids[indices]` fits in `used_count` columns at `col_h`: a kid
/// that overruns the current column wraps to the next (the paginator's
/// atomic flow), and `BreakRule::Column` forces a column break around a
/// kid (CSS Fragmentation 3 §3.2, §3.5).
fn fits_in_cols(indices: &[usize], kids: &[Fragment], col_h: f32, used_count: u32) -> bool {
    if col_h <= 0.0 {
        return false;
    }
    let mut cursor = ColCursor::new();
    for &i in indices {
        let kid = &kids[i];
        cursor.pre_break(kid);
        cursor.wrap_if_overflows(kid.height.max(0.0), col_h);
        if cursor.col >= used_count {
            return false;
        }
        cursor.place(kid.height.max(0.0));
        cursor.post_break(kid);
    }
    true
}

/// Re-place the kids `indices` on `last_page` at the balanced `col_h`.
/// Mutates each slot's `col` and `y_in_col` in place. Returns the tallest
/// filled column after the pass. Honours `BreakRule::Column` so a forced
/// column break pins the paginator's column cursor even when balance would
/// otherwise collapse the two kids into one column.
fn reflow_last_page(
    slots: &mut [Option<Slot>],
    indices: &[usize],
    col_h: f32,
    facts: MulticolFacts,
    last_page: u32,
    kids: &[Fragment],
) -> f32 {
    let mut cursor = ColCursor::new();
    let last_col = facts.used_count.saturating_sub(1);
    for &i in indices {
        let kid = &kids[i];
        cursor.pre_break(kid);
        let h = kid.height.max(0.0);
        cursor.wrap_if_overflows(h, col_h);
        cursor.col = cursor.col.min(last_col);
        slots[i] = Some(Slot {
            page: last_page,
            col: cursor.col,
            y_in_col: cursor.cur,
            height: h,
        });
        cursor.place(h);
        cursor.post_break(kid);
    }
    cursor.max_fill
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
/// clipped content exists. Out-of-flow kids (R1 HIGH-1) do not count — they
/// are placed through [`super::walk::Walker::place_out_of_flow`] and do not
/// feed the column fill.
#[must_use]
pub fn overflowing_children(
    children: &[Fragment],
    full_cap: f32,
    boxes: &FlowBoxes,
) -> Vec<NodeId> {
    if full_cap <= 0.0 {
        return Vec::new();
    }
    children
        .iter()
        .filter(|c| is_flowable(c, boxes) && c.height > full_cap)
        .map(|c| c.node_id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::value::{BorderEdges, LengthPct, Position};
    use crate::paginate::BoxFacts;
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

    /// Empty `FlowBoxes` — no OOF kids, every child is in-flow. The paginator
    /// tests use this helper so the pre-R1-HIGH-1 invariants stay testable.
    fn no_oof() -> FlowBoxes {
        FlowBoxes::default()
    }

    /// `FlowBoxes` with `ids` marked as `position: absolute` so [`is_flowable`]
    /// filters them from the column flow.
    fn abspos(ids: &[u32]) -> FlowBoxes {
        let mut boxes = FlowBoxes::default();
        for &id in ids {
            boxes.insert(
                NodeId(id),
                BoxFacts {
                    position: Position::Absolute,
                    margin_top: 0.0,
                    margin_bottom: 0.0,
                    padding_top: 0.0,
                    padding_bottom: 0.0,
                    inset_top: LengthPct::Auto,
                    inset_bottom: LengthPct::Auto,
                    shift: 0.0,
                    definite_height: false,
                    multicol: None,
                },
            );
        }
        boxes
    }

    #[test]
    fn six_200px_blocks_in_two_columns_on_a_320px_body_fill_three_pages() {
        // r3-multicol: break-inside:avoid, used_count=2, page body 320.
        let kids: Vec<Fragment> = (0..6).map(|i| block(i as u32 + 1, 0.0, 200.0)).collect();
        let flow = flow_children(&kids, facts2(300.0, 0.0), (320.0, 320.0), &no_oof());
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
        let flow = flow_children(&kids, three, (320.0, 320.0), &no_oof());
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
        let flow = flow_children(&kids, facts2(200.0, 0.0), (120.0, 320.0), &no_oof());
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
        let flow = flow_children(&kids, facts2(260.0, 16.0), (320.0, 320.0), &no_oof());
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
        let flow = flow_children(&[], facts2(300.0, 0.0), (320.0, 320.0), &no_oof());
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
        let flow = flow_children(&[rule], facts2(260.0, 16.0), (320.0, 320.0), &no_oof());
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
        let flagged = overflowing_children(&kids, 320.0, &no_oof());
        assert_eq!(flagged, vec![NodeId(2), NodeId(3)]);
    }

    #[test]
    fn overflowing_children_short_circuits_on_a_non_positive_cap() {
        let kids = vec![block(1, 0.0, 10.0)];
        assert!(overflowing_children(&kids, 0.0, &no_oof()).is_empty());
        assert!(overflowing_children(&kids, -5.0, &no_oof()).is_empty());
    }

    #[test]
    fn abspos_descendants_are_filtered_from_the_column_flow() {
        // R1 HIGH-1: a `position: absolute` descendant of a multicol container
        // is filtered from `flow_children` so the paginator places it through
        // `place_out_of_flow` at its specified position, not as an in-flow
        // block that consumes column space. Here kid 2 is abspos; the flow
        // only sees kids 1 and 3 in the column fill, and the last-page
        // balance (R1 HIGH-3) places them side-by-side at col_h = 100.
        let kids = vec![
            block(1, 0.0, 100.0),
            block(2, 0.0, 50.0),  // abspos — filtered
            block(3, 0.0, 100.0), // in-flow, lands in col 1 after balance
        ];
        let flow = flow_children(&kids, facts2(300.0, 0.0), (320.0, 320.0), &abspos(&[2]));
        assert!(flow.slots[1].is_none(), "abspos kid must be filtered");
        assert_eq!(flow.slots[0].unwrap().y_in_col, 0.0);
        assert_eq!(flow.slots[0].unwrap().col, 0);
        // Kid 3 lands in col 1 at y=0 after balance (col_h = max(100, 200/2) =
        // 100 fits red col0 / blue col1); without the abspos filter kid 3
        // would be pushed to col 2 by the abspos's phantom column consumption.
        assert_eq!(flow.slots[2].unwrap().col, 1);
        assert_eq!(flow.slots[2].unwrap().y_in_col, 0.0);
        // Overflow listing treats the abspos kid the same way.
        let mut tall = kids;
        tall[1] = block(2, 0.0, 500.0); // would overflow if flowed
        let over = overflowing_children(&tall, 320.0, &abspos(&[2]));
        assert!(
            !over.contains(&NodeId(2)),
            "abspos kid not flow-overflowing"
        );
    }

    #[test]
    fn break_before_column_forces_a_new_column_before_the_kid() {
        // R1 HIGH-2: `break-before: column` on a multicol child moves it to
        // the next column (or opens a new page when the last column is
        // already full). Three 50px kids where kid 2 has break-before:column
        // — would naturally stack in col 0 (50+50+50=150 <= cap 320), but
        // kid 2 forces a break so it lands at col 1 y=0; kid 3 follows it at
        // col 1 y=50.
        let mut kids = vec![
            block(1, 0.0, 50.0),
            block(2, 0.0, 50.0),
            block(3, 0.0, 50.0),
        ];
        kids[1].break_meta.break_before = BreakRule::Column;
        let flow = flow_children(&kids, facts2(300.0, 0.0), (320.0, 320.0), &no_oof());
        assert_eq!(flow.slots[0].unwrap().col, 0);
        assert_eq!(flow.slots[1].unwrap().col, 1);
        assert_eq!(flow.slots[1].unwrap().y_in_col, 0.0);
        assert_eq!(flow.slots[2].unwrap().col, 1);
        assert_eq!(flow.slots[2].unwrap().y_in_col, 50.0);
    }

    #[test]
    fn break_before_column_at_the_fragmentainer_start_is_suppressed() {
        // CSS Fragmentation 3 §3.5 "class A breaks": a column break at the
        // start of the fragmentainer (column 0, cursor 0) does nothing.
        let mut kids = vec![block(1, 0.0, 50.0), block(2, 0.0, 50.0)];
        kids[0].break_meta.break_before = BreakRule::Column;
        let flow = flow_children(&kids, facts2(300.0, 0.0), (320.0, 320.0), &no_oof());
        assert_eq!(flow.slots[0].unwrap().col, 0, "no break at fresh col 0");
        assert_eq!(flow.slots[0].unwrap().y_in_col, 0.0);
    }

    #[test]
    fn break_after_column_pushes_the_next_kid_to_a_fresh_column() {
        // `break-after: column` on a kid closes its column; the next kid
        // opens the next one. Kid 1 at col 0, kid 1 has break-after:column,
        // so kid 2 starts col 1.
        let mut kids = vec![block(1, 0.0, 50.0), block(2, 0.0, 50.0)];
        kids[0].break_meta.break_after = BreakRule::Column;
        let flow = flow_children(&kids, facts2(300.0, 0.0), (320.0, 320.0), &no_oof());
        assert_eq!(flow.slots[0].unwrap().col, 0);
        assert_eq!(flow.slots[1].unwrap().col, 1);
        assert_eq!(flow.slots[1].unwrap().y_in_col, 0.0);
    }

    #[test]
    fn break_before_column_that_exhausts_the_last_column_opens_a_new_page() {
        // A kid with break-before:column when the cursor already sits in
        // the last column of the current page spills to a fresh page.
        let mut kids = vec![
            block(1, 0.0, 50.0), // col 0
            block(2, 0.0, 50.0), // col 1 (break-before:column from col 0)
            block(3, 0.0, 50.0), // page 1 col 0 (break-before:column from col 1)
        ];
        kids[1].break_meta.break_before = BreakRule::Column;
        kids[2].break_meta.break_before = BreakRule::Column;
        let flow = flow_children(&kids, facts2(300.0, 0.0), (320.0, 320.0), &no_oof());
        assert_eq!(flow.slots[1].unwrap().col, 1);
        assert_eq!(flow.slots[1].unwrap().page, 0);
        assert_eq!(flow.slots[2].unwrap().col, 0);
        assert_eq!(flow.slots[2].unwrap().page, 1);
    }

    #[test]
    fn the_last_page_balances_its_filled_column_height() {
        // R1 HIGH-3 (column-fill: balance on the last page, CSS Multicol 1
        // §3): three kids of 100px on a 320px-cap 2-column body. The forward
        // pass would stack all three in col 0 (300 <= 320); balance shrinks
        // the col_h to the smallest value that fits them in 2 cols (200), so
        // kid 1 lands col 0 y 0, kid 2 col 0 y 100, kid 3 col 1 y 0, and
        // `fill_heights[last]` collapses from 300 to 200.
        let kids: Vec<Fragment> = (0..3).map(|i| block(i as u32 + 1, 0.0, 100.0)).collect();
        let flow = flow_children(&kids, facts2(300.0, 0.0), (320.0, 320.0), &no_oof());
        assert_eq!(flow.slots[0].unwrap().col, 0);
        assert_eq!(flow.slots[0].unwrap().y_in_col, 0.0);
        assert_eq!(flow.slots[1].unwrap().col, 0);
        assert_eq!(flow.slots[1].unwrap().y_in_col, 100.0);
        assert_eq!(flow.slots[2].unwrap().col, 1);
        assert_eq!(flow.slots[2].unwrap().y_in_col, 0.0);
        assert_eq!(flow.fill_heights, vec![200.0]);
        // The last page's rule tracks the filled column, not the cap (the
        // rule shrinks on the balanced tail).
        assert_eq!(flow.rule_heights, vec![200.0]);
    }

    #[test]
    fn balance_honours_break_before_column_in_the_fitness_check() {
        // A kid with `break-before: column` forces a column break at the
        // balance col_h too; the balancer must pick a col_h large enough to
        // fit the resulting sub-flows. Kids [50, 50, 50] with break-before:
        // column on kid 2: balance col_h must allow kid 1 alone in col 0,
        // kids 2+3 in col 1 → smallest col_h is 100.
        let mut kids: Vec<Fragment> = (0..3).map(|i| block(i as u32 + 1, 0.0, 50.0)).collect();
        kids[1].break_meta.break_before = BreakRule::Column;
        let flow = flow_children(&kids, facts2(300.0, 0.0), (320.0, 320.0), &no_oof());
        // The balanced flow preserves the break: kid 2 opens col 1 regardless.
        assert_eq!(flow.slots[1].unwrap().col, 1);
        assert_eq!(flow.slots[1].unwrap().y_in_col, 0.0);
        assert_eq!(flow.slots[2].unwrap().col, 1);
        assert_eq!(flow.slots[2].unwrap().y_in_col, 50.0);
        assert_eq!(flow.fill_heights, vec![100.0]);
    }

    #[test]
    fn force_column_break_at_start_of_a_later_column_is_also_suppressed() {
        // CSS Fragmentation 3 §3.5 "class A breaks": a column break is
        // suppressed at the *start* of a fragmentainer, meaning column 0
        // only — a `break-before: column` on a kid whose predecessor just
        // closed a column (so cur == 0 but col > 0) is NOT at the
        // fragmentainer start and MUST advance. This test pins the
        // `col == 0` half of the suppression condition.
        let mut kids = vec![
            block(1, 0.0, 50.0),
            block(2, 0.0, 50.0),
            block(3, 0.0, 50.0),
        ];
        // kid 1 closes column 0, kid 2 opens column 1 with break-before:
        // column so it should still advance (end up in column 2 or spill),
        // not stay in column 1 at y=0 like a start-of-fragmentainer
        // suppression would predict.
        kids[0].break_meta.break_after = BreakRule::Column;
        kids[1].break_meta.break_before = BreakRule::Column;
        let three = MulticolFacts {
            used_count: 3,
            used_width: 100.0,
            gap: 0.0,
        };
        let flow = flow_children(&kids, three, (320.0, 320.0), &no_oof());
        assert_eq!(flow.slots[0].unwrap().col, 0);
        assert_eq!(flow.slots[1].unwrap().col, 2, "break-before kicks col 2");
        // Without the `col == 0` guard the suppression would fire at
        // col=1/cur=0 and kid 2 would stay at col 1.
    }

    #[test]
    fn balance_col_h_scales_with_total_over_used_count_not_modulo() {
        // Six 100px kids on 2 columns: smallest balanced col_h is 300
        // (three per column). Pinning this against a naive
        // `total % used_count` mutation would collapse the lower bound
        // to 0 and the binary search would still converge upward, but
        // the sum-based arithmetic is the one Chrome 156 relies on.
        let kids: Vec<Fragment> = (0..6).map(|i| block(i as u32 + 1, 0.0, 100.0)).collect();
        let flow = flow_children(&kids, facts2(300.0, 0.0), (600.0, 600.0), &no_oof());
        assert_eq!(flow.fill_heights, vec![300.0]);
        // Column 0 holds kids 0..=2 (y 0/100/200), column 1 kids 3..=5.
        assert_eq!(flow.slots[0].unwrap().col, 0);
        assert_eq!(flow.slots[2].unwrap().col, 0);
        assert_eq!(flow.slots[3].unwrap().col, 1);
        assert_eq!(flow.slots[5].unwrap().col, 1);
        assert_eq!(flow.slots[5].unwrap().y_in_col, 200.0);
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

    #[test]
    fn rebalance_last_page_on_an_empty_flow_returns_unchanged() {
        // Guard the defensive early-out in `rebalance_last_page` for the
        // never-happens-in-practice case where `fill_heights` is empty: the
        // forward pass always emits at least one page (empty-multicol test
        // above), but the function is `pub` and a caller could hand it a
        // default `Flow`. Returning the input untouched keeps the invariant
        // "a flow without pages has no last page to balance".
        let empty = Flow::default();
        let out = rebalance_last_page(empty, facts2(300.0, 0.0), &[]);
        assert!(out.slots.is_empty());
        assert!(out.fill_heights.is_empty());
        assert!(out.rule_heights.is_empty());
    }

    #[test]
    fn balance_col_h_returns_upper_bound_when_nothing_fits() {
        // `balance_col_h` falls back to the upper bound when `fits_in_cols`
        // rejects both the lower and upper trial heights. Kids of height 0
        // give `lo_init = hi_init = 0`, and `fits_in_cols` rejects
        // `col_h <= 0.0` (its first guard), so the function returns `hi_init`
        // (0.0) without entering the 30-step bisection.
        let kids = vec![block(1, 0.0, 0.0), block(2, 0.0, 0.0)];
        let indices = vec![0, 1];
        let col_h = balance_col_h(&indices, &kids, 2);
        assert_eq!(col_h, 0.0);
        // The `col_h <= 0.0` guard in `fits_in_cols` keeps the balancer from
        // dividing by zero or running unbounded; assert it directly too.
        assert!(!fits_in_cols(&indices, &kids, 0.0, 2));
        assert!(!fits_in_cols(&indices, &kids, -1.0, 2));
    }
}
