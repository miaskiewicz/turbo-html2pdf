//! Box decorations of the dissolved `<html>`/`<body>` boxes, painted on every page
//! fragment (miaskiewicz/turbo-html2pdf#31). The render path dissolves those boxes before
//! pagination so their content flows as the page flow; their backgrounds and borders are
//! re-emitted here as one slice per page, as Chrome 156 prints them under the initial
//! `box-decoration-break: slice`: the box starts at its galley top on the first page and at
//! the page area's top after that, runs to the page area's end on every page but the last,
//! and ends where its content (plus its padding, border and the last child's margin) ends
//! on the last page; the top border edge paints only on the first page, the bottom edge only
//! on the last. A sliced box's corners are square (Chrome rounds only the box's own
//! corners). A wrapper with nothing visible to paint adds nothing, so documents without
//! `<html>`/`<body>` decorations paginate exactly as before.

use crate::layout::fragment::{Fragment, FragmentContent, NodeId};

use super::walk::PageSpan;

/// Whether `f` paints anything of its own: a background colour or gradient, a shadow,
/// or a border side with a width and a colour.
pub(super) fn paints(f: &Fragment) -> bool {
    let side = |s: &crate::layout::value::BorderSide| s.width > 0 && s.color.is_some();
    matches!(
        &f.content,
        FragmentContent::Box { background, border, shadow, gradient, .. }
            if background.is_some()
                || gradient.is_some()
                || shadow.is_some()
                || [&border.top, &border.right, &border.bottom, &border.left].into_iter().any(side)
    )
}

/// Prepend each painting wrapper's slice to every page's body fragments (outermost
/// wrapper first, so it paints under the inner one and under the content). `wrappers`
/// are the dissolved boxes, children already removed, in galley coordinates; `top` is
/// the galley top the first page starts at; `spans` align with `pages`.
pub fn decorate(pages: &mut [Vec<Fragment>], spans: &[PageSpan], wrappers: &[Fragment], top: f32) {
    // The last page with content (a document with none ends where it starts: its
    // wrappers are their galley boxes on the first page).
    let last = spans.iter().rposition(|s| s.last.is_some()).unwrap_or(0);
    let (galley_end, page_end) = spans[last].last.unwrap_or((top, 0.0));
    for (index, (page, span)) in pages.iter_mut().zip(spans).enumerate().take(last + 1) {
        let slices = wrappers.iter().filter(|w| paints(w)).map(|w| {
            let start = if index == 0 { w.y - top } else { 0.0 };
            let end = if index == last {
                (page_end + (w.bottom() - galley_end)).min(span.capacity.max(page_end))
            } else {
                span.capacity
            };
            slice(w, start, end, index == 0, index == last)
        });
        let slices: Vec<Fragment> = slices.collect();
        page.splice(0..0, slices);
    }
}

/// A block split across pages whose box paints (miaskiewicz/turbo-html2pdf#37 item 1):
/// its own paint (children removed), where it starts — page, the index in that page's
/// body where its first fragment went, its page-local border-box top — and where it ends
/// (page, page-local bottom).
#[derive(Debug, Clone)]
pub(super) struct SplitBox {
    pub frag: Fragment,
    pub start: (usize, usize, f32),
    pub end: (usize, f32),
}

/// Slice every split box onto the pages it spans, as Chrome 156 prints them under the
/// initial `box-decoration-break: slice` (the same slices as the `<html>`/`<body>` ones):
/// from its top on its first page (else the page area's top) to its bottom on its last
/// page (else the page area's end). Each slice goes under the box's own content on that
/// page — on the first page at the index its first fragment took, after it at the top —
/// and an enclosing box's slice under an inner one's (`boxes` are in closing order, inner
/// boxes first).
pub(super) fn slice_split_boxes(
    pages: &mut [Vec<Fragment>],
    spans: &[PageSpan],
    boxes: &[SplitBox],
) {
    let mut inserts: Vec<Insert> = boxes
        .iter()
        .enumerate()
        .flat_map(|(order, b)| box_slices(b, order, spans))
        .collect();
    // Back to front per page, so the recorded indices stay valid; at one index, the
    // inner box first, then the enclosing one in front of it.
    inserts.sort_by_key(|i| (i.page, std::cmp::Reverse(i.at), i.order));
    for i in inserts {
        let body = &mut pages[i.page];
        body.insert(i.at.min(body.len()), i.piece);
    }
}

/// One slice to insert: its page, its index in that page's body, its box's closing
/// order, the slice.
struct Insert {
    page: usize,
    at: usize,
    order: usize,
    piece: Fragment,
}

