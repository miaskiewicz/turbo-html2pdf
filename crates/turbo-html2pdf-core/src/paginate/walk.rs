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
//! Margins at page starts (CSS Fragmentation §5.2, Chrome 156; miaskiewicz/turbo-html2pdf#31):
//! the first page starts at the galley top, so the leading gap of its first fragment — the
//! top margins (and the dissolved `<html>`/`<body>` margins, borders and padding) above it —
//! is kept; after an unforced break it is truncated (a fresh page's first fragment sits at
//! `y = 0`); after a forced break only the next block's own top margin (collapsed with its
//! first child's, [`super::boxes::leading_margin`]) is kept, not the larger bottom margin of
//! the block before the break (miaskiewicz/turbo-html2pdf#32 item 5; a box whose margins are
//! unknown — no [`FlowBoxes`] entry — keeps the galley gap). Only margins are truncated: a
//! block split across pages whose first content starts a page keeps its top border and
//! padding (and the margins inside them) above that content (item 4, `pending`).
//!
//! Relative positioning (miaskiewicz/turbo-html2pdf#34): a `relative`/`sticky` box is
//! painted shifted but the flow continues from its unshifted position, so gaps and fill
//! are measured on flow positions; the children of a split positioned box paint at its
//! level. A split block's shifted paint is cut at the page boundaries it crosses, the
//! parts past them painted on the neighbouring pages (miaskiewicz/turbo-html2pdf#42,
//! [`super::shifted`]).
//!
//! Box ends (item 6): no break separates a box's last content from the bottom padding and
//! border that end the box (and its ancestors', `<body>`'s included), so the last content
//! fits a page only together with them ([`super::boxes::trailing`]); a block that fits only
//! without them is split so just its last content moves on. A block taller than its content
//! instead ends in remaining height, which breaks like content: what does not fit runs onto
//! the next page(s) before the next block (miaskiewicz/turbo-html2pdf#33). Its height is
//! laid out from where it starts through the page capacities, each page it breaks on
//! counting to its end (gaps left by children moved on included), as Chrome 156 does
//! (miaskiewicz/turbo-html2pdf#43).
//!
//! Out-of-flow boxes (item 1, [`super::out_of_flow`]): fixed boxes and absolute boxes with a
//! vertical inset are taken out of the galley before this walk and placed afterwards from
//! the containing-block starts it records (`anchors`); an absolute box at its static
//! position stays in the flow and is placed there without taking space — on the page its
//! static position falls on, which may be before content the walk already placed (the
//! galley lists it after its in-flow siblings; miaskiewicz/turbo-html2pdf#41).
//!
//! Forced breaks (miaskiewicz/turbo-html2pdf#32): a forced break on a page with no flow
//! content yet — before the first content, or a `break-before` right after a
//! `break-after` — starts no new page (Chrome 156 makes no blank page for either).
//!
//! Split boxes (miaskiewicz/turbo-html2pdf#37 item 1): a block split across pages emits
//! its children directly; when its box paints (background, border, shadow) the walk
//! records where it starts and ends and slices it onto every page it spans, under its
//! content ([`super::decorate::slice_split_boxes`], `box-decoration-break: slice`, as the
//! dissolved `<html>`/`<body>` boxes are sliced from the per-page [`PageSpan`]s). A block
//! with no in-flow content is atomic, except that a content-free one (an empty block, a
//! table row of empty cells) that does not fit is sliced at the page end, as Chrome 156
//! slices it (miaskiewicz/turbo-html2pdf#47, [`sliceable`]). A block containing a forced
//! break is split there (miaskiewicz/turbo-html2pdf#38).
//!
//! **Deferred (documented):** orphans/widows are enforced at each single break but the
//! multi-page paragraph tail is greedy; a block with content that fits on an empty page
//! moves there whole rather than being split at the page end (AC-6.2, where Chrome would
//! split it).

use crate::error::{Diagnostics, LintCode, Span};
use crate::layout::fragment::{Fragment, FragmentContent, NodeId, RepeatKind};
use crate::layout::value::BreakRule;

use super::boxes::{
    bottom_frame, last_content, leading_margin, remaining_height, trailing, wrapper_tail, BoxFacts,
    FlowBoxes,
};
use super::decorate::{paints, slice_split_boxes, SplitBox};
use super::out_of_flow::{Anchors, Taken};
use super::shifted::{carry, pieces, window};

/// A row bound past any page end: the last part of a sliced block is not clipped below.
const FAR_END: f32 = 1.0e4;
use super::Galley;

/// Per-page footnote reservation: the area (px) the page at the given 0-based
/// index must keep clear for its footnotes, subtracted from the base capacity
/// (§6.4). The default — used by the footnote-free entry points — reserves zero
/// on every page.
pub type Reserve<'a> = dyn Fn(usize) -> f32 + 'a;

/// What one walked page holds, for slicing box decorations onto it: its usable body
/// height (the fragmentainer's extent) and, once anything is placed, the galley bottom of
/// the last fragment placed on it with that fragment's page-local bottom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSpan {
    pub capacity: f32,
    pub last: Option<(f32, f32)>,
}

/// The walk's result: each page's body fragments (page-local) and its [`PageSpan`].
pub struct Walked {
    pub pages: Vec<Vec<Fragment>>,
    pub spans: Vec<PageSpan>,
    /// Where each split positioned block starts (page, page-local top).
    pub anchors: Anchors,
    /// The inset absolute boxes of split positioned blocks, placed from those starts.
    pub deferred: Vec<Taken>,
}

/// How the current page started, for the gap kept above its first fragment.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PageStart {
    /// After an unforced break: the margins adjoining it are truncated.
    Unforced,
    /// After a forced break: the next block's own (collapsed) top margin is kept
    /// (miaskiewicz/turbo-html2pdf#32 item 5). Carries the galley bottom of the content
    /// before the break, whose gap is kept instead when that margin is unknown.
    Forced(Option<f32>),
}

/// Where a split painting block starts: page, index in its body, page-local top.
type BoxStart = (usize, usize, f32);

