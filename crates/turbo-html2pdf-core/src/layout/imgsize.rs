//! Raster-image box sizing (§7.4, Phase 9b). Turns an image's intrinsic pixel
//! size into a painted box size that fits the page: scaled to preserve aspect
//! ratio and clamped so it never overflows.
//!
//! **Overflow caps (user spec).** Every image is bounded by
//! `max-width = 100%` of the containing block and `max-height ≈ 60%` of the page
//! body height. Because images are never split across pages, the height cap is
//! the mitigation that keeps an oversized image on a single page. When the page
//! body height is unknown at layout time (a region measured without geometry),
//! only the width cap applies — the height clamp is then a no-op hook the caller
//! fills by threading `body_height` (see `ImageCtx`).

use crate::image::Intrinsic;

use super::fragment::ImagePlacement;
use super::value::{BoxStyle, LengthPct};

/// The fraction of the page body height an image may occupy (user spec).
const MAX_HEIGHT_FRACTION: f32 = 0.6;

/// A resolved image box: its painted px size and the placement to emit.
pub struct SizedImage {
    pub width: f32,
    pub height: f32,
    pub placement: ImagePlacement,
}

/// Inputs the sizer needs beyond the intrinsic dimensions.
pub struct SizeCtx<'a> {
    /// The box's resolved style (explicit `width`/`height` override intrinsic).
    pub style: &'a BoxStyle,
    /// Containing-block width — the basis for a `%` `width`/`max-width` and the
    /// 100% width cap.
    pub cb_width: f32,
    /// Containing-block height, if a definite one is known — the basis for a `%`
    /// `height` (e.g. an `<img height:100%>` filling a sized hero card). `None`
    /// leaves a `%` height to fall back to the intrinsic aspect ratio.
    pub cb_height: Option<f32>,
    /// Page body height, if known (the 60% height cap basis).
    pub body_height: Option<f32>,
}

/// Size a replaced `<img>` box from its intrinsic pixel dimensions and the
/// overflow caps, returning the painted box plus the placement to emit.
pub fn size_replaced(name: String, intrinsic: Intrinsic, ctx: &SizeCtx) -> SizedImage {
    let (iw, ih) = (intrinsic.width as f32, intrinsic.height as f32);
    let (base_w, base_h) = base_size(iw, ih, ctx);
    let (width, height) = apply_caps(base_w, base_h, ctx);
    SizedImage {
        width,
        height,
        placement: placement_of(name, intrinsic),
    }
}

/// Live layout has CSS constraints, but no pagination overflow caps.
pub(crate) fn size_replaced_live(name: String, intrinsic: Intrinsic, ctx: &SizeCtx) -> SizedImage {
    let (iw, ih) = (intrinsic.width as f32, intrinsic.height as f32);
    let frame = live_frame(ctx.style);
    let width = live_dimension(ctx.style.width, Some(ctx.cb_width), frame.0);
    let height = live_dimension(ctx.style.height, ctx.cb_height, frame.1);
    let (w, h) = live_base(iw, ih, width, height);
    let limits = live_limits(ctx, frame);
    let (width, height) = constrain_live(w, h, width.is_none(), height.is_none(), limits);
    SizedImage {
        width,
        height,
        placement: placement_of(name, intrinsic),
    }
}

pub(crate) fn size_viewport_live(
    intrinsic_width: Option<f32>,
    intrinsic_height: Option<f32>,
    ratio: Option<f32>,
    layout_scale: f32,
    ctx: &SizeCtx,
) -> (f32, f32) {
    let frame = live_frame(ctx.style);
    let width = live_dimension(ctx.style.width, Some(ctx.cb_width), frame.0)
        .map(|value| viewport_layout_unit(value, layout_scale));
    let height = live_dimension(ctx.style.height, ctx.cb_height, frame.1)
        .map(|value| viewport_layout_unit(value, layout_scale));
    let intrinsic_width = intrinsic_width.map(|value| viewport_layout_unit(value, layout_scale));
    let intrinsic_height = intrinsic_height.map(|value| viewport_layout_unit(value, layout_scale));
    let base = viewport_base(
        width,
        height,
        intrinsic_width,
        intrinsic_height,
        ratio,
        ctx.cb_width,
    );
    let limits = live_limits(ctx, frame);
    let used = if ratio.is_some() {
        constrain_live(base.0, base.1, width.is_none(), height.is_none(), limits)
    } else {
        (
            base.0.clamp(limits.min_w, limits.max_w),
            base.1.clamp(limits.min_h, limits.max_h),
        )
    };
    (
        viewport_layout_unit(used.0, layout_scale),
        viewport_layout_unit(used.1, layout_scale),
    )
}

// Blink155 LayoutUnit stores six fractional bits, saturates and truncates toward
// zero. Convert actual used dimensions before producing a native fragment; the
// caller's measured physical layout scale is separate from JavaScript DPR.
fn viewport_layout_unit(value: f32, scale: f32) -> f32 {
    ((value * scale * 64.0) as i32) as f32 / (scale * 64.0)
}