/// The slices of split box `b` (the `order`-th closed) on the pages it spans.
fn box_slices(b: &SplitBox, order: usize, spans: &[PageSpan]) -> Vec<Insert> {
    let ((first, index, top), (last, bottom)) = (b.start, b.end);
    spans[first..=last]
        .iter()
        .zip(first..)
        .map(|(span, page)| {
            let (on_first, on_last) = (page == first, page == last);
            let start = if on_first { top } else { 0.0 };
            let end = if on_last { bottom } else { span.capacity };
            Insert {
                page,
                at: if on_first { index } else { 0 },
                order,
                piece: slice(&b.frag, start, end, on_first, on_last),
            }
        })
        .collect()
}

/// The page-local slice `[start, end)` of wrapper `w`: its own paint only (a synthetic
/// fragment, id 0, so it never carries the wrapper's semantics), with the top / bottom
/// border edge kept only on the first / last slice.
fn slice(w: &Fragment, start: f32, end: f32, first: bool, last: bool) -> Fragment {
    let mut f = w.clone();
    f.node_id = NodeId(0);
    f.children.clear();
    f.y = start;
    f.height = (end - start).max(0.0);
    if let FragmentContent::Box {
        border,
        border_radius,
        ..
    } = &mut f.content
    {
        border.top.width *= u16::from(first);
        border.bottom.width *= u16::from(last);
        *border_radius *= f32::from(u8::from(first && last));
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::value::{BorderEdges, BorderSide, Rgba};

    /// A box fragment at `y` (100px wide, `height` tall) with the given border and radius.
    fn wrapper(y: f32, height: f32, border: BorderEdges, radius: f32) -> Fragment {
        let content = FragmentContent::Box {
            background: None,
            border,
            border_radius: radius,
            shadow: None,
            gradient: None,
            transform: None,
        };
        Fragment::new(NodeId(7), 0.0, y, 100.0, height, content)
    }

    fn all(side: BorderSide) -> BorderEdges {
        BorderEdges {
            top: side,
            right: side,
            bottom: side,
            left: side,
        }
    }

    fn radius(f: &Fragment) -> f32 {
        match &f.content {
            FragmentContent::Box { border_radius, .. } => *border_radius,
            _ => unreachable!("a box"),
        }
    }

    fn widths(f: &Fragment) -> (u16, u16) {
        match &f.content {
            FragmentContent::Box { border, .. } => (border.top.width, border.bottom.width),
            _ => unreachable!("a box"),
        }
    }

    #[test]
    fn only_a_border_with_a_width_and_a_colour_paints() {
        let black = Some(Rgba::BLACK);
        let zero = BorderSide {
            width: 0,
            color: black,
        };
        assert!(!paints(&wrapper(0.0, 10.0, all(zero), 0.0)));
        let colourless = BorderSide {
            width: 2,
            color: None,
        };
        assert!(!paints(&wrapper(0.0, 10.0, all(colourless), 0.0)));
        let one = BorderSide {
            width: 1,
            color: black,
        };
        assert!(paints(&wrapper(0.0, 10.0, all(one), 0.0)));
    }

    #[test]
    fn a_slice_keeps_its_edges_and_corners_only_where_the_box_does() {
        let side = BorderSide {
            width: 3,
            color: Some(Rgba::BLACK),
        };
        let w = wrapper(0.0, 500.0, all(side), 8.0);
        let whole = slice(&w, 5.0, 45.0, true, true);
        assert_eq!(
            (whole.y, whole.height, whole.node_id),
            (5.0, 40.0, NodeId(0))
        );
        assert_eq!((widths(&whole), radius(&whole)), ((3, 3), 8.0));
        let first = slice(&w, 5.0, 45.0, true, false);
        assert_eq!((widths(&first), radius(&first)), ((3, 0), 0.0));
        let middle = slice(&w, 0.0, 45.0, false, false);
        assert_eq!((widths(&middle), radius(&middle)), ((0, 0), 0.0));
        let last = slice(&w, 0.0, 45.0, false, true);
        assert_eq!((widths(&last), radius(&last)), ((0, 3), 0.0));
    }

    #[test]
    fn the_first_slice_starts_at_the_wrapper_below_the_first_page_top() {
        let side = BorderSide {
            width: 1,
            color: Some(Rgba::BLACK),
        };
        // The first page starts at galley y 10; the wrapper's box starts at 30 and ends at
        // 90, with its content ending at galley 80 (page-local 70).
        let w = wrapper(30.0, 60.0, all(side), 0.0);
        let spans = [PageSpan {
            capacity: 300.0,
            last: Some((80.0, 70.0)),
        }];
        let mut pages = vec![Vec::new()];
        decorate(&mut pages, &spans, &[w], 10.0);
        assert_eq!(pages[0].len(), 1);
        assert_eq!((pages[0][0].y, pages[0][0].height), (20.0, 60.0));
    }
}
