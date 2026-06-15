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
use super::value::BoxStyle;

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
    /// Containing-block width (the 100% width cap basis).
    pub cb_width: f32,
    /// Page body height, if known (the 60% height cap basis).
    pub body_height: Option<f32>,
}

/// Size a replaced `<img>` box from its intrinsic pixel dimensions and the
/// overflow caps, returning the painted box plus the placement to emit.
pub fn size_replaced(name: String, intrinsic: Intrinsic, ctx: &SizeCtx) -> SizedImage {
    let (iw, ih) = (intrinsic.width as f32, intrinsic.height as f32);
    let (base_w, base_h) = base_size(iw, ih, ctx.style);
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
    }
}

/// The pre-cap box size: explicit `width`/`height` when set (filling the missing
/// axis from the intrinsic aspect ratio), else the intrinsic pixel size.
fn base_size(iw: f32, ih: f32, style: &BoxStyle) -> (f32, f32) {
    let w = style.width.resolve(0.0);
    let h = style.height.resolve(0.0);
    match (w, h) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, scale_other(w, iw, ih)),
        (None, Some(h)) => (scale_other(h, ih, iw), h),
        (None, None) => (iw, ih),
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
