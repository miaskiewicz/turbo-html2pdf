//! Box painting (§7): a `Box` fragment's background fill and per-side borders.
//! The background is one filled rect (`re`/`f`). Each border side with width > 0
//! is painted as its own filled rect spanning that side's width band, so
//! asymmetric borders (different widths/colors per side) render correctly.

use pdf_writer::Content;

use crate::layout::fragment::Fragment;
use crate::layout::value::{BorderEdges, Rgba};

use super::color::set_fill;
use super::unit::{flip_y, px_to_pt};

/// An axis-aligned rectangle already in PDF user space (points, y-up).
struct PdfRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// Paint one `Box` fragment's background then borders into `content`. `cmyk`
/// selects the device colour space (see [`set_fill`]).
pub fn paint_box(
    content: &mut Content,
    frag: &Fragment,
    background: Option<Rgba>,
    border: &BorderEdges,
    page_height_pt: f32,
    cmyk: bool,
) {
    if let Some(bg) = background {
        fill_rect(content, &box_rect(frag, page_height_pt), bg, cmyk);
    }
    paint_borders(content, frag, border, page_height_pt, cmyk);
}

/// Fill one rectangle with a solid color.
fn fill_rect(content: &mut Content, r: &PdfRect, color: Rgba, cmyk: bool) {
    set_fill(content, color, cmyk);
    content.rect(r.x, r.y, r.w, r.h);
    content.fill_nonzero();
}

/// The fragment's content box as a PDF rect (bottom-left origin, y-flipped).
fn box_rect(frag: &Fragment, page_height_pt: f32) -> PdfRect {
    PdfRect {
        x: px_to_pt(frag.x),
        y: flip_y(frag.y + frag.height, page_height_pt),
        w: px_to_pt(frag.width),
        h: px_to_pt(frag.height),
    }
}

/// Paint every border side whose width is > 0 as a filled band.
fn paint_borders(
    content: &mut Content,
    frag: &Fragment,
    border: &BorderEdges,
    page_height_pt: f32,
    cmyk: bool,
) {
    let outer = box_rect(frag, page_height_pt);
    let sides = [
        (border.top, side_rect(&outer, Edge::Top, border.top.width)),
        (
            border.right,
            side_rect(&outer, Edge::Right, border.right.width),
        ),
        (
            border.bottom,
            side_rect(&outer, Edge::Bottom, border.bottom.width),
        ),
        (
            border.left,
            side_rect(&outer, Edge::Left, border.left.width),
        ),
    ];
    for (spec, rect) in sides {
        if spec.width > 0 {
            fill_rect(content, &rect, spec.color.unwrap_or(Rgba::BLACK), cmyk);
        }
    }
}

/// Which side of the box a border band sits on.
#[derive(Clone, Copy)]
enum Edge {
    Top,
    Right,
    Bottom,
    Left,
}

/// The filled-band rect for one border side: a thin strip of the given pixel
/// width along that edge of the (already PDF-space, y-up) box.
fn side_rect(outer: &PdfRect, edge: Edge, width_px: u16) -> PdfRect {
    let t = px_to_pt(f32::from(width_px));
    match edge {
        Edge::Top => PdfRect {
            x: outer.x,
            y: outer.y + outer.h - t,
            w: outer.w,
            h: t,
        },
        Edge::Bottom => PdfRect {
            x: outer.x,
            y: outer.y,
            w: outer.w,
            h: t,
        },
        Edge::Left => PdfRect {
            x: outer.x,
            y: outer.y,
            w: t,
            h: outer.h,
        },
        Edge::Right => PdfRect {
            x: outer.x + outer.w - t,
            y: outer.y,
            w: t,
            h: outer.h,
        },
    }
}
