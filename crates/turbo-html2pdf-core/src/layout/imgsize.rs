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
