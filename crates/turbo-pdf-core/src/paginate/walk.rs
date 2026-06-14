//! The break walk (§6.2): the core fragmenter. Walks the galley top-down,
//! filling one page's body capacity at a time and starting a fresh page when the
//! current one fills (AC-3.0.1) — page count is purely a function of content and
//! capacity, never an input (AC-6.0).
//!
//! Break-decision precedence (AC-6.2): forced (`break-before/after:page`) beats
//! atomicity (`break-inside:avoid`) beats widows/orphans beats greedy fill. A
//! block that cannot fit even on an empty page is split internally; an atomic
//! leaf taller than a page overflows with a [`LintCode::RegionOverflow`].
//!
//! Repeatable table headers (`BreakMeta.repeatable == Header`, AC-6.3/5.8) are
//! re-emitted atop every continued page of the table they head.
//!
//! Coordinates: input fragments are in absolute galley space; output fragments
//! are translated into page-local body coordinates (top-left of the body area is
//! `(margin.left, …)`; this walk emits relative to body-top `y = 0`, and page
//! assembly offsets by the body origin).
//!
//! **Deferred (documented):** when a block is split across pages its box
//! wrapper (background/border) is not re-emitted around each part — children are
//! emitted directly; orphans/widows are enforced at each single break but the
//! multi-page paragraph tail is greedy. Both are refined alongside the Phase 9
//! emitter.

use crate::error::{Diagnostics, LintCode, Span};
use crate::layout::fragment::{Fragment, FragmentContent, RepeatKind};
use crate::layout::value::BreakRule;

/// Per-page footnote reservation: the area (px) the page at the given 0-based
/// index must keep clear for its footnotes, subtracted from the base capacity
/// (§6.4). The default — used by the footnote-free entry points — reserves zero
/// on every page.
pub type Reserve<'a> = dyn Fn(usize) -> f32 + 'a;

/// Accumulates pages while walking the galley.
struct Walker<'a> {
    /// Base body height per page (net of header/footer bands); the footnote band
    /// is subtracted per page via `reserve`.
    capacity: f32,
    /// Footnote area reserved on each page, by 0-based page index.
    reserve: &'a Reserve<'a>,
    /// Completed and in-progress pages; the last entry is the current page.
    pages: Vec<Vec<Fragment>>,
    /// Next free page-local `y` on the current page.
    cursor: f32,
    /// Galley bottom of the last fragment placed on the current page, or `None`
    /// at the top of a page (so the leading inter-block gap is dropped).
    prev_bottom: Option<f32>,
}

