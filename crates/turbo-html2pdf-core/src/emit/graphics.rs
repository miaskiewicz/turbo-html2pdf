//! Box painting (§7) and multi-column `column-rule` painting. A `Box`
//! fragment's background is one filled rect (`re`/`f`); each border side with
//! width > 0 is painted as its own filled rect spanning that side's width band,
//! so asymmetric borders (different widths/colors per side) render correctly.
//! A `ColumnRule` fragment paints as one filled rect (`solid`), a column of
//! Bezier-approximated disks (`dotted`), a column of dashes (`dashed`), or two
//! long-edge bands (`double`) — mirroring turbo-dom-raster's `paint_column_rule`
//! (`paint_png.rs`) one fragment at a time, under a `q`/`Q` save/restore so a
//! future graphics-state change (dash pattern, CTM) cannot leak to the next
//! fragment.
//!
//! **Alpha note**: this painter drops the fill colour's alpha channel on the
//! PDF side (see [`super::color::set_fill`]). A `column-rule-color: rgba(..)`
//! with partial alpha paints alpha-blended on the raster path but solid in
//! the PDF; a GState-based `/ca` pathway for partial alpha is a separate
//! follow-up (R1 emit LOW-1). The width/height/colour-visibility short-circuit
//! at the top of [`paint_column_rule`] still rejects fully transparent rules
//! (alpha 0) on both paths.

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
/// `dotted` stamps anti-aliasable disks of diameter `width` on period
/// `2 * width` as Bezier-approximated circles; `dashed` stamps dashes of length
/// `2 * width` on period `3 * width`; `double` paints two bands each
/// `floor(width / 3)` thick at the long edges. The painter mirrors
/// turbo-dom-raster's `paint_column_rule` so the PDF and the screenshot painter
/// paint one fragment identically. The whole paint sits inside a `q`/`Q`
/// save/restore so a future graphics-state change (dash pattern, CTM) cannot
/// leak into whatever the walker paints next.
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
    content.save_state();
    match style {
        ColumnRuleStyle::Solid => {
            fill_rect(content, &box_rect(frag, page_height_pt), color, cmyk);
        }
        ColumnRuleStyle::Dotted => paint_dotted_rule(content, frag, color, page_height_pt, cmyk),
        ColumnRuleStyle::Dashed => paint_dashed_rule(content, frag, color, page_height_pt, cmyk),
        ColumnRuleStyle::Double => paint_double_rule(content, frag, color, page_height_pt, cmyk),
        // turbo-dom 0.8.2 (#457) added the split-half 3D styles. Chrome 156
        // paints groove/outset as dark|light and ridge/inset as light|dark
        // across the rule's inline axis; until the PDF painter learns the
        // per-side shade, degrade to a solid band so the fragment still
        // paints as its author color.
        ColumnRuleStyle::Groove
        | ColumnRuleStyle::Ridge
        | ColumnRuleStyle::Inset
        | ColumnRuleStyle::Outset => {
            fill_rect(content, &box_rect(frag, page_height_pt), color, cmyk);
        }
    }
    content.restore_state();
}

/// Kappa: the cubic-Bezier control-point offset that approximates a
/// quarter-circle to within ~0.0003 units (the same constant as turbo-dom-raster's
/// `circle_path`, so the PDF and raster disks agree to subpixel accuracy).
const CIRCLE_KAPPA: f32 = 0.552_284_8;

/// Stamp disks of diameter `f.width` down the rule's centerline, period
/// `2 * width`, each disk approximated by four cubic Bezier quadrants — the same
/// construction raster uses (`paint_png.rs::circle_path`). Chrome 156's Blink
/// multicol painter paints `kDottedStroke` with cap-round, i.e. anti-aliased
/// disks; stamping circles here keeps PDF and screenshot paint pixel-identical
/// along the disk perimeter.
fn paint_dotted_rule(
    content: &mut Content,
    f: &Fragment,
    color: Rgba,
    page_height_pt: f32,
    cmyk: bool,
) {
    // `paint_column_rule`'s outer guard already rejects width <= 0 / height <= 0
    // / alpha 0, so `diameter`, `radius` and `period` are all strictly positive
    // whenever this is called — no defensive inner guard needed.
    let diameter = f.width;
    let radius = diameter / 2.0;
    let period = 2.0 * diameter;
    debug_assert!(period > 0.0 && radius > 0.0, "guarded by paint_column_rule");
    set_fill(content, color, cmyk);
    let cx = f.x + radius;
    let mut cy = f.y + radius;
    let end = f.y + f.height;
    // Each dot is centered at `(cx, cy)` with its top at `cy - radius`. The
    // raster painter stamps while `cy - radius < end`; mirror that so the two
    // produce the same count of disks.
    while cy - radius < end {
        emit_circle(content, cx, cy, radius, page_height_pt);
        content.fill_nonzero();
        cy += period;
    }
}

