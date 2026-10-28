//! The paint of a relatively offset block split across pages
//! (miaskiewicz/turbo-html2pdf#42). Chrome 156 offsets a `relative` box in the fragmented
//! flow: the pages it spans are consecutive windows onto one strip (page `q`'s body area
//! starts where page `q - 1`'s ends), so content shifted across a page boundary is cut
//! there and its other part paints on the neighbouring page — the part shifted above a
//! page's area at the end of the previous page, the part shifted past a page's end at the
//! top of the next one. A page after the box's first shows the strip from 1px above its
//! area (Chrome bleeds that row), its first page has no top edge and its last page no
//! bottom edge (the shift paints into the page margins there, as before).

use crate::layout::fragment::{ClipRect, Fragment, NodeId};

use super::walk::PageSpan;

/// The id of a piece of paint carried onto a neighbouring page, and of every fragment in
/// it: a copy of content its own page already carries, so the emitter paints it as an
/// artifact — not tagged, not extracted as text (miaskiewicz/turbo-html2pdf#44).
pub const CARRIED: NodeId = NodeId(u32::MAX);

/// Far enough past any page edge to stand for an unbounded clip side (px).
const FAR: f32 = 1.0e4;

/// The pieces of `f`, painted on page `page` of the shifted box spanning pages
/// `first..=last`: one per page whose window it reaches, translated onto that page and
/// clipped to its window where it does not fit inside it. `page` always gets its piece
/// (clipped to nothing when `f` lies wholly outside its window).
pub(super) fn pieces(
    f: &Fragment,
    page: usize,
    (first, last): (usize, usize),
    spans: &[PageSpan],
) -> Vec<(usize, Fragment)> {
    (first..=last)
        .filter_map(|q| {
            let rows = rows(q, (first, last), spans);
            let piece = piece_on(f, strip_offset(spans, page, q), rows, q == page);
            piece.map(|mut p| {
                if q != page {
                    carried(&mut p);
                }
                (q, p)
            })
        })
        .collect()
}

/// The page-local rows page `q` of a box spanning `first..=last` shows of the strip: from
/// 1px above its area (unbounded on the first page) to its area's end (unbounded on the
/// last).
fn rows(q: usize, (first, last): (usize, usize), spans: &[PageSpan]) -> (f32, f32) {
    let lo = if q > first { -1.0 } else { -FAR };
    let hi = if q < last { spans[q].capacity } else { FAR };
    (lo, hi)
}

/// `f` moved `dy` onto a page showing rows `lo..hi`, clipped to them unless it lies
/// inside; `None` when it does not reach them, unless `keep` (its own page).
fn piece_on(f: &Fragment, dy: f32, (lo, hi): (f32, f32), keep: bool) -> Option<Fragment> {
    let (top, bottom) = (f.y + dy, f.bottom() + dy);
    let reaches = bottom > lo && top < hi;
    (keep || reaches).then(|| {
        let mut piece = f.clone();
        piece.translate(0.0, dy);
        if !(top >= lo && bottom <= hi) {
            piece.clip = Some(window(&piece, lo, hi));
        }
        piece
    })
}

/// Mark `f` and its subtree as [`CARRIED`].
fn carried(f: &mut Fragment) {
    f.node_id = CARRIED;
    f.children.iter_mut().for_each(carried);
}

/// How far a position on page `from` moves when shown on page `to` of the strip: the
/// capacities of the pages between them.
fn strip_offset(spans: &[PageSpan], from: usize, to: usize) -> f32 {
    let between = |a: usize, b: usize| spans[a..b].iter().map(|s| s.capacity).sum::<f32>();
    if to < from {
        between(to, from)
    } else {
        -between(from, to)
    }
}

/// `piece`'s clip region (px from its border-box corner) limited to the page-local rows
/// `lo..hi`, within the clip it already has.
fn window(piece: &Fragment, lo: f32, hi: f32) -> ClipRect {
    let rows = ClipRect {
        x: -FAR,
        y: lo - piece.y,
        width: 2.0 * FAR,
        height: hi - lo,
    };
    piece.clip.map_or(rows, |own| intersect(own, rows))
}

