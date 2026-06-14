//! Footnotes and the body/footnote fixpoint (§3.6, §6.4).
//!
//! A `<t:footnote>` puts a numbered marker inline and moves its body to the
//! footnote area of whatever page the marker lands on *after pagination*
//! (AC-3.15) — a content-driven, mutual constraint: the notes a page references
//! reserve area at its bottom, which shrinks its body capacity, which can change
//! which markers land on it. We resolve it by a damped fixpoint (§6.4): paginate
//! with the previous pass's reservation, recompute the reservation from where the
//! markers actually landed, and repeat until the per-page reservation stops
//! changing or a cap of [`MAX_ITERS`] passes is hit (then we accept the last pass
//! and lint [`LintCode::FootnoteConvergence`], never dropping content).
//!
//! The footnote area sits above the bottom margin/footer band and grows upward as
//! notes accumulate; a separator rule heads it (§3.19). A note taller than the
//! whole area continues onto the next page's area (§3.18, AC-3.18) so content is
//! never lost.
//!
//! Endnotes (`<t:endnote>`/`<t:endnotes/>`) are a separate, feature-gated flow —
//! TODO(phase15): they collect to a document-end section rather than per page.

use crate::error::{Diagnostics, LintCode, Span};
use crate::layout::fragment::{Fragment, FragmentContent, NodeId};
use crate::layout::value::Rgba;

use super::geometry::PageGeometry;
use super::walk::walk_reserved;

/// Hard cap on fixpoint passes (§6.4, K=4). Beyond this we accept the current
/// placement and lint rather than loop forever.
pub const MAX_ITERS: usize = 4;

/// The gap above the separator rule and below it before the first note, and the
/// stacking gap between notes, in px. Small and fixed so the reserved band is
/// predictable.
const SEPARATOR_GAP: f32 = 4.0;
/// The separator rule's own thickness.
const SEPARATOR_RULE: f32 = 1.0;

/// One resolved footnote: its document-order index and its laid-out body as a
/// flat stack of text-line fragments (mark prefix included), each carrying its
/// own height so an oversized note can be split line-by-line across pages.
#[derive(Debug, Clone)]
pub struct Note {
    pub index: usize,
    lines: Vec<Fragment>,
    pub height: f32,
}

impl Note {
    /// Build a note from its laid-out body galley, flattening it to its text
    /// lines (the galley nests the synthetic `<p>` above its lines).
    pub fn new(index: usize, galley: Fragment) -> Note {
        let mut lines = Vec::new();
        flatten_lines(&galley, &mut lines);
        let height = lines.iter().map(|l| l.height).sum();
        Note {
            index,
            lines,
            height,
        }
    }

    /// This note's text lines, each a positioned fragment.
    fn lines(&self) -> &[Fragment] {
        &self.lines
    }
}

/// Collect the text-line leaves of a galley subtree in top-down order.
fn flatten_lines(frag: &Fragment, out: &mut Vec<Fragment>) {
    if matches!(frag.content, FragmentContent::TextLine { .. }) {
        out.push(frag.clone());
    }
    for child in &frag.children {
        flatten_lines(child, out);
    }
}

/// The footnote-area band placed on one page: the separator (if any notes) plus
/// the note line fragments, in page-local coordinates measured from the band top.
#[derive(Debug, Clone, Default)]
pub struct FootnoteBand {
    /// The painted fragments (separator rule + note lines), band-local.
    pub fragments: Vec<Fragment>,
    /// The band's total reserved height (0 when the page references no notes).
    pub height: f32,
}

/// The full result of resolving footnotes: one band per page, aligned with the
/// page list, plus whether the fixpoint converged.
pub struct Resolved {
    pub bands: Vec<FootnoteBand>,
    pub pages: Vec<Vec<Fragment>>,
}

/// Run the body/footnote fixpoint (§6.4). Returns the paginated body together
/// with each page's footnote band. With no notes this degenerates to a single
/// zero-reservation walk.
pub fn resolve(
    root: &Fragment,
    geometry: PageGeometry,
    notes: &[Note],
    diags: &mut Diagnostics,
) -> Resolved {
    let base = geometry.body_height();
    let mut reservation: Vec<f32> = Vec::new();
    let mut pages = walk_pass(root, base, &reservation, diags);
    for _ in 0..MAX_ITERS {
        let next = measure_reservation(&pages, notes, base);
        if next == reservation {
            return assemble(pages, base, notes);
        }
        reservation = next;
        pages = walk_pass(root, base, &reservation, diags);
    }
    let next = measure_reservation(&pages, notes, base);
    if next != reservation {
        diags.push(
            LintCode::FootnoteConvergence,
            "footnote/body layout did not converge within the iteration cap",
            Span::default(),
        );
    }
    assemble(pages, base, notes)
}

/// One pagination pass against a per-page reservation lookup.
fn walk_pass(
    root: &Fragment,
    base: f32,
    reservation: &[f32],
    diags: &mut Diagnostics,
) -> Vec<Vec<Fragment>> {
    let reserve = |page: usize| reservation.get(page).copied().unwrap_or(0.0);
    walk_reserved(root, base, &reserve, diags)
}

/// The footnote indices a page references, in document order, read off the
/// `BreakMeta.footnotes` of the markers that landed on it.
fn owned_indices(page: &[Fragment]) -> Vec<usize> {
    let mut out = Vec::new();
    for frag in page {
        collect_indices(frag, &mut out);
    }
    out
}