fn viewport_base(
    width: Option<f32>,
    height: Option<f32>,
    intrinsic_width: Option<f32>,
    intrinsic_height: Option<f32>,
    ratio: Option<f32>,
    containing_width: f32,
) -> (f32, f32) {
    match (width, height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (
            w,
            ratio.map_or(intrinsic_height.unwrap_or(150.0), |r| w / r),
        ),
        (None, Some(h)) => (ratio.map_or(intrinsic_width.unwrap_or(300.0), |r| h * r), h),
        (None, None) => {
            viewport_intrinsic(intrinsic_width, intrinsic_height, ratio, containing_width)
        }
    }
}

fn viewport_intrinsic(
    width: Option<f32>,
    height: Option<f32>,
    ratio: Option<f32>,
    containing_width: f32,
) -> (f32, f32) {
    match (width, height, ratio) {
        (Some(w), Some(h), _) => (w, h),
        (Some(w), None, Some(r)) => (w, w / r),
        (None, Some(h), Some(r)) => (h * r, h),
        (None, None, Some(r)) => (containing_width, containing_width / r),
        _ => (width.unwrap_or(300.0), height.unwrap_or(150.0)),
    }
}

fn live_frame(style: &BoxStyle) -> (f32, f32) {
    if style.box_sizing == super::value::BoxSizing::ContentBox {
        return (0.0, 0.0);
    }
    (
        style.padding.horizontal() + style.border.widths().horizontal(),
        style.padding.vertical() + style.border.widths().vertical(),
    )
}

fn live_dimension(value: LengthPct, basis: Option<f32>, frame: f32) -> Option<f32> {
    resolve_dim(value, basis).map(|v| (v - frame).max(0.0))
}

fn live_base(iw: f32, ih: f32, width: Option<f32>, height: Option<f32>) -> (f32, f32) {
    match (width, height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, scale_other(w, iw, ih)),
        (None, Some(h)) => (scale_other(h, ih, iw), h),
        (None, None) => (iw, ih),
    }
}

struct Limits {
    min_w: f32,
    max_w: f32,
    min_h: f32,
    max_h: f32,
}

fn live_limits(ctx: &SizeCtx, frame: (f32, f32)) -> Limits {
    let s = ctx.style;
    let min_w = live_dimension(s.min_width, Some(ctx.cb_width), frame.0).unwrap_or(0.0);
    let min_h = live_dimension(s.min_height, ctx.cb_height, frame.1).unwrap_or(0.0);
    Limits {
        min_w,
        min_h,
        max_w: live_dimension(s.max_width, Some(ctx.cb_width), frame.0)
            .unwrap_or(f32::INFINITY)
            .max(min_w),
        max_h: live_dimension(s.max_height, ctx.cb_height, frame.1)
            .unwrap_or(f32::INFINITY)
            .max(min_h),
    }
}

fn constrain_live(w: f32, h: f32, auto_w: bool, auto_h: bool, limits: Limits) -> (f32, f32) {
    if auto_w && auto_h {
        return constrain_ratio(w, h, &limits);
    }
    let width = w.clamp(limits.min_w, limits.max_w);
    let height = h.clamp(limits.min_h, limits.max_h);
    match (auto_w, auto_h) {
        (true, false) => (
            scale_other(height, h, w).clamp(limits.min_w, limits.max_w),
            height,
        ),
        (false, true) => (
            width,
            scale_other(width, w, h).clamp(limits.min_h, limits.max_h),
        ),
        _ => (width, height),
    }
}

// CSS 2.2 §10.4: preserve intrinsic ratio for both-auto axes where constraints
// permit it; conflicting width/height constraints can necessarily break it.
fn constrain_ratio(w: f32, h: f32, limits: &Limits) -> (f32, f32) {
    if w <= 0.0 || h <= 0.0 {
        return (
            w.clamp(limits.min_w, limits.max_w),
            h.clamp(limits.min_h, limits.max_h),
        );
    }
    let lower = (limits.min_w / w).max(limits.min_h / h);
    let upper = (limits.max_w / w).min(limits.max_h / h);
    let scale = 1.0_f32.min(upper).max(lower);
    (
        (w * scale).clamp(limits.min_w, limits.max_w),
        (h * scale).clamp(limits.min_h, limits.max_h),
    )
}

/// The placement an intrinsic-sized image emits (the resolver name plus its
/// source size and alpha flag).
pub fn placement_of(name: String, intrinsic: Intrinsic) -> ImagePlacement {
    ImagePlacement {
        name,
        intrinsic_w: intrinsic.width,
        intrinsic_h: intrinsic.height,
        has_alpha: intrinsic.has_alpha,
        tint: None,
    }
}