/// The intersection of two clip regions (empty: zero size).
fn intersect(a: ClipRect, b: ClipRect) -> ClipRect {
    let (x0, y0) = (a.x.max(b.x), a.y.max(b.y));
    let (x1, y1) = (
        (a.x + a.width).min(b.x + b.width),
        (a.y + a.height).min(b.y + b.height),
    );
    ClipRect {
        x: x0,
        y: y0,
        width: (x1 - x0).max(0.0),
        height: (y1 - y0).max(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::fragment::{FragmentContent, NodeId};
    use crate::layout::value::BorderEdges;

    fn block(y: f32, height: f32) -> Fragment {
        let content = FragmentContent::Box {
            background: None,
            border: BorderEdges::default(),
            border_radius: 0.0,
            shadow: None,
            gradient: None,
            transform: None,
        };
        Fragment::new(NodeId(1), 10.0, y, 100.0, height, content)
    }

    fn spans(caps: &[f32]) -> Vec<PageSpan> {
        caps.iter()
            .map(|&capacity| PageSpan {
                capacity,
                last: None,
            })
            .collect()
    }

    /// A piece's page, top and clipped rows (page-local).
    type Summary = (usize, f32, Option<(f32, f32)>);

    fn summary(pieces: &[(usize, Fragment)]) -> Vec<Summary> {
        pieces
            .iter()
            .map(|(q, p)| (*q, p.y, p.clip.map(|c| (p.y + c.y, p.y + c.y + c.height))))
            .collect()
    }

    #[test]
    fn content_shifted_above_a_continuation_page_paints_at_the_previous_pages_end() {
        // Page 2 of 320px pages: a block shifted to -20..120 is clipped to -1.. there and
        // its top 20px paint at 300..320 on page 1.
        let got = pieces(&block(-20.0, 140.0), 1, (0, 1), &spans(&[320.0, 320.0]));
        assert_eq!(
            summary(&got),
            [
                (0, 300.0, Some((-FAR, 320.0))),
                (1, -20.0, Some((-1.0, FAR))),
            ]
        );
        // The copy on page 1 (and its subtree) is carried; page 2 keeps its own piece.
        assert_eq!(got[0].1.node_id, CARRIED);
        assert_eq!(got[1].1.node_id, NodeId(1));
    }

    #[test]
    fn a_carried_piece_marks_its_whole_subtree() {
        let mut f = block(-20.0, 140.0);
        let mut child = block(-20.0, 10.0);
        child.children = vec![block(-20.0, 5.0)];
        f.children = vec![child];
        let got = pieces(&f, 1, (0, 1), &spans(&[320.0, 320.0]));
        let ids = |p: &Fragment| {
            (
                p.node_id,
                p.children[0].node_id,
                p.children[0].children[0].node_id,
            )
        };
        assert_eq!(ids(&got[0].1), (CARRIED, CARRIED, CARRIED));
        assert_eq!(ids(&got[1].1), (NodeId(1), NodeId(1), NodeId(1)));
    }

    #[test]
    fn content_shifted_past_a_page_end_paints_at_the_next_pages_top() {
        // Page 1 of three: 20..340 is cut at 320, its last 20px paint from -1 on page 2
        // (300px pages; the box spans pages 1-3, page 3 is out of reach).
        let got = pieces(
            &block(20.0, 320.0),
            0,
            (0, 2),
            &spans(&[320.0, 300.0, 300.0]),
        );
        assert_eq!(
            summary(&got),
            [
                (0, 20.0, Some((-FAR, 320.0))),
                (1, -300.0, Some((-1.0, 300.0))),
            ]
        );
        // From the last page back over two pages: 700px above the top of page 3.
        assert_eq!(strip_offset(&spans(&[400.0, 300.0, 300.0]), 2, 0), 700.0);
        assert_eq!(strip_offset(&spans(&[400.0, 300.0, 300.0]), 0, 2), -700.0);
    }

    #[test]
    fn content_inside_its_window_is_left_alone() {
        let got = pieces(
            &block(0.0, 319.0),
            1,
            (0, 2),
            &spans(&[320.0, 320.0, 320.0]),
        );
        assert_eq!(summary(&got), [(1, 0.0, None)]);
        // Page 2 shows the strip from 1px above its area: a block ending at 319 on page 1
        // stays there; one ending at 320 also shows its last row on page 2 (Chrome's
        // bleed); one ending at 321 is cut at page 1's end.
        let got = pieces(&block(200.0, 119.0), 0, (0, 1), &spans(&[320.0, 320.0]));
        assert_eq!(summary(&got), [(0, 200.0, None)]);
        let got = pieces(&block(200.0, 120.0), 0, (0, 1), &spans(&[320.0, 320.0]));
        assert_eq!(
            summary(&got),
            [(0, 200.0, None), (1, -120.0, Some((-1.0, FAR)))]
        );
        let got = pieces(&block(200.0, 121.0), 0, (0, 1), &spans(&[320.0, 320.0]));
        assert_eq!(
            summary(&got),
            [
                (0, 200.0, Some((-FAR, 320.0))),
                (1, -120.0, Some((-1.0, FAR)))
            ]
        );
        // The same at the top edge of a continuation page: from -1 the block is inside page
        // 2's window, and its first row is page 1's last.
        let got = pieces(&block(-1.0, 50.0), 1, (0, 1), &spans(&[320.0, 320.0]));
        assert_eq!(
            summary(&got),
            [(0, 319.0, Some((-FAR, 320.0))), (1, -1.0, None)]
        );
    }

    #[test]
    fn a_piece_wholly_outside_its_page_window_is_clipped_away_there() {
        // -60..-10 on page 2 shows only on page 1 (260..310); page 2 keeps its piece,
        // clipped to rows it does not reach.
        let got = pieces(&block(-60.0, 50.0), 1, (0, 1), &spans(&[320.0, 320.0]));
        assert_eq!(
            summary(&got),
            [(0, 260.0, None), (1, -60.0, Some((-1.0, FAR)))]
        );
    }

    #[test]
    fn a_window_keeps_inside_the_clip_a_box_already_has() {
        let mut f = block(-20.0, 140.0);
        f.clip = Some(ClipRect {
            x: 5.0,
            y: 10.0,
            width: 30.0,
            height: 40.0,
        });
        let got = pieces(&f, 1, (0, 1), &spans(&[320.0, 320.0]));
        // Page 2: own clip rows -10..30 within -1..: -1..30; page 1: 310..350 within ..320.
        let clip = |i: usize| got[i].1.clip.unwrap();
        assert_eq!(
            (clip(1).x, clip(1).y, clip(1).width, clip(1).height),
            (5.0, 19.0, 30.0, 31.0)
        );
        assert_eq!((clip(0).y, clip(0).height), (10.0, 10.0));
        // Disjoint regions intersect to nothing.
        let none = intersect(
            clip(1),
            ClipRect {
                x: 100.0,
                y: 100.0,
                width: 1.0,
                height: 1.0,
            },
        );
        assert_eq!((none.width, none.height), (0.0, 0.0));
    }
}