/// Accumulates pages while walking the galley.
struct Walker<'a> {
    /// The flow's used box values (margins after a forced break).
    boxes: &'a FlowBoxes,
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
    /// at the top of a page after an unforced break (so the leading inter-block gap
    /// is dropped). The first page starts at the galley top.
    prev_bottom: Option<f32>,
    /// One span per page in `pages`.
    spans: Vec<PageSpan>,
    /// No flow content placed on the current page yet (repeated headers aside): a
    /// forced break here is the break the page already started with (or, on the
    /// first page, a break before the first content), so it makes no new page —
    /// Chrome 156 ignores a forced break before the first content and treats
    /// `break-after` + `break-before` at one break point as one break.
    fresh: bool,
    /// How the current page started (read while `prev_bottom` is `None`).
    start: PageStart,
    /// The outermost block being split that has placed nothing yet: its galley top and
    /// collapsed top margin. A break before its first content is a break before the
    /// block, so a page it starts keeps the block's top border and padding (and the
    /// margins inside them) above that content (miaskiewicz/turbo-html2pdf#32 item 4).
    pending: Option<(f32, Option<f32>)>,
    /// The relative paint shift of the blocks being split around the current one: a
    /// `relative` box is painted shifted (its subtree with it) but the flow continues
    /// from its unshifted position, so gaps and fill are measured on flow positions
    /// (miaskiewicz/turbo-html2pdf#34).
    shift: f32,
    /// The paint layer of the innermost positioned block being split: its children are
    /// emitted as separate fragments, so each carries the block's positioned paint
    /// level (`z-index`), or they would paint under the flow that follows the block.
    layer: Option<Option<i32>>,
    /// The positioned blocks being split (id, galley top): the containing blocks of the
    /// absolute boxes the walk meets; where they start (`anchors`; `open`: those that
    /// placed nothing yet, anchored by their first fragment placed); and the inset
    /// absolute boxes deferred to be placed from those starts.
    positioned: Vec<(NodeId, f32)>,
    anchors: Anchors,
    open: Vec<(NodeId, f32)>,
    deferred: Vec<Taken>,
    /// The painting blocks being split (their own paint and, once their first fragment is
    /// placed, where they start) and those done, sliced onto their pages after the walk
    /// (miaskiewicz/turbo-html2pdf#37 item 1).
    decorated: Vec<(Fragment, Option<BoxStart>)>,
    split_boxes: Vec<SplitBox>,
    /// Set by [`Self::carry_frame`] for the block being closed: the page its content ends
    /// on and where, its bottom padding/border starting the next page.
    frame_cut: Option<(usize, f32)>,
    /// The blocks being split that are taller than their content (a definite height):
    /// their galley flow top and, once their first fragment is placed, where they start
    /// (page, page-local flow top) — their height is laid out from there through the
    /// page capacities (miaskiewicz/turbo-html2pdf#43).
    sized: Vec<(f32, Option<(usize, f32)>)>,
    /// Every fragment placed in the flow, in order: its galley flow bottom, page and
    /// page-local bottom — where a static position met after later content maps to
    /// (miaskiewicz/turbo-html2pdf#41).
    placed: Vec<(f32, usize, f32)>,
    /// The relatively offset blocks being split: the page of their first fragment and the
    /// fragments placed for them (page, index), re-cut onto the pages they span when they
    /// close (miaskiewicz/turbo-html2pdf#42, [`super::shifted`]).
    threads: Vec<Thread>,
}

/// A relatively offset block being split: its first page and its placed fragments.
type Thread = (Option<usize>, Vec<(usize, usize)>);