/// Accumulate a fragment subtree's footnote-marker indices in pre-order.
fn collect_indices(frag: &Fragment, out: &mut Vec<usize>) {
    out.extend(&frag.break_meta.footnotes);
    for child in &frag.children {
        collect_indices(child, out);
    }
}

/// The notes a page owns, looked up by index (in order).
fn notes_on<'a>(page: &[Fragment], notes: &'a [Note]) -> Vec<&'a Note> {
    owned_indices(page)
        .into_iter()
        .filter_map(|i| notes.iter().find(|n| n.index == i))
        .collect()
}

/// Compute each page's reserved footnote area for the next pass: the height the
/// page's notes occupy, capped at `base` so a page never reserves more than its
/// whole body (the overflow continues to the next page, §3.18).
fn measure_reservation(pages: &[Vec<Fragment>], notes: &[Note], base: f32) -> Vec<f32> {
    let mut carry = 0.0_f32;
    pages
        .iter()
        .map(|page| page_reservation(page, notes, base, &mut carry))
        .collect()
}

/// The reservation for one page: any carried-over continuation height plus this
/// page's own notes, capped at `base`; the excess becomes the next page's carry.
fn page_reservation(page: &[Fragment], notes: &[Note], base: f32, carry: &mut f32) -> f32 {
    let own = stacked_height(&notes_on(page, notes));
    let want = *carry + own;
    if want <= 0.0 {
        *carry = 0.0;
        return 0.0;
    }
    let band = band_height(want);
    if band <= base {
        *carry = 0.0;
        band
    } else {
        *carry = want - content_room(base);
        base
    }
}

/// The stacked content height of a set of notes (galley heights plus inter-note
/// gaps); 0 for an empty set.
fn stacked_height(notes: &[&Note]) -> f32 {
    if notes.is_empty() {
        return 0.0;
    }
    let bodies: f32 = notes.iter().map(|n| n.height).sum();
    let gaps = SEPARATOR_GAP * (notes.len() - 1) as f32;
    bodies + gaps
}

/// The full band height for `content` px of notes: the separator furniture plus
/// the content.
fn band_height(content: f32) -> f32 {
    SEPARATOR_GAP + SEPARATOR_RULE + SEPARATOR_GAP + content
}

/// The note content room inside a band of the maximum height `base`.
fn content_room(base: f32) -> f32 {
    (base - (SEPARATOR_GAP + SEPARATOR_RULE + SEPARATOR_GAP)).max(0.0)
}

// --------------------------------------------------------------------------
// band assembly
// --------------------------------------------------------------------------

/// Build the painted footnote band for each page from the converged placement,
/// splitting any note that overruns the band onto the next page (§3.18).
fn assemble(pages: Vec<Vec<Fragment>>, base: f32, notes: &[Note]) -> Resolved {
    let mut carry: Vec<Fragment> = Vec::new();
    let mut bands = Vec::with_capacity(pages.len());
    for page in &pages {
        let mut lines = std::mem::take(&mut carry);
        lines.extend(page_lines(page, notes));
        bands.push(build_band(&mut lines, content_room(base), &mut carry));
    }
    Resolved { bands, pages }
}

/// The note lines a page owns, flattened in document order with an inter-note
/// gap baked into each note's first line offset.
fn page_lines(page: &[Fragment], notes: &[Note]) -> Vec<Fragment> {
    let mut out = Vec::new();
    for note in notes_on(page, notes) {
        for line in note.lines() {
            out.push(line.clone());
        }
    }
    out
}

/// Lay `lines` into a band no taller than `room`, carrying the overflow lines
/// into `carry` for the next page. Empty `lines` yields an empty band.
fn build_band(lines: &mut Vec<Fragment>, room: f32, carry: &mut Vec<Fragment>) -> FootnoteBand {
    if lines.is_empty() {
        return FootnoteBand::default();
    }
    let take = lines_fitting(lines, room);
    let rest = lines.split_off(take.max(1));
    carry.extend(rest);
    paint_band(std::mem::take(lines))
}

/// How many leading `lines` fit in `room` px of note content, stacked top-down.
fn lines_fitting(lines: &[Fragment], room: f32) -> usize {
    let mut used = 0.0_f32;
    let mut count = 0;
    for line in lines {
        if used + line.height > room {
            break;
        }
        used += line.height;
        count += 1;
    }
    count
}

/// Paint a band from the lines that stay on this page: a separator rule on top,
/// then the note lines stacked beneath it; report the band's total height.
fn paint_band(lines: Vec<Fragment>) -> FootnoteBand {
    let mut fragments = vec![separator_rule()];
    let mut y = SEPARATOR_GAP + SEPARATOR_RULE + SEPARATOR_GAP;
    let mut content = 0.0_f32;
    for mut line in lines {
        let h = line.height;
        line.x = 0.0;
        line.y = y;
        fragments.push(line);
        y += h;
        content += h;
    }
    FootnoteBand {
        fragments,
        height: band_height(content),
    }
}

/// The default separator: a thin full-width-ish rule, painted as a bordered box
/// of the rule thickness. Width is left to the band placement to translate.
fn separator_rule() -> Fragment {
    Fragment::new(
        NodeId(0),
        0.0,
        SEPARATOR_GAP,
        1.0,
        SEPARATOR_RULE,
        FragmentContent::Box {
            background: Some(Rgba::BLACK),
            border: crate::layout::value::BorderEdges::default(),
        },
    )
}
