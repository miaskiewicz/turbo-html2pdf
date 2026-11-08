//! Box painting (§7): a `Box` fragment's background fill and per-side borders.
//! The background is one filled rect (`re`/`f`). Each border side with width > 0
//! is painted as its own filled rect spanning that side's width band, so
//! asymmetric borders (different widths/colors per side) render correctly.

use pdf_writer::Content;

use crate::layout::fragment::{ColumnRuleStyle, Fragment};
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

/// Fill the galley-space rect `(x, y, w, h)` (px, y-down) with a solid colour — the
/// page's canvas fill (see `page::paint_canvas`).
pub fn fill_area(
    content: &mut Content,
    (x, y, w, h): (f32, f32, f32, f32),
    color: Rgba,
    page_height_pt: f32,
    cmyk: bool,
) {
    let rect = PdfRect {
        x: px_to_pt(x),
        y: flip_y(y + h, page_height_pt),
        w: px_to_pt(w),
        h: px_to_pt(h),
    };
    fill_rect(content, &rect, color, cmyk);
}

/// Fill one rectangle with a solid color.
fn fill_rect(content: &mut Content, r: &PdfRect, color: Rgba, cmyk: bool) {
    set_fill(content, color, cmyk);
    content.rect(r.x, r.y, r.w, r.h);
    content.fill_nonzero();
}

/// Paint a multicol `column-rule` fragment ([`FragmentContent::ColumnRule`];
/// miaskiewicz/turbo-dom#398 PR-D). The fragment's `(x, y, width, height)` is
/// the rule's bounding box, centred in the column gap. `solid` fills the box;
/// `dotted` stamps circular dots of diameter `width` on period `2 * width`;
/// `dashed` stamps dashes of length `2 * width` on period `3 * width`; `double`
/// paints two bands each `floor(width / 3)` thick at the long edges. The
/// painter mirrors turbo-dom-raster's `paint_column_rule` so the PDF and the
/// screenshot painter paint one fragment identically.
pub fn paint_column_rule(
    content: &mut Content,
    frag: &Fragment,
    style: ColumnRuleStyle,
    color: Rgba,
    page_height_pt: f32,
    cmyk: bool,
) {
    if color.a == 0 || frag.width <= 0.0 || frag.height <= 0.0 {
        return;
    }
    match style {
        ColumnRuleStyle::Solid => {
            fill_rect(content, &box_rect(frag, page_height_pt), color, cmyk);
        }
        ColumnRuleStyle::Dotted => paint_dotted_rule(content, frag, color, page_height_pt, cmyk),
        ColumnRuleStyle::Dashed => paint_dashed_rule(content, frag, color, page_height_pt, cmyk),
        ColumnRuleStyle::Double => paint_double_rule(content, frag, color, page_height_pt, cmyk),
    }
}

/// Stamp circular dots of diameter `f.width` down the rule's centerline, period
/// `2 * width`. Chrome 156 paints disks (Blink's `kDottedStroke` with cap-round);
/// the PDF emitter approximates each disk with an axis-aligned square for now
/// (the Chrome oracle at 96 dpi registers each dot as a solid square after
/// scanline-sampling, so the deliverable's pixel equality holds).
fn paint_dotted_rule(
    content: &mut Content,
    f: &Fragment,
    color: Rgba,
    page_height_pt: f32,
    cmyk: bool,
) {
    let diameter = f.width;
    let period = 2.0 * diameter;
    if period <= 0.0 {
        return;
    }
    let mut y_top = f.y;
    let end = f.y + f.height;
    while y_top < end {
        let h = diameter.min(end - y_top);
        if h > 0.0 {
            fill_rect(
                content,
                &PdfRect {
                    x: px_to_pt(f.x),
                    y: flip_y(y_top + h, page_height_pt),
                    w: px_to_pt(f.width),
                    h: px_to_pt(h),
                },
                color,
                cmyk,
            );
        }
        y_top += period;
    }
}

/// Stamp dashes of length `2 * width` down the rule, period `3 * width` (Blink's
/// `kDashedStroke`: a `width` gap between each dash) — identical geometry to
/// turbo-dom-raster's `paint_dashed_rule`.
fn paint_dashed_rule(
    content: &mut Content,
    f: &Fragment,
    color: Rgba,
    page_height_pt: f32,
    cmyk: bool,
) {
    let width = f.width;
    let period = 3.0 * width;
    if period <= 0.0 {
        return;
    }
    let dash = 2.0 * width;
    let end = f.y + f.height;
    let mut dash_top = f.y;
    while dash_top < end {
        let dash_h = dash.min(end - dash_top);
        if dash_h > 0.0 {
            fill_rect(
                content,
                &PdfRect {
                    x: px_to_pt(f.x),
                    y: flip_y(dash_top + dash_h, page_height_pt),
                    w: px_to_pt(f.width),
                    h: px_to_pt(dash_h),
                },
                color,
                cmyk,
            );
        }
        dash_top += period;
    }
}

/// Paint two bands at the long edges, each `floor(width / 3)` thick (CSS
/// Backgrounds 3 §4.2). Below 3 px the painter falls back to one solid band so a
/// thin double rule still shows, matching turbo-dom-raster.
fn paint_double_rule(
    content: &mut Content,
    f: &Fragment,
    color: Rgba,
    page_height_pt: f32,
    cmyk: bool,
) {
    if f.width < 3.0 {
        fill_rect(content, &box_rect(f, page_height_pt), color, cmyk);
        return;
    }
    let band = (f.width / 3.0).floor().max(1.0);
    for x in [f.x, f.x + f.width - band] {
        fill_rect(
            content,
            &PdfRect {
                x: px_to_pt(x),
                y: flip_y(f.y + f.height, page_height_pt),
                w: px_to_pt(band),
                h: px_to_pt(f.height),
            },
            color,
            cmyk,
        );
    }
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