/// Build one disk of radius `r` centered at `(cx, cy)` as four cubic-Bezier
/// quadrants in the PDF content stream. The caller is responsible for the
/// subsequent `f`/`f*` painting operator. `r <= 0` leaves `content` untouched
/// (the caller also short-circuits).
fn emit_circle(content: &mut Content, cx: f32, cy: f32, r: f32, page_height_pt: f32) {
    let k = r * CIRCLE_KAPPA;
    let pt = |px: f32, py: f32| (px_to_pt(px), flip_y(py, page_height_pt));
    let (right, top) = pt(cx + r, cy);
    content.move_to(right, top);
    let curve = |content: &mut Content, c1: (f32, f32), c2: (f32, f32), to: (f32, f32)| {
        let (c1, c2, to) = (pt(c1.0, c1.1), pt(c2.0, c2.1), pt(to.0, to.1));
        content.cubic_to(c1.0, c1.1, c2.0, c2.1, to.0, to.1);
    };
    curve(content, (cx + r, cy + k), (cx + k, cy + r), (cx, cy + r));
    curve(content, (cx - k, cy + r), (cx - r, cy + k), (cx - r, cy));
    curve(content, (cx - r, cy - k), (cx - k, cy - r), (cx, cy - r));
    curve(content, (cx + k, cy - r), (cx + r, cy - k), (cx + r, cy));
    content.close_path();
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
    // `paint_column_rule`'s outer guard already rejects width <= 0, so `period`
    // is strictly positive whenever this is called — no defensive inner guard.
    let width = f.width;
    let period = 3.0 * width;
    debug_assert!(period > 0.0, "guarded by paint_column_rule");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::fragment::{FragmentContent, NodeId};
    use crate::layout::value::Rgba;

    const PAGE_H_PT: f32 = 400.0;

    fn rule_fragment(x: f32, y: f32, w: f32, h: f32, style: ColumnRuleStyle) -> Fragment {
        Fragment::new(
            NodeId(1),
            x,
            y,
            w,
            h,
            FragmentContent::ColumnRule {
                style,
                color: Rgba {
                    r: 0xff,
                    g: 0x80,
                    b: 0x00,
                    a: 0xff,
                },
            },
        )
    }

    fn paint(frag: &Fragment, style: ColumnRuleStyle) -> Vec<u8> {
        let mut content = Content::new();
        paint_column_rule(
            &mut content,
            frag,
            style,
            Rgba {
                r: 0xff,
                g: 0x80,
                b: 0x00,
                a: 0xff,
            },
            PAGE_H_PT,
            false,
        );
        content.finish().to_vec()
    }

    fn count_ops(stream: &[u8], op: &[u8]) -> usize {
        let text = std::str::from_utf8(stream).expect("content stream is ASCII operators");
        text.split_whitespace()
            .filter(|tok| tok.as_bytes() == op)
            .count()
    }

    #[test]
    fn solid_rule_paints_one_rect_wrapped_in_save_restore() {
        // Mirror of the raster painter: one `re` + one `f` fill, inside a `q`/`Q` pair.
        let f = rule_fragment(100.0, 50.0, 4.0, 180.0, ColumnRuleStyle::Solid);
        let stream = paint(&f, ColumnRuleStyle::Solid);
        assert_eq!(count_ops(&stream, b"q"), 1, "save_state missing");
        assert_eq!(count_ops(&stream, b"Q"), 1, "restore_state missing");
        assert_eq!(count_ops(&stream, b"re"), 1, "solid paints one rect");
        assert_eq!(count_ops(&stream, b"f"), 1, "solid fills once");
        // No circles or multiple rects — solid is one filled band.
        assert_eq!(count_ops(&stream, b"c"), 0);
    }

    #[test]
    fn dotted_rule_stamps_bezier_circles_at_period_2w() {
        // 4-px rule, 180 px tall, period 8 px -> dots at cy = 2, 10, 18, …; the raster
        // painter's loop condition is `cy - radius < end`, so the last dot is the one
        // whose top row is still inside the rule. For height 180 and period 8, radius 2:
        // `cy - 2 < 182` => cy up to 181 (cy ∈ {2, 10, …, 178} i.e. 23 dots).
        // Each disk is four cubic-Bezier quadrants: 4 `c` ops per dot + one `f` fill per
        // dot. No `re` ops.
        let f = rule_fragment(100.0, 50.0, 4.0, 180.0, ColumnRuleStyle::Dotted);
        let stream = paint(&f, ColumnRuleStyle::Dotted);
        assert_eq!(count_ops(&stream, b"q"), 1);
        assert_eq!(count_ops(&stream, b"Q"), 1);
        assert_eq!(
            count_ops(&stream, b"re"),
            0,
            "dotted uses circles, not rects"
        );
        let dots = count_ops(&stream, b"f");
        assert!(dots > 1, "dotted stamps multiple disks (got {dots})");
        assert_eq!(
            count_ops(&stream, b"c"),
            dots * 4,
            "4 Bezier quadrants per disk"
        );
    }

    #[test]
    fn dashed_rule_stamps_rect_dashes_at_period_3w() {
        // 4-px rule: period 12 px, dash 8 px. For height 180 → dashes at y = 50..58,
        // 62..70, …; 15 dashes (`dash_top < end` while dash_top ∈ {50, 62, …, 218}).
        let f = rule_fragment(100.0, 50.0, 4.0, 180.0, ColumnRuleStyle::Dashed);
        let stream = paint(&f, ColumnRuleStyle::Dashed);
        assert_eq!(count_ops(&stream, b"q"), 1);
        assert_eq!(count_ops(&stream, b"Q"), 1);
        let dashes = count_ops(&stream, b"re");
        assert!(dashes > 1, "dashed stamps multiple segments (got {dashes})");
        assert_eq!(count_ops(&stream, b"f"), dashes, "one fill per dash");
        assert_eq!(count_ops(&stream, b"c"), 0, "dashed uses rects, not curves");
    }

    #[test]
    fn double_rule_paints_two_long_edge_bands() {
        // 6-px rule paints two 2-px bands at the long edges — two `re`/`f` pairs.
        let f = rule_fragment(100.0, 50.0, 6.0, 180.0, ColumnRuleStyle::Double);
        let stream = paint(&f, ColumnRuleStyle::Double);
        assert_eq!(count_ops(&stream, b"q"), 1);
        assert_eq!(count_ops(&stream, b"Q"), 1);
        assert_eq!(count_ops(&stream, b"re"), 2, "double paints two bands");
        assert_eq!(count_ops(&stream, b"f"), 2);
    }

    #[test]
    fn three_d_rules_degrade_to_a_solid_band() {
        // turbo-dom 0.8.2 (#457) added Groove/Ridge/Inset/Outset; until the PDF
        // painter learns the Chrome split-half shading, each degrades to one
        // filled rect — one `re`/`f` pair inside the save/restore.
        for style in [
            ColumnRuleStyle::Groove,
            ColumnRuleStyle::Ridge,
            ColumnRuleStyle::Inset,
            ColumnRuleStyle::Outset,
        ] {
            let f = rule_fragment(100.0, 50.0, 4.0, 180.0, style);
            let stream = paint(&f, style);
            assert_eq!(count_ops(&stream, b"q"), 1, "{style:?} missing save_state");
            assert_eq!(
                count_ops(&stream, b"Q"),
                1,
                "{style:?} missing restore_state"
            );
            assert_eq!(count_ops(&stream, b"re"), 1, "{style:?} paints one rect");
            assert_eq!(count_ops(&stream, b"f"), 1, "{style:?} fills once");
            assert_eq!(count_ops(&stream, b"c"), 0, "{style:?} uses no curves");
        }
    }

    #[test]
    fn double_rule_under_3px_falls_back_to_one_solid_band() {
        // Matches `paint_png.rs::paint_double_rule`: below 3 px one solid fill.
        let f = rule_fragment(100.0, 50.0, 2.0, 180.0, ColumnRuleStyle::Double);
        let stream = paint(&f, ColumnRuleStyle::Double);
        assert_eq!(count_ops(&stream, b"re"), 1);
        assert_eq!(count_ops(&stream, b"f"), 1);
    }

    #[test]
    fn zero_dim_or_transparent_rule_paints_nothing() {
        // Width 0, height 0 or alpha 0 → the whole call is a no-op (no `q`/`Q` either).
        let mut c = Content::new();
        paint_column_rule(
            &mut c,
            &rule_fragment(100.0, 50.0, 0.0, 180.0, ColumnRuleStyle::Solid),
            ColumnRuleStyle::Solid,
            Rgba {
                r: 0xff,
                g: 0,
                b: 0,
                a: 0xff,
            },
            PAGE_H_PT,
            false,
        );
        assert!(c.finish().as_slice().is_empty());
        let transparent = Rgba {
            r: 0xff,
            g: 0,
            b: 0,
            a: 0,
        };
        let mut c = Content::new();
        paint_column_rule(
            &mut c,
            &rule_fragment(100.0, 50.0, 4.0, 180.0, ColumnRuleStyle::Dotted),
            ColumnRuleStyle::Dotted,
            transparent,
            PAGE_H_PT,
            false,
        );
        assert!(c.finish().as_slice().is_empty());
    }
}