impl<'a> Walker<'a> {
    fn new(capacity: f32, reserve: &'a Reserve<'a>, galley: Galley<'a>) -> Walker<'a> {
        let top = galley.root.y;
        Walker {
            boxes: galley.boxes,
            capacity,
            reserve,
            pages: vec![Vec::new()],
            cursor: 0.0,
            prev_bottom: Some(top),
            spans: vec![PageSpan {
                capacity: capacity - reserve(0),
                last: None,
            }],
            fresh: true,
            start: PageStart::Unforced,
            pending: None,
            shift: 0.0,
            layer: None,
            positioned: Vec::new(),
            anchors: Anchors::new(),
            open: Vec::new(),
            deferred: Vec::new(),
            decorated: Vec::new(),
            split_boxes: Vec::new(),
            frame_cut: None,
            sized: Vec::new(),
            placed: Vec::new(),
            threads: Vec::new(),
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

    /// The galley gap to honor before `f` (0 for overlap), or at a page top the gap
    /// the page's start keeps.
    fn gap_before(&self, f: &Fragment) -> f32 {
        match self.prev_bottom {
            Some(b) => (self.flow_top(f) - b).max(0.0),
            None => self.page_start_gap(f),
        }
    }

    /// `f`'s own relative paint shift.
    fn own_shift(&self, f: &Fragment) -> f32 {
        self.boxes.get(f.node_id).map_or(0.0, |b| b.shift)
    }

    /// `f`'s galley top at its flow position (without the relative shifts).
    fn flow_top(&self, f: &Fragment) -> f32 {
        f.y - self.shift - self.own_shift(f)
    }

    /// The gap above the first fragment of a page. The page starts at the outermost
    /// block that `f` opens and that placed nothing yet (else at `f`): its top border
    /// and padding above `f` are kept, its top margin is truncated after an unforced
    /// break and kept after a forced one (or, unknown, the galley gap from the content
    /// before the break is kept).
    fn page_start_gap(&self, f: &Fragment) -> f32 {
        let flow_top = self.flow_top(f);
        let (top, margin) = self
            .pending
            .unwrap_or_else(|| (flow_top, leading_margin(self.boxes, f)));
        let inside = (flow_top - top).max(0.0);
        let PageStart::Forced(before) = self.start else {
            return inside;
        };
        match margin {
            Some(m) => inside + m.max(0.0),
            None => before.map_or(inside, |b| (flow_top - b).max(0.0)),
        }
    }

    /// Start a fresh page, re-emitting `headers` at its top.
    fn new_page(&mut self, headers: &[Fragment]) {
        self.pages.push(Vec::new());
        self.spans.push(PageSpan {
            capacity: self.page_capacity(),
            last: None,
        });
        self.cursor = 0.0;
        self.prev_bottom = None;
        self.start = PageStart::Unforced;
        for h in headers {
            self.emit(h);
        }
        // The content after repeated headers starts right below them: the gap before it
        // is the page start's, not its galley distance from the header's original place
        // at the top of the table (miaskiewicz/turbo-html2pdf#36).
        self.prev_bottom = None;
        self.fresh = true;
    }

    /// Start a fresh page at a forced break: margins after a forced break are kept
    /// (§5.2), so the next fragment keeps its own top margin (below the repeated headers
    /// opening the page, if any). A page with no flow content yet is already that page.
    fn forced_page(&mut self, headers: &[Fragment]) {
        if !self.fresh {
            let before = self.prev_bottom;
            self.new_page(headers);
            self.start = PageStart::Forced(before);
        }
    }

    /// Place `f` at the cursor (honoring its leading gap), translated into page
    /// coordinates (painted with its relative shifts), and advance the cursor.
    fn emit(&mut self, f: &Fragment) {
        let top = self.cursor + self.gap_before(f);
        self.put(f, top);
        self.advance(f, top + f.height);
    }

    /// `f` placed, its flow bottom at page-local `bottom` on the current page: the cursor
    /// moves below it.
    fn advance(&mut self, f: &Fragment, bottom: f32) {
        let flow_bottom = self.flow_top(f) + f.height;
        self.cursor = bottom;
        self.prev_bottom = Some(flow_bottom);
        self.fresh = false;
        self.pending = None;
        let page = self.pages.len() - 1;
        self.placed.push((flow_bottom, page, bottom));
        let span = self.spans.last_mut().expect("one span per page");
        span.last = Some((flow_bottom, bottom));
    }

    /// Slice content-free block `f` ([`sliceable`]) at the page end, as Chrome 156 slices
    /// an empty block or table row that does not fit (miaskiewicz/turbo-html2pdf#47): it
    /// starts at the cursor and runs on through the following pages (below their repeated
    /// headers), each page showing the next part of it — the whole box drawn there,
    /// clipped to the rows it covers on that page (`box-decoration-break: slice`). Its
    /// continuations are [`super::CARRIED`] copies: its own page carries its semantics.
    fn slice(&mut self, f: &Fragment, headers: &[Fragment]) {
        let mut top = self.cursor + self.gap_before(f);
        let (mut shown, mut first) = (0.0, true);
        while top - shown + f.height > self.page_capacity() {
            let rows = (top, self.page_capacity());
            self.put_rows(f, top - shown, rows, first);
            shown += rows.1 - top;
            first = false;
            self.new_page(headers);
            top = self.cursor;
            // Repeated headers filling the whole page leave no rows for the next
            // part: let the rest overflow here instead of adding pages forever.
            if top >= self.page_capacity() {
                break;
            }
        }
        self.put_rows(f, top - shown, (top, FAR_END), first);
        self.advance(f, top - shown + f.height);
    }

    /// Put `f` with its flow top at page-local `at`, clipped to the page-local rows
    /// `lo..hi`: through [`Self::put`] when it is `first` placed, else as a [`super::CARRIED`]
    /// copy.
    fn put_rows(&mut self, f: &Fragment, at: f32, (lo, hi): (f32, f32), first: bool) {
        if first {
            self.put(f, at);
        } else {
            let mut copy = f.clone();
            copy.translate(0.0, at - self.flow_top(f));
            carry(&mut copy);
            self.current().push(copy);
        }
        let piece = self.current().last_mut().expect("just pushed");
        let shift = piece.y - at;
        piece.clip = Some(window(piece, lo + shift, hi + shift));
    }

    /// Push a copy of `f` with its flow top at page-local `top` (painted with its
    /// relative shifts and its split ancestors' paint level), anchoring the containing
    /// blocks it starts.
    fn put(&mut self, f: &Fragment, top: f32) {
        self.put_on(self.pages.len() - 1, f, top);
    }

    /// [`Self::put`] onto page `page`.
    fn put_on(&mut self, page: usize, f: &Fragment, top: f32) {
        let mut clone = f.clone();
        clone.translate(0.0, top - self.flow_top(f));
        if let Some(z) = self.layer {
            lift(&mut clone, z);
        }
        for (id, galley_top) in std::mem::take(&mut self.open) {
            self.anchors
                .insert(id, (page, clone.y - (f.y - galley_top)));
        }
        let index = self.pages[page].len();
        self.start_blocks(f, (page, index), (top, clone.y));
        self.pages[page].push(clone);
    }

    /// Record `f`, going to `page` at `index` with its flow top at page-local `top`
    /// (painted at `painted`), as the start of the split blocks that placed nothing yet,
    /// and as a fragment of the offset block being split.
    fn start_blocks(
        &mut self,
        f: &Fragment,
        (page, index): (usize, usize),
        (top, painted): (f32, f32),
    ) {
        for (block, start) in self.decorated.iter_mut().filter(|(_, s)| s.is_none()) {
            *start = Some((page, index, painted - (f.y - block.y)));
        }
        let flow_top = self.flow_top(f);
        for (block_top, start) in self.sized.iter_mut().filter(|(_, s)| s.is_none()) {
            *start = Some((page, top - (flow_top - *block_top)));
        }
        if let Some((first, placed)) = self.threads.last_mut() {
            first.get_or_insert(page);
            placed.push((page, index));
        }
    }

    /// An out-of-flow box left in the flow (an `absolute` box with neither `top` nor
    /// `bottom`): at its static position, taking no space. The galley lists it after its
    /// in-flow siblings, so a static position above content already placed is mapped
    /// through the placed fragment ending at or above it, on that fragment's page
    /// (miaskiewicz/turbo-html2pdf#41).
    fn put_static(&mut self, f: &Fragment) {
        match self.placed_above(self.flow_top(f)) {
            Some((page, top)) => self.put_on(page, f, top),
            None => self.put(f, self.cursor + self.gap_before(f)),
        }
    }

    /// Where galley flow position `y` lies when content below it was placed already:
    /// the page and page-local position below the last placed fragment ending at or
    /// above it (`None` when `y` is at or below the content placed so far, or above all
    /// of it).
    fn placed_above(&self, y: f32) -> Option<(usize, f32)> {
        if self.prev_bottom.is_none_or(|b| y > b - 0.5) {
            return None;
        }
        let &(bottom, page, local) = self.placed.iter().rev().find(|p| p.0 <= y + 0.5)?;
        Some((page, local + (y - bottom)))
    }

    /// An out-of-flow box the walk meets: an inset absolute box of the positioned block
    /// being split is deferred, placed from that block's start after the walk; one at its
    /// static position is placed there.
    fn place_out_of_flow(&mut self, f: &Fragment) {
        let anchored = self.boxes.get(f.node_id).is_some_and(BoxFacts::anchored);
        match self.positioned.last() {
            Some(&(cb, top)) if anchored => self.deferred.push(Taken {
                frag: f.clone(),
                cb: Some(cb),
                dy: f.y - top,
            }),
            _ => self.put_static(f),
        }
    }

    /// True if `f`, followed by `tail` px that must stay on its page, fits in the
    /// remaining capacity of the current page.
    fn fits_now(&self, f: &Fragment, tail: f32) -> bool {
        self.cursor + self.gap_before(f) + f.height + tail <= self.page_capacity()
    }

    /// True if `f` (followed by `tail`) would fit on an otherwise-empty page below
    /// `headers`. The next page's reservation (one past the current index) bounds it.
    fn fits_on_empty(&self, f: &Fragment, headers: &[Fragment], tail: f32) -> bool {
        let next = self.capacity - (self.reserve)(self.pages.len());
        headers_height(headers) + f.height + tail <= next
    }

    /// Place one fragment, honoring forced breaks around it. `tail` is what must
    /// follow it on its page (see [`Self::placement`]).
    fn place_one(
        &mut self,
        f: &Fragment,
        headers: &[Fragment],
        tail: f32,
        diags: &mut Diagnostics,
    ) {
        if self
            .boxes
            .get(f.node_id)
            .is_some_and(|b| b.position.is_out_of_flow())
        {
            return self.place_out_of_flow(f);
        }
        if f.break_meta.break_before == BreakRule::Page {
            self.forced_page(headers);
        }
        self.place_fitting(f, headers, tail, diags);
        if f.break_meta.break_after == BreakRule::Page {
            self.forced_page(headers);
        }
    }

    /// Place `f` where it fits, breaking or splitting as needed.
    fn place_fitting(
        &mut self,
        f: &Fragment,
        headers: &[Fragment],
        tail: f32,
        diags: &mut Diagnostics,
    ) {
        match self.placement(f, headers, tail) {
            Placement::Here => self.emit(f),
            Placement::NextPage => {
                self.new_page(headers);
                self.emit(f);
            }
            Placement::Split => self.split(f, headers, tail, diags),
            Placement::Slice => self.slice(f, headers),
        }
    }

    /// Where `f` goes: here if it fits; else on the next page if it fits on an empty
    /// one; else split. `tail` is the bottom padding/border (and the margins inside
    /// them) of the ancestors `f` is the last content of: it cannot be separated from
    /// `f` by a break, so `f` fits only with it (miaskiewicz/turbo-html2pdf#32 item 6;
    /// Chrome 156 moves a block that exactly fills the page to the next one with that
    /// padding). A block that fits here only without its tail is split, so just its
    /// last content moves on; a leaf moves on if it fits on an empty page with its
    /// tail, and otherwise stays.
    fn placement(&self, f: &Fragment, headers: &[Fragment], tail: f32) -> Placement {
        // A forced break inside `f` breaks the page there (miaskiewicz/turbo-html2pdf#38):
        // the walker must visit its children.
        if self.breaks_inside(f) {
            return Placement::Split;
        }
        if self.fits_now(f, tail) {
            return Placement::Here;
        }
        if sliceable(self.boxes, f) {
            return Placement::Slice;
        }
        self.placement_breaking(f, headers, tail)
    }

    /// [`Self::placement`] of an `f` that does not fit here with its `tail` and is not
    /// sliced: split if it fits here without its tail, else moved to the next page if it
    /// fits on an empty one, else split.
    fn placement_breaking(&self, f: &Fragment, headers: &[Fragment], tail: f32) -> Placement {
        if self.fits_now(f, 0.0) {
            return self.placement_without_tail(f, headers, tail);
        }
        if self.fits_on_empty(f, headers, 0.0) {
            return Placement::NextPage;
        }
        Placement::Split
    }

    /// Whether `f` has in-flow content and a forced break inside it.
    fn breaks_inside(&self, f: &Fragment) -> bool {
        last_content(self.boxes, &f.children).is_some() && f.children.iter().any(has_forced_break)
    }

    /// [`Self::placement`] of an `f` that fits here only without its `tail`.
    fn placement_without_tail(&self, f: &Fragment, headers: &[Fragment], tail: f32) -> Placement {
        if !f.children.is_empty() && !f.break_meta.break_inside_avoid {
            Placement::Split
        } else if self.fits_on_empty(f, headers, tail) {
            Placement::NextPage
        } else {
            Placement::Here
        }
    }

    /// Split a block that cannot fit on any page: recurse into its children, or
    /// overflow an atomic leaf with a lint. `tail` follows its last content.
    fn split(&mut self, f: &Fragment, headers: &[Fragment], tail: f32, diags: &mut Diagnostics) {
        // A block without in-flow content (none, or only out-of-flow boxes) is atomic.
        if last_content(self.boxes, &f.children).is_none() {
            return self.overflow_leaf(f, headers, diags);
        }
        let opened = self.open_block(f);
        let shift = self.own_shift(f);
        self.shift += shift;
        let layer = self.layer;
        self.layer = layer_of(f, layer);
        if is_paragraph(f) {
            let sized = self.begin_sized(f);
            self.split_paragraph(f, headers, tail, diags);
            self.end_block(f, (sized, f.children.len().checked_sub(1)), headers);
        } else {
            self.split_block(f, headers, tail, diags);
        }
        self.layer = layer;
        self.shift -= shift;
        self.close_block(f);
        if opened {
            self.pending = None;
        }
    }

    /// Start splitting `f`: it becomes the pending page-start block unless an enclosing
    /// one is (returns whether it did), and a positioned block becomes the containing
    /// block of the absolute boxes met inside it, anchored by its first fragment placed.
    fn open_block(&mut self, f: &Fragment) -> bool {
        if f.is_positioned {
            self.positioned.push((f.node_id, f.y));
            self.open.push((f.node_id, f.y));
        }
        if paints(f) {
            let mut own = f.clone();
            own.children.clear();
            self.decorated.push((own, None));
        }
        if self.own_shift(f) != 0.0 {
            self.threads.push((None, Vec::new()));
        }
        let opened = self.pending.is_none();
        if opened {
            self.pending = Some((self.flow_top(f), leading_margin(self.boxes, f)));
        }
        opened
    }

    /// Done splitting `f`: a positioned block stops being the containing block of what the
    /// walk meets. Its start is always known by now: `split` only opens a block with
    /// in-flow content (`last_content`), every in-flow child is placed through `put`
    /// (`emit`, `split` recursing to it, `overflow_leaf`), and `put` anchors and drains
    /// every open block — so nothing is left open when it closes.
    fn close_block(&mut self, f: &Fragment) {
        if f.is_positioned {
            self.positioned.pop();
            debug_assert!(self.open.is_empty(), "a split block placed nothing");
        }
        let cut = self.frame_cut.take();
        if paints(f) {
            self.close_decorated(f, cut);
        }
        if self.own_shift(f) != 0.0 {
            let (first, placed) = self.threads.pop().expect("opened with the block");
            self.recut(first.expect("a split block places its content"), &placed);
        }
    }

    /// Re-cut the fragments `placed` for a relatively offset block that started on page
    /// `first` and ends on the current page onto the pages its shifted paint reaches
    /// ([`pieces`]): a fragment keeps its place, clipped to its page's window, and its
    /// parts past that window go to the neighbouring pages.
    fn recut(&mut self, first: usize, placed: &[(usize, usize)]) {
        let range = (first, self.pages.len() - 1);
        for &(page, index) in placed {
            for (q, piece) in pieces(&self.pages[page][index], page, range, &self.spans) {
                if q == page {
                    self.pages[page][index] = piece;
                } else {
                    self.pages[q].push(piece);
                }
            }
        }
    }

    /// Done splitting painting block `f`: its box ends below its last content by its
    /// trailing padding/border (at most the page area's end), on the current page.
    fn close_decorated(&mut self, f: &Fragment, cut: Option<(usize, f32)>) {
        if let Some((frag, Some(start))) = self.decorated.pop() {
            self.end_decorated(f, frag, start);
            if let Some(b) = self.split_boxes.last_mut() {
                b.cut = cut;
            }
        }
    }

    /// Record split painting block `f` (`frag`, started at `start`) as ending on the
    /// current page.
    fn end_decorated(&mut self, f: &Fragment, frag: Fragment, start: BoxStart) {
        let flow_bottom = self.flow_top(f) + f.height;
        let trailing = self.prev_bottom.map_or(0.0, |b| (flow_bottom - b).max(0.0));
        let visual = self.shift + self.own_shift(f);
        let bottom = (self.cursor + trailing + visual).min(self.page_capacity());
        let end = (self.pages.len() - 1, bottom);
        self.split_boxes.push(SplitBox {
            frag,
            start,
            end,
            shift: visual,
            cut: None,
        });
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
    fn split_block(
        &mut self,
        f: &Fragment,
        headers: &[Fragment],
        tail: f32,
        diags: &mut Diagnostics,
    ) {
        let own_headers = collect_headers(&f.children);
        let active = if own_headers.is_empty() {
            headers.to_vec()
        } else {
            own_headers.clone()
        };
        let last = last_content(self.boxes, &f.children);
        let sized = self.begin_sized(f);
        for h in &own_headers {
            self.emit(h);
        }
        self.place_children(f, &active, (tail, last), diags);
        self.end_block(f, (sized, last), headers);
    }

    /// Start split block `f`: one with a definite height records where its content
    /// starts (through `put`) so [`Self::end_sized`] can lay its height out; returns
    /// whether it has one.
    fn begin_sized(&mut self, f: &Fragment) -> bool {
        let sized = self.boxes.get(f.node_id).is_some_and(|b| b.definite_height);
        if sized {
            // `f`'s own shift is already in `self.shift` here.
            self.sized.push((f.y - self.shift, None));
        }
        sized
    }

    /// End split block `f` after its content (`last`): a definite height (`sized`) is laid
    /// out through its pages, an auto one carries its galley remaining height.
    fn end_block(
        &mut self,
        f: &Fragment,
        (sized, last): (bool, Option<usize>),
        headers: &[Fragment],
    ) {
        if sized {
            // Its in-flow content is placed through `put`, which records its start.
            let start = self.sized.pop().and_then(|(_, start)| start);
            self.end_sized(f, start.expect("a split block places its content"), headers);
        } else if let Some(i) = last {
            self.carry_remaining(f, &f.children[i], headers);
        }
    }

    /// Place split block `f`'s children (its repeated headers aside) below `headers`; its
    /// last content (index `last`) carries `f`'s tail.
    fn place_children(
        &mut self,
        f: &Fragment,
        headers: &[Fragment],
        (tail, last): (f32, Option<usize>),
        diags: &mut Diagnostics,
    ) {
        for (i, child) in f.children.iter().enumerate() {
            if !is_repeat_header(child) {
                let after = self.tail_of(f, child, tail, Some(i) == last);
                self.place_one(child, headers, after, diags);
            }
        }
    }

    /// After the last content of split block `f`, with a definite height and started at
    /// `start` (page, page-local top): Chrome 156 counts the space `f` takes on each page
    /// it breaks on (to the page end, including a gap left when a child moves to the next
    /// page) toward its height, so `f` ends that far into the current page — above its
    /// last content when that overflows it — or, its remaining height not fitting, runs on
    /// the next page(s), where the next block follows it (miaskiewicz/turbo-html2pdf#33,
    /// #43 — only margins are truncated at a break). A height used up before the current
    /// page ends `f` at its top, where the next block starts as after a break.
    fn end_sized(&mut self, f: &Fragment, (first, top): (usize, f32), headers: &[Fragment]) {
        let page = self.pages.len() - 1;
        let used: f32 = self.spans[first..page].iter().map(|s| s.capacity).sum();
        let end = top + f.height - used;
        let bottom = f.bottom() - self.shift;
        if end <= 0.0 {
            self.cursor = 0.0;
            self.prev_bottom = None;
        } else if end > self.page_capacity() {
            let over = end - self.page_capacity();
            self.continue_box(over, bottom, headers);
        } else {
            self.end_box_at(end, bottom);
        }
    }

    /// After the last content `last` of split block `f` with an auto height: a box taller
    /// than its content (`min-height`, a table's spacing) runs that remaining height and
    /// its bottom padding/border on below it; what does not fit this page continues on
    /// the next page(s), where the next block follows it (miaskiewicz/turbo-html2pdf#33).
    /// `last`'s bottom margin before it is truncated at the page end.
    fn carry_remaining(&mut self, f: &Fragment, last: &Fragment, headers: &[Fragment]) {
        let Some(rest) = remaining_height(self.boxes, f, last) else {
            return self.carry_frame(f, headers);
        };
        let margin = f.bottom() - last.bottom() - rest;
        let cap = self.page_capacity();
        if self.cursor + margin + rest > cap {
            let over = rest - (cap - self.cursor - margin).max(0.0);
            self.continue_box(over, f.bottom() - self.shift, headers);
        }
    }

    /// After the last content of split block `f` with an auto height: a bottom
    /// padding/border that was too tall to keep with that content ([`Self::tail_of`]) and
    /// does not fit what is left of the page starts the next page whole and runs on from
    /// there, like Chrome 156 (miaskiewicz/turbo-html2pdf#49); the last content's bottom
    /// margin before it is truncated at the page end.
    fn carry_frame(&mut self, f: &Fragment, headers: &[Fragment]) {
        let frame = bottom_frame(self.boxes, f);
        let end = self.prev_bottom.map_or(self.cursor, |b| {
            self.cursor + (f.bottom() - self.shift - b).max(0.0)
        });
        if frame > 0.0 && end > self.page_capacity() + 0.5 {
            self.frame_cut = Some((self.pages.len() - 1, self.cursor));
            self.continue_box(frame, f.bottom() - self.shift, headers);
        }
    }

    /// The box ending at galley `bottom` ends at page-local `end` on the current page.
    fn end_box_at(&mut self, end: f32, bottom: f32) {
        self.cursor = end;
        self.prev_bottom = Some(bottom);
        self.fresh = false;
        let span = self.spans.last_mut().expect("one span per page");
        span.last = Some((bottom, end));
    }

    /// Run `over` px of a box ending at galley `bottom` onto the following page(s).
    fn continue_box(&mut self, mut over: f32, bottom: f32, headers: &[Fragment]) {
        self.new_page(headers);
        while over > self.page_capacity() {
            over -= self.page_capacity();
            self.new_page(headers);
        }
        self.end_box_at(self.cursor + over, bottom);
    }

    /// What must follow `child` of `parent` on its page: for the parent's last content,
    /// the parent's [`trailing`] extent (else nothing).
    fn tail_of(&self, parent: &Fragment, child: &Fragment, tail: f32, last: bool) -> f32 {
        if !last {
            return 0.0;
        }
        let after = trailing(self.boxes, parent, child, tail);
        // A tail too tall to fit even a fresh page with the last piece of content it
        // follows does not hold that content back: it breaks after it instead
        // ([`Self::carry_frame`]).
        if self.last_leaf(child).height + after > self.capacity + 0.5 {
            0.0
        } else {
            after
        }
    }

    /// What must follow the galley's last content `child`: the dissolved `<html>`/`<body>`
    /// tail ([`wrapper_tail`]), unless it is too tall to keep with it (as [`Self::tail_of`]).
    fn wrapper_tail_of(&self, galley: Galley, child: &Fragment) -> f32 {
        let after = wrapper_tail(galley, child);
        if self.last_leaf(child).height + after > self.capacity + 0.5 {
            0.0
        } else {
            after
        }
    }

    /// After the galley's last content: the `<html>`/`<body>` bottom padding/border that
    /// does not fit what is left of the page starts the next page and runs on, as
    /// [`Self::carry_frame`] (miaskiewicz/turbo-html2pdf#49).
    fn carry_wrapper_frame(&mut self, galley: Galley) {
        let Some(i) = last_content(galley.boxes, &galley.root.children) else {
            return;
        };
        let frame = wrapper_tail(galley, &galley.root.children[i]);
        if self.cursor + frame > self.page_capacity() + 0.5 {
            let bottom = self.prev_bottom.unwrap_or_default() + frame;
            self.continue_box(frame, bottom, &[]);
        }
    }

    /// The innermost last content of `f` (itself when it has none): what a tail after `f`
    /// must share a page with.
    fn last_leaf<'f>(&self, f: &'f Fragment) -> &'f Fragment {
        match last_content(self.boxes, &f.children) {
            Some(i) => self.last_leaf(&f.children[i]),
            None => f,
        }
    }

    /// Split a paragraph line by line, honoring orphans and widows.
    fn split_paragraph(
        &mut self,
        f: &Fragment,
        headers: &[Fragment],
        tail: f32,
        diags: &mut Diagnostics,
    ) {
        let lines = &f.children;
        let run = LineRun {
            lines,
            orphans: f.break_meta.orphans as usize,
            widows: f.break_meta.widows as usize,
            tail: lines
                .last()
                .map_or(0.0, |l| trailing(self.boxes, f, l, tail)),
        };
        let mut start = 0;
        while start < lines.len() {
            start = self.place_line_run(&run, start, headers, diags);
        }
    }

    /// Place one page's worth of lines starting at `start`; returns the next
    /// index to place (and breaks to a new page if lines remain).
    fn place_line_run(
        &mut self,
        run: &LineRun,
        start: usize,
        headers: &[Fragment],
        diags: &mut Diagnostics,
    ) -> usize {
        let lines = run.lines;
        let empty = self.fresh || self.current().is_empty();
        let fit = self.lines_that_fit(&lines[start..], run.tail, empty, diags);
        let take = resolve_take(start, fit, lines.len(), run.orphans, run.widows, empty);
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

    /// How many leading lines fit on the current page (the last of `lines` with the
    /// paragraph's `tail` after it); at least 1 on an empty page (forcing progress)
    /// with an overflow lint.
    fn lines_that_fit(
        &self,
        lines: &[Fragment],
        tail: f32,
        empty: bool,
        diags: &mut Diagnostics,
    ) -> usize {
        let cap = self.page_capacity();
        let mut cursor = self.cursor;
        let mut prev = self.prev_bottom;
        let mut count = 0;
        for (i, line) in lines.iter().enumerate() {
            let gap = prev.map_or_else(
                || self.page_start_gap(line),
                |b| (self.flow_top(line) - b).max(0.0),
            );
            let after = if i + 1 == lines.len() { tail } else { 0.0 };
            if cursor + gap + line.height + after > cap {
                break;
            }
            cursor += gap + line.height;
            prev = Some(self.flow_top(line) + line.height);
            count += 1;
        }
        force_progress(count, empty, lines.is_empty(), diags)
    }
}

/// The paint layer for the children of split block `f`: its own when it is positioned,
/// else the enclosing one.
fn layer_of(f: &Fragment, enclosing: Option<Option<i32>>) -> Option<Option<i32>> {
    if f.is_positioned {
        Some(f.z_index)
    } else {
        enclosing
    }
}

/// Paint `f` (a child emitted on its own) at its split positioned ancestor's level: a
/// `z-index` ancestor (a stacking context) puts it at that level; a `z-index: auto` one
/// makes an in-flow child paint as positioned.
fn lift(f: &mut Fragment, z: Option<i32>) {
    if z.is_some() || !f.is_positioned {
        f.z_index = z.or(f.z_index);
        f.is_positioned = true;
    }
}

/// Whether `f` or a fragment inside it carries a forced break (`break-before` /
/// `break-after: page`).
fn has_forced_break(f: &Fragment) -> bool {
    let forced = |rule: BreakRule| rule == BreakRule::Page;
    forced(f.break_meta.break_before)
        || forced(f.break_meta.break_after)
        || f.children.iter().any(has_forced_break)
}

/// Where [`Walker::placement`] puts a fragment.
enum Placement {
    Here,
    NextPage,
    Split,
    Slice,
}

/// Whether `f` is a content-free block Chrome 156 slices across a page break
/// (miaskiewicz/turbo-html2pdf#47): a box with nothing but such boxes inside (an empty
/// `<div>`, a table row of empty cells), none of them `break-inside: avoid`, positioned,
/// out of flow or a repeated table header (a table repeats it rather than slicing).
fn sliceable(boxes: &FlowBoxes, f: &Fragment) -> bool {
    let plain = matches!(f.content, FragmentContent::Box { .. }) && !f.is_positioned;
    plain && breakable(boxes, f) && f.children.iter().all(|c| sliceable(boxes, c))
}

/// Whether nothing keeps `f` whole: no `break-inside: avoid` in its break hints (a table
/// row's included, turbo-dom#158), not out of flow, not a repeated table header.
fn breakable(boxes: &FlowBoxes, f: &Fragment) -> bool {
    let facts = boxes.get(f.node_id);
    let avoid = f.break_meta.break_inside_avoid;
    let out = facts.is_some_and(|b| b.position.is_out_of_flow());
    !avoid && !out && !is_repeat_header(f)
}

/// A paragraph's lines being split across pages, with its widows/orphans and the
/// tail that follows its last line.
struct LineRun<'l> {
    lines: &'l [Fragment],
    orphans: usize,
    widows: usize,
    tail: f32,
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
/// reservation it learns from the previous pass. The first page starts at the top
/// of `root`.
pub fn walk_reserved(
    galley: Galley,
    capacity: f32,
    reserve: &Reserve,
    diags: &mut Diagnostics,
) -> Walked {
    let mut w = Walker::new(capacity, reserve, galley);
    let children = &galley.root.children;
    let last = last_content(galley.boxes, children);
    for (i, child) in children.iter().enumerate() {
        let tail = if Some(i) == last {
            w.wrapper_tail_of(galley, child)
        } else {
            0.0
        };
        w.place_one(child, &[], tail, diags);
    }
    w.carry_wrapper_frame(galley);
    let mut pages = w.pages;
    slice_split_boxes(&mut pages, &w.spans, &w.split_boxes);
    Walked {
        pages,
        spans: w.spans,
        anchors: w.anchors,
        deferred: w.deferred,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::fragment::{FragmentContent, NodeId};
    use crate::layout::value::BorderEdges;

    fn boxf(positioned: bool, z: Option<i32>) -> Fragment {
        let content = FragmentContent::Box {
            background: None,
            border: BorderEdges::default(),
            border_radius: 0.0,
            shadow: None,
            gradient: None,
            transform: None,
        };
        let mut f = Fragment::new(NodeId(1), 0.0, 0.0, 10.0, 10.0, content);
        (f.is_positioned, f.z_index) = (positioned, z);
        f
    }

    #[test]
    fn split_children_paint_at_their_positioned_ancestors_level() {
        assert_eq!(layer_of(&boxf(true, Some(3)), None), Some(Some(3)));
        assert_eq!(layer_of(&boxf(false, None), Some(None)), Some(None));
        // A stacking-context ancestor sets the level of every child.
        let mut child = boxf(true, Some(-1));
        lift(&mut child, Some(2));
        assert_eq!((child.is_positioned, child.z_index), (true, Some(2)));
        // A `z-index: auto` ancestor lifts in-flow children, keeps positioned ones.
        let mut flow = boxf(false, None);
        lift(&mut flow, None);
        assert_eq!((flow.is_positioned, flow.z_index), (true, None));
        let mut own = boxf(true, Some(-1));
        lift(&mut own, None);
        assert_eq!((own.is_positioned, own.z_index), (true, Some(-1)));
    }

    #[test]
    fn a_block_moves_whole_only_if_it_fits_the_next_pages_reserved_capacity() {
        let at = |id: u32, y: f32, h: f32, children: Vec<Fragment>| {
            let mut f = boxf(false, None);
            (f.node_id, f.y, f.height, f.children) = (NodeId(id), y, h, children);
            // Leaves stand for content (`break-inside: avoid`), not empty boxes, which
            // would be sliced at a page end (miaskiewicz/turbo-html2pdf#47).
            f.break_meta.break_inside_avoid = f.children.is_empty();
            f
        };
        let block = at(
            2,
            50.0,
            120.0,
            vec![at(3, 50.0, 60.0, vec![]), at(4, 110.0, 60.0, vec![])],
        );
        let root = at(0, 0.0, 170.0, vec![at(1, 0.0, 50.0, vec![]), block]);
        let boxes = FlowBoxes::default();
        let galley = Galley {
            root: &root,
            wrappers: &[],
            boxes: &boxes,
        };
        // 150px pages; the second reserves 40px (a footnote band): the 120px block does
        // not fit there whole (110px), so it is split at the end of the first page.
        let reserve = |page: usize| if page == 1 { 40.0 } else { 0.0 };
        let walked = walk_reserved(galley, 150.0, &reserve, &mut Diagnostics::default());
        let ids = |p: usize| -> Vec<NodeId> { walked.pages[p].iter().map(|f| f.node_id).collect() };
        assert_eq!(ids(0), [NodeId(1), NodeId(3)]);
        assert_eq!(ids(1), [NodeId(4)]);
    }

    #[test]
    fn remaining_height_that_exactly_fills_a_page_ends_there() {
        let at = |id: u32, y: f32, h: f32, children: Vec<Fragment>| {
            let mut f = boxf(false, None);
            (f.node_id, f.y, f.height, f.children) = (NodeId(id), y, h, children);
            // Leaves stand for content (`break-inside: avoid`), not empty boxes, which
            // would be sliced at a page end (miaskiewicz/turbo-html2pdf#47).
            f.break_meta.break_inside_avoid = f.children.is_empty();
            f
        };
        // 150px pages. The 250px box splits: its second child moves to page 2 (0..100)
        // and its 50px of remaining height end exactly at that page's end; the next
        // block, 10px below the box in the galley, starts page 3 at its top.
        let split = at(
            2,
            0.0,
            250.0,
            vec![at(3, 0.0, 100.0, vec![]), at(4, 100.0, 100.0, vec![])],
        );
        let root = at(0, 0.0, 290.0, vec![split, at(5, 260.0, 30.0, vec![])]);
        let boxes = FlowBoxes::default();
        let galley = Galley {
            root: &root,
            wrappers: &[],
            boxes: &boxes,
        };
        let walked = walk_reserved(galley, 150.0, &|_| 0.0, &mut Diagnostics::default());
        let at_y = |p: usize| -> Vec<(NodeId, f32)> {
            walked.pages[p].iter().map(|f| (f.node_id, f.y)).collect()
        };
        assert_eq!(walked.pages.len(), 3);
        assert_eq!(at_y(1), [(NodeId(4), 0.0)]);
        assert_eq!(at_y(2), [(NodeId(5), 0.0)]);
    }

    #[test]
    fn a_definite_height_counts_each_page_it_breaks_on_to_the_page_end() {
        let at = |id: u32, y: f32, h: f32, children: Vec<Fragment>| {
            let mut f = boxf(false, None);
            (f.node_id, f.y, f.height, f.children) = (NodeId(id), y, h, children);
            // Leaves stand for content (`break-inside: avoid`), not empty boxes, which
            // would be sliced at a page end (miaskiewicz/turbo-html2pdf#47).
            f.break_meta.break_inside_avoid = f.children.is_empty();
            f
        };
        // 150px pages. The 250px box (`height: 250px`) splits: its second child moves to
        // page 2 (0..100); page 1 counts 150px of its height (the 50px gap included), so
        // it ends 100px down page 2 and the next block, 10px below it, follows at 110
        // (miaskiewicz/turbo-html2pdf#43; an auto height ends a page later, above).
        let split = at(
            2,
            0.0,
            250.0,
            vec![at(3, 0.0, 100.0, vec![]), at(4, 100.0, 100.0, vec![])],
        );
        let root = at(0, 0.0, 290.0, vec![split, at(5, 260.0, 30.0, vec![])]);
        let mut boxes = FlowBoxes::default();
        boxes.insert(
            NodeId(2),
            BoxFacts {
                position: crate::layout::value::Position::Static,
                margin_top: 0.0,
                margin_bottom: 0.0,
                padding_top: 0.0,
                padding_bottom: 0.0,
                inset_top: crate::layout::value::LengthPct::Auto,
                inset_bottom: crate::layout::value::LengthPct::Auto,
                shift: 0.0,
                definite_height: true,
            },
        );
        let galley = Galley {
            root: &root,
            wrappers: &[],
            boxes: &boxes,
        };
        let walked = walk_reserved(galley, 150.0, &|_| 0.0, &mut Diagnostics::default());
        let at_y = |p: usize| -> Vec<(NodeId, f32)> {
            walked.pages[p].iter().map(|f| (f.node_id, f.y)).collect()
        };
        assert_eq!(walked.pages.len(), 2);
        assert_eq!(at_y(1), [(NodeId(4), 0.0), (NodeId(5), 110.0)]);
    }
}
