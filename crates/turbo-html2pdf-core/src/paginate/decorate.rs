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
fn paints(f: &Fragment) -> bool {
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