impl<'a> Walker<'a> {
    fn new(capacity: f32, reserve: &'a Reserve<'a>) -> Walker<'a> {
        Walker {
            capacity,
            reserve,
            pages: vec![Vec::new()],
            cursor: 0.0,
            prev_bottom: None,
        }
    }

    /// The current (last) page's fragment list.
    fn current(&mut self) -> &mut Vec<Fragment> {
        self.pages.last_mut().expect("always one page in progress")
    }

    /// The usable body height on the current page: the base capacity less this
    /// page's reserved footnote area.
    fn page_capacity(&self) -> f32 {
        self.capacity - (self.reserve)(self.pages.len() - 1)
    }

    /// The galley gap to honor before `f` (0 at a page top or for overlap).
    fn gap_before(&self, f: &Fragment) -> f32 {
        self.prev_bottom.map_or(0.0, |b| (f.y - b).max(0.0))
    }

    /// Start a fresh page, re-emitting `headers` at its top.
    fn new_page(&mut self, headers: &[Fragment]) {
        self.pages.push(Vec::new());
        self.cursor = 0.0;
        self.prev_bottom = None;
        for h in headers {
            self.emit(h);
        }
    }

    /// Place `f` at the cursor (honoring its leading gap), translated into page
    /// coordinates, and advance the cursor.
    fn emit(&mut self, f: &Fragment) {
        let top = self.cursor + self.gap_before(f);
        let mut clone = f.clone();
        clone.translate(0.0, top - f.y);
        self.current().push(clone);
        self.cursor = top + f.height;
        self.prev_bottom = Some(f.bottom());
    }

    /// True if `f` fits in the remaining capacity of the current page.
    fn fits_now(&self, f: &Fragment) -> bool {
        self.cursor + self.gap_before(f) + f.height <= self.page_capacity()
    }

    /// True if `f` would fit on an otherwise-empty page below `headers`. The
    /// next page's reservation (one past the current index) bounds it.
    fn fits_on_empty(&self, f: &Fragment, headers: &[Fragment]) -> bool {
        let next = self.capacity - (self.reserve)(self.pages.len());
        headers_height(headers) + f.height <= next
    }

    /// Place one fragment, honoring forced breaks around it.
    fn place_one(&mut self, f: &Fragment, headers: &[Fragment], diags: &mut Diagnostics) {
        if f.break_meta.break_before == BreakRule::Page {
            self.new_page(headers);
        }
        self.place_fitting(f, headers, diags);
        if f.break_meta.break_after == BreakRule::Page {
            self.new_page(headers);
        }
    }

    /// Place `f` where it fits, breaking or splitting as needed.
    fn place_fitting(&mut self, f: &Fragment, headers: &[Fragment], diags: &mut Diagnostics) {
        if self.fits_now(f) {
            self.emit(f);
        } else if self.fits_on_empty(f, headers) {
            self.new_page(headers);
            self.emit(f);
        } else {
            self.split(f, headers, diags);
        }
    }

    /// Split a block that cannot fit on any page: recurse into its children, or
    /// overflow an atomic leaf with a lint.
    fn split(&mut self, f: &Fragment, headers: &[Fragment], diags: &mut Diagnostics) {
        if f.children.is_empty() {
            self.overflow_leaf(f, headers, diags);
        } else if is_paragraph(f) {
            self.split_paragraph(f, headers, diags);
        } else {
            self.split_block(f, headers, diags);
        }
    }

    /// An atomic fragment taller than a whole page: place it on a fresh page and
    /// let it overflow, flagging the clipped content.
    fn overflow_leaf(&mut self, f: &Fragment, headers: &[Fragment], diags: &mut Diagnostics) {
        if !self.current().is_empty() {
            self.new_page(headers);
        }
        self.emit(f);
        diags.push(
            LintCode::RegionOverflow,
            "content taller than the page body was clipped",
            Span::default(),
        );
    }

    /// Split a generic block: emit its repeatable headers, then its other
    /// children, re-emitting the headers atop each continued page.
    fn split_block(&mut self, f: &Fragment, headers: &[Fragment], diags: &mut Diagnostics) {
        let own_headers = collect_headers(&f.children);
        let active = if own_headers.is_empty() {
            headers.to_vec()
        } else {
            own_headers.clone()
        };
        for h in &own_headers {
            self.emit(h);
        }
        for child in &f.children {
            if !is_repeat_header(child) {
                self.place_one(child, &active, diags);
            }
        }
    }

    /// Split a paragraph line by line, honoring orphans and widows.
    fn split_paragraph(&mut self, f: &Fragment, headers: &[Fragment], diags: &mut Diagnostics) {
        let lines = &f.children;
        let orphans = f.break_meta.orphans as usize;
        let widows = f.break_meta.widows as usize;
        let mut start = 0;
        while start < lines.len() {
            start = self.place_line_run(lines, start, orphans, widows, headers, diags);
        }
    }

    /// Place one page's worth of lines starting at `start`; returns the next
    /// index to place (and breaks to a new page if lines remain).
    fn place_line_run(
        &mut self,
        lines: &[Fragment],
        start: usize,
        orphans: usize,
        widows: usize,
        headers: &[Fragment],
        diags: &mut Diagnostics,
    ) -> usize {
        let empty = self.current().is_empty();
        let fit = self.lines_that_fit(&lines[start..], empty, diags);
        let take = resolve_take(start, fit, lines.len(), orphans, widows, empty);
        if take == 0 {
            self.new_page(headers);
            return start;
        }
        for line in &lines[start..start + take] {
            self.emit(line);
        }
        let next = start + take;
        if next < lines.len() {
            self.new_page(headers);
        }
        next
    }

    /// How many leading lines fit on the current page; at least 1 on an empty
    /// page (forcing progress) with an overflow lint.
    fn lines_that_fit(&self, lines: &[Fragment], empty: bool, diags: &mut Diagnostics) -> usize {
        let cap = self.page_capacity();
        let mut cursor = self.cursor;
        let mut prev = self.prev_bottom;
        let mut count = 0;
        for line in lines {
            let gap = prev.map_or(0.0, |b| (line.y - b).max(0.0));
            if cursor + gap + line.height > cap {
                break;
            }
            cursor += gap + line.height;
            prev = Some(line.bottom());
            count += 1;
        }
        force_progress(count, empty, lines.is_empty(), diags)
    }
}