/// The pre-cap box size: explicit `width`/`height` when set (filling the missing
/// axis from the intrinsic aspect ratio), else the intrinsic pixel size.
fn base_size(iw: f32, ih: f32, ctx: &SizeCtx) -> (f32, f32) {
    // Resolve an explicit `width`/`height` against the containing block (a `%`
    // `width` against `cb_width`, a `%` `height` against `cb_height` when a
    // definite one is known). A `%` with no basis, or `auto`, stays unresolved so
    // the intrinsic aspect ratio fills it in. Previously `%` resolved against 0,
    // collapsing every `width:100%` responsive image to a 0-size box.
    let w = resolve_dim(ctx.style.width, Some(ctx.cb_width));
    let h = resolve_dim(ctx.style.height, ctx.cb_height);
    match (w, h) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, scale_other(w, iw, ih)),
        (None, Some(h)) => (scale_other(h, ih, iw), h),
        (None, None) => (iw, ih),
    }
}

/// A `<length-percentage>`/`auto` image dimension resolved against a basis: a px
/// length is itself, a `%` needs a (definite) basis, and `auto`/an unresolvable
/// `%` yields `None` (the caller falls back to the intrinsic aspect ratio).
fn resolve_dim(v: LengthPct, basis: Option<f32>) -> Option<f32> {
    match v {
        LengthPct::Px(px) => Some(px),
        LengthPct::Pct(p) => basis.map(|b| p / 100.0 * b),
        LengthPct::Calc { pct, px } => basis.map(|b| pct / 100.0 * b + px),
        LengthPct::Auto => None,
    }
}

/// The dependent axis when one axis is fixed at `given`: `given * (other /
/// base)`, preserving aspect ratio. Falls back to `given` for a degenerate
/// (zero) base so the result stays finite.
fn scale_other(given: f32, base: f32, other: f32) -> f32 {
    if base > 0.0 {
        given * other / base
    } else {
        given
    }
}

/// Clamp a box to the width and (when known) height caps, preserving aspect
/// ratio by scaling both axes by the tighter of the two fit ratios.
fn apply_caps(w: f32, h: f32, ctx: &SizeCtx) -> (f32, f32) {
    let max_w = ctx.cb_width;
    let max_h = ctx.body_height.map(|bh| bh * MAX_HEIGHT_FRACTION);
    let scale = fit_scale(w, h, max_w, max_h);
    (w * scale, h * scale)
}

/// The largest uniform scale ≤ 1 that fits `(w, h)` inside the caps. A zero or
/// missing cap dimension imposes no limit on that axis.
fn fit_scale(w: f32, h: f32, max_w: f32, max_h: Option<f32>) -> f32 {
    let sw = axis_scale(w, max_w);
    let sh = max_h.map_or(1.0, |m| axis_scale(h, m));
    sw.min(sh).min(1.0)
}

/// The fit ratio for one axis: `cap / value`, or `1.0` when the value already
/// fits or the cap is non-positive (no limit).
fn axis_scale(value: f32, cap: f32) -> f32 {
    if cap > 0.0 && value > cap {
        cap / value
    } else {
        1.0
    }
}

#[cfg(test)]
mod coverage_tests {
    use super::*;

    #[test]
    fn live_degenerate_intrinsic_dimensions_stay_finite_and_honor_css_minimums() {
        use crate::layout::value::{resolve_box_style, ResolveCtx};
        use crate::ComputedStyle;
        let style = resolve_box_style(
            &ComputedStyle::from_pairs([("min-width", "12px"), ("min-height", "6px")]),
            ResolveCtx {
                parent_font_size: 16.0,
                cb_width: 200.0,
            },
        );
        let ctx = SizeCtx {
            style: &style,
            cb_width: 200.0,
            cb_height: None,
            body_height: None,
        };
        for (width, height, expected) in [
            (0, 0, (12.0, 6.0)),
            (0, 20, (12.0, 20.0)),
            (20, 0, (20.0, 6.0)),
        ] {
            let sized = size_replaced_live(
                "degenerate".into(),
                Intrinsic {
                    width,
                    height,
                    has_alpha: false,
                },
                &ctx,
            );
            assert_eq!((sized.width, sized.height), expected);
            assert!(sized.width.is_finite() && sized.height.is_finite());
        }
    }

    #[test]
    fn resolve_dim_handles_px_percent_calc_and_auto() {
        assert_eq!(resolve_dim(LengthPct::Px(40.0), None), Some(40.0));
        assert_eq!(resolve_dim(LengthPct::Pct(50.0), Some(200.0)), Some(100.0));
        // calc(50% + 10px) against a 200px basis = 110.
        assert_eq!(
            resolve_dim(
                LengthPct::Calc {
                    pct: 50.0,
                    px: 10.0
                },
                Some(200.0)
            ),
            Some(110.0)
        );
        // A calc with no basis (indefinite CB) can't resolve → None.
        assert_eq!(
            resolve_dim(
                LengthPct::Calc {
                    pct: 50.0,
                    px: 10.0
                },
                None
            ),
            None
        );
        assert_eq!(resolve_dim(LengthPct::Auto, Some(200.0)), None);
    }
}