/// Guarantee at least one line is taken on an empty page so the walk terminates,
/// flagging the overflow.
fn force_progress(count: usize, empty: bool, no_lines: bool, diags: &mut Diagnostics) -> usize {
    if count == 0 && empty && !no_lines {
        diags.push(
            LintCode::RegionOverflow,
            "a single line is taller than the page body",
            Span::default(),
        );
        return 1;
    }
    count
}

/// Resolve how many lines to place: the widows/orphans-adjusted count, except
/// on an empty page where orphans cannot be satisfied — there we place whatever
/// fits (`fit`) rather than loop forever deferring to ever-fresh pages.
fn resolve_take(
    start: usize,
    fit: usize,
    total: usize,
    orphans: usize,
    widows: usize,
    empty: bool,
) -> usize {
    let take = adjust_widows_orphans(start, fit, total, orphans, widows);
    if take == 0 && empty {
        fit
    } else {
        take
    }
}

/// Pull lines back from a page break to satisfy widows (min lines after) and
/// orphans (min lines before), within what fits.
fn adjust_widows_orphans(
    start: usize,
    fit: usize,
    total: usize,
    orphans: usize,
    widows: usize,
) -> usize {
    if start + fit >= total {
        return fit; // no break here — the rest fits
    }
    let take = satisfy_widows(fit, total - start - fit, widows);
    enforce_orphans(take, orphans)
}

/// Reduce `take` so the lines pushed past the break number at least `widows`.
fn satisfy_widows(take: usize, remaining: usize, widows: usize) -> usize {
    if remaining < widows {
        return take.saturating_sub(widows - remaining);
    }
    take
}

/// If fewer than `orphans` lines would stay on this page, take none (defer the
/// whole run to the next page).
fn enforce_orphans(take: usize, orphans: usize) -> usize {
    if take < orphans {
        0
    } else {
        take
    }
}

/// True when every child is a text line (a paragraph eligible for line splitting).
fn is_paragraph(f: &Fragment) -> bool {
    !f.children.is_empty()
        && f.children
            .iter()
            .all(|c| matches!(c.content, FragmentContent::TextLine { .. }))
}

/// True when a fragment is a repeatable table header.
fn is_repeat_header(f: &Fragment) -> bool {
    f.break_meta.repeatable == Some(RepeatKind::Header)
}

/// The repeatable-header children of a block, in order.
fn collect_headers(children: &[Fragment]) -> Vec<Fragment> {
    children
        .iter()
        .filter(|c| is_repeat_header(c))
        .cloned()
        .collect()
}

/// Total stacked height of a header set (galley extents, gaps ignored).
fn headers_height(headers: &[Fragment]) -> f32 {
    headers.iter().map(|h| h.height).sum()
}

/// Walk the galley `root`'s children into pages of body fragments against a
/// per-page footnote `reserve` that shrinks each page's body capacity (§6.4).
/// `capacity` is the base usable body height per page; the footnote-free walk
/// passes a reserve that returns 0 everywhere. The fixpoint re-runs this with the
/// reservation it learns from the previous pass.
pub fn walk_reserved(
    root: &Fragment,
    capacity: f32,
    reserve: &Reserve,
    diags: &mut Diagnostics,
) -> Vec<Vec<Fragment>> {
    let mut w = Walker::new(capacity, reserve);
    for child in &root.children {
        w.place_one(child, &[], diags);
    }
    w.pages
}
