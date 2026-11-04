//! Per-page content stream (§7). Walks every band of a [`Page`] in galley order,
//! painting box backgrounds/borders and text lines into a single content stream.
//! Bands beyond the body (header/footer/footnotes) are empty until Phases 7/8,
//! but we iterate them so they paint the moment they fill.
//!
//! Under the `pdf-ua` feature each painted fragment is wrapped in marked content
//! (`/Tag <</MCID n>> BDC … EMC`) so it can be linked into the document's
//! `StructTreeRoot`; decorative paints (box backgrounds/borders, the watermark,
//! running header/footer chrome) are marked `/Artifact` so assistive tech skips
//! them (AC-11.1).

use std::cell::Cell;
use std::collections::BTreeSet;

use pdf_writer::{Content, Name, TextStr};

use crate::layout::fragment::{Fragment, FragmentContent, TilePattern};
use crate::paginate::{Page, CARRIED};

use super::fonts::FontStore;
use super::graphics::{fill_area, paint_box};
use super::image::{paint_image, ImageStore};
use super::text::paint_text;
use super::unit::px_to_pt;
use super::watermark::{self, Watermark};

/// The painter context threaded through a page's fragments: the resource stores,
/// the page height (points) used for the galley→PDF y-flip, and the render's
/// `cmyk` colour-space flag (see [`super::color::set_fill`]).
struct PaintCtx<'a> {
    fonts: &'a FontStore,
    images: &'a ImageStore,
    page_height_pt: f32,
    cmyk: bool,
    /// Whether CSS `opacity` paints translucent (off for PDF/A, which emits no `/ca`).
    fade: bool,
    /// The opacity in effect: the product of the painted ancestors' opacities.
    alpha: Cell<f32>,
}

/// Build the content-stream bytes for one page. A watermark, when present, is
/// painted first so the body bands draw on top of it (behind-body ordering).
/// `cmyk` selects the device colour space; `fade` is whether the watermark's
/// `/ca` transparency fade is emitted (suppressed for a PDF/A render).
///
/// Under the `pdf-ua` feature, `tags` is `Some` only when this render emits
/// tagged PDF (`opts.pdf_ua`): then the painter brackets content in marked
/// content (`BDC`/`EMC`). When `tags` is `None` — a flag-off render — the page
/// is painted plain, byte-for-byte the untagged stream.
#[allow(clippy::too_many_arguments)]
pub fn content_stream(
    page: &Page,
    fonts: &FontStore,
    images: &ImageStore,
    watermark: Option<&Watermark>,
    cmyk: bool,
    fade: bool,
    #[cfg(feature = "pdf-ua")] tags: Option<&super::ua::PageTags>,
) -> Vec<u8> {
    let ctx = PaintCtx {
        fonts,
        images,
        page_height_pt: px_to_pt(page.geometry.height),
        cmyk,
        fade,
        alpha: Cell::new(1.0),
    };
    let mut content = Content::new();
    #[cfg(feature = "pdf-ua")]
    let mut marker = tags.map(|t| super::ua::Marker::new(t, &page.semantics));
    paint_canvas(
        &mut content,
        page,
        &ctx,
        #[cfg(feature = "pdf-ua")]
        marker.is_some(),
    );
    paint_watermark(
        &mut content,
        page,
        watermark,
        fonts,
        images,
        cmyk,
        fade,
        #[cfg(feature = "pdf-ua")]
        marker.is_some(),
    );
    paint_bands(
        &mut content,
        page,
        &ctx,
        #[cfg(feature = "pdf-ua")]
        marker.as_mut(),
    );
    content.finish().to_vec()
}

/// Paint the canvas colour (the document's propagated root/`<body>` background,
/// miaskiewicz/turbo-html2pdf#29) over the page area — inside the page margins, the
/// area the document is laid out in — behind everything else, composited onto the
/// white sheet. A tagged render marks it `/Artifact` (decoration).
fn paint_canvas(
    content: &mut Content,
    page: &Page,
    ctx: &PaintCtx,
    #[cfg(feature = "pdf-ua")] tagged: bool,
) {
    let Some(canvas) = page.canvas else {
        return;
    };
    let g = &page.geometry;
    let area = (
        g.margin.left,
        g.margin.top,
        g.width - g.margin.left - g.margin.right,
        g.height - g.margin.top - g.margin.bottom,
    );
    #[cfg(feature = "pdf-ua")]
    if tagged {
        super::ua::begin_artifact(content);
    }
    fill_area(
        content,
        area,
        over_white(canvas),
        ctx.page_height_pt,
        ctx.cmyk,
    );
    #[cfg(feature = "pdf-ua")]
    if tagged {
        content.end_marked_content();
    }
}

/// `c` composited over the white sheet (source-over, straight alpha), opaque.
fn over_white(c: crate::layout::value::Rgba) -> crate::layout::value::Rgba {
    let a = u32::from(c.a);
    let mix = |v: u8| ((u32::from(v) * a + 255 * (255 - a) + 127) / 255) as u8;
    crate::layout::value::Rgba {
        r: mix(c.r),
        g: mix(c.g),
        b: mix(c.b),
        a: 255,
    }
}

/// Paint the page watermark behind the body. When this render emits tagged PDF
/// (`tagged`), the watermark is wrapped as an `/Artifact` so assistive tech skips
/// it. `cmyk` selects the colour space; `fade` enables the `/ca` transparency
/// fade (off for a PDF/A render).
#[allow(clippy::too_many_arguments)]
fn paint_watermark(
    content: &mut Content,
    page: &Page,
    watermark: Option<&Watermark>,
    fonts: &FontStore,
    images: &ImageStore,
    cmyk: bool,
    fade: bool,
    #[cfg(feature = "pdf-ua")] tagged: bool,
) {
    let Some(mark) = watermark else {
        return;
    };
    #[cfg(feature = "pdf-ua")]
    super::ua::begin_watermark_artifact(content, tagged);
    watermark::paint(content, mark, page, fonts, images, cmyk, fade);
    #[cfg(feature = "pdf-ua")]
    super::ua::end_watermark_artifact(content, tagged);
}

/// `frags` (siblings) in paint order: [`Fragment::paint_order`]'s stable sort by
/// stacking level — negative `z-index`, in-flow boxes, positioned boxes, positive
/// `z-index`; source order within a level (miaskiewicz/turbo-html2pdf#35).
pub(super) fn painted(frags: &[Fragment]) -> Vec<&Fragment> {
    let mut sorted: Vec<&Fragment> = frags.iter().collect();
    sorted.sort_by_key(|f| (f.paint_z(), f.is_positioned));
    sorted
}

/// Paint every band of a page in back-to-front order. Without `pdf-ua` the band
/// identity is irrelevant; with a tagged render the header/footer bands paint as
/// artifacts.
#[cfg(not(feature = "pdf-ua"))]
fn paint_bands(content: &mut Content, page: &Page, ctx: &PaintCtx) {
    for band in bands(page) {
        for frag in painted(band) {
            paint_fragment(content, frag, ctx);
        }
    }
}

/// `pdf-ua` variant: thread an optional marked-content marker. `Some` tags each
/// band; `None` paints plain (byte-for-byte the untagged stream).
#[cfg(feature = "pdf-ua")]
fn paint_bands(
    content: &mut Content,
    page: &Page,
    ctx: &PaintCtx,
    mut marker: Option<&mut super::ua::Marker>,
) {
    for (index, band) in bands(page).into_iter().enumerate() {
        for frag in painted(band) {
            // A table header row repeated on this page paints as an artifact.
            let repeat = index == 0 && marker.as_deref().is_some_and(|m| m.repeats(frag));
            paint_fragment(
                content,
                frag,
                ctx,
                marker.as_deref_mut(),
                is_artifact_band(index) || repeat,
            );
        }
    }
}

/// Whether band `i` (in [`bands`] order: body, header, footer, footnotes) is a
/// pagination artifact: the running header/footer chrome is skipped by assistive
/// tech; the body and footnotes carry the document's tagged content.
#[cfg(feature = "pdf-ua")]
fn is_artifact_band(i: usize) -> bool {
    i == 1 || i == 2
}

/// The four paint bands in back-to-front order.
fn bands(page: &Page) -> [&[Fragment]; 4] {
    [&page.body, &page.header, &page.footer, &page.footnotes]
}

/// Paint one fragment under its CSS `opacity` (miaskiewicz/turbo-html2pdf#55): an
/// `opacity: 0` subtree is not painted at all (as Chrome 156 `page.pdf()`); a translucent
/// one paints in a saved graphics state whose `ExtGState` sets `/ca` and `/CA` to its
/// effective opacity (nested opacities multiply).
fn paint_fragment(
    content: &mut Content,
    frag: &Fragment,
    ctx: &PaintCtx,
    #[cfg(feature = "pdf-ua")] marker: Option<&mut super::ua::Marker>,
    #[cfg(feature = "pdf-ua")] artifact_band: bool,
) {
    if invisible(frag) {
        return;
    }
    let outer = begin_opacity(content, frag, ctx);
    paint_clipped(
        content,
        frag,
        ctx,
        #[cfg(feature = "pdf-ua")]
        marker,
        #[cfg(feature = "pdf-ua")]
        artifact_band,
    );
    end_opacity(content, ctx, outer);
}

/// Whether `frag`'s subtree paints nothing (`opacity: 0`).
pub(super) fn invisible(frag: &Fragment) -> bool {
    frag.opacity <= 0.0
}

/// Enter `frag`'s opacity, returning the outer opacity to restore, or `None` when the
/// fragment is opaque or the render emits no transparency.
fn begin_opacity(content: &mut Content, frag: &Fragment, ctx: &PaintCtx) -> Option<f32> {
    if frag.opacity >= 1.0 || !ctx.fade {
        return None;
    }
    let outer = ctx.alpha.get();
    let key = alpha_key(outer * frag.opacity);
    ctx.alpha.set(outer * frag.opacity);
    content.save_state();
    content.set_parameters(Name(opacity_gs_name(key).as_bytes()));
    Some(outer)
}

/// Leave the opacity [`begin_opacity`] entered.
fn end_opacity(content: &mut Content, ctx: &PaintCtx, outer: Option<f32>) {
    if let Some(outer) = outer {
        content.restore_state();
        ctx.alpha.set(outer);
    }
}

/// An opacity in thousandths, the key of its `ExtGState` (`0.5` -> `500`).
fn alpha_key(alpha: f32) -> u16 {
    (alpha * 1000.0).round() as u16
}

/// The `ExtGState` resource name of an opacity key (`GSo500`).
pub(super) fn opacity_gs_name(key: u16) -> String {
    format!("GSo{key}")
}

/// The opacity keys a page's translucent fragments paint with, each written as one
/// `ExtGState` of the page's resources.
pub(super) fn opacity_keys(page: &Page) -> BTreeSet<u16> {
    let mut keys = BTreeSet::new();
    for band in bands(page) {
        for frag in band {
            collect_opacity(frag, 1.0, &mut keys);
        }
    }
    keys
}

fn collect_opacity(frag: &Fragment, outer: f32, keys: &mut BTreeSet<u16>) {
    if invisible(frag) {
        return;
    }
    let alpha = outer * frag.opacity.min(1.0);
    if frag.opacity < 1.0 {
        keys.insert(alpha_key(alpha));
    }
    for child in &frag.children {
        collect_opacity(child, alpha, keys);
    }
}

/// Paint one fragment (its own content first, then its children atop it, in paint
/// order), inside its `clip: rect()` region when it has one (miaskiewicz/turbo-dom#52): a
/// saved graphics state with a rectangular clipping path, rounded when the region has
/// corner radii. An empty region still paints
/// (nothing shows), so tagged renders number their marked content the same as the
/// structure walk.
fn paint_clipped(
    content: &mut Content,
    frag: &Fragment,
    ctx: &PaintCtx,
    #[cfg(feature = "pdf-ua")] mut marker: Option<&mut super::ua::Marker>,
    #[cfg(feature = "pdf-ua")] artifact_band: bool,
) {
    if let Some(clip) = frag.clip {
        content.save_state();
        clip_to(content, frag, clip, ctx.page_height_pt);
    }
    paint_one(
        content,
        frag,
        ctx,
        #[cfg(feature = "pdf-ua")]
        marker.as_deref_mut(),
        #[cfg(feature = "pdf-ua")]
        artifact_band,
    );
    for child in painted(&frag.children) {
        paint_fragment(
            content,
            child,
            ctx,
            #[cfg(feature = "pdf-ua")]
            marker.as_deref_mut(),
            #[cfg(feature = "pdf-ua")]
            artifact_band,
        );
    }
    if frag.clip.is_some() {
        content.restore_state();
    }
}

/// Intersect the clipping path with `frag`'s clip region (px from its border-box corner).
fn clip_to(
    content: &mut Content,
    frag: &Fragment,
    clip: crate::layout::fragment::ClipRect,
    page_height_pt: f32,
) {
    if clip.radii.is_rounded() {
        rounded_path(
            content,
            frag.x + clip.x,
            frag.y + clip.y,
            clip,
            page_height_pt,
        );
    } else {
        content.rect(
            px_to_pt(frag.x + clip.x),
            super::unit::flip_y(frag.y + clip.y + clip.height, page_height_pt),
            px_to_pt(clip.width),
            px_to_pt(clip.height),
        );
    }
    content.clip_nonzero();
    content.end_path();
}

/// The outline of `clip` placed at page px (`x`, `y`): a rounded rectangle whose
/// elliptical corners (miaskiewicz/turbo-dom#248/#253, a rounded box's background tiles)
/// are cubic Béziers, as Chrome clips the tiles to the box's rounded corners.
fn rounded_path(
    content: &mut Content,
    x: f32,
    y: f32,
    clip: crate::layout::fragment::ClipRect,
    page_height_pt: f32,
) {
    // The control-point distance of a quarter ellipse, as a fraction of its radius.
    const K: f32 = 0.552_284_8;
    let pt = |px: f32, py: f32| (px_to_pt(px), super::unit::flip_y(py, page_height_pt));
    let (w, h) = (clip.width, clip.height);
    let [tl, tr, br, bl] = clip.radii.corners();
    let (r, b) = (x + w, y + h);
    let start = pt(x + tl.h, y);
    content.move_to(start.0, start.1);
    let line = |content: &mut Content, px: f32, py: f32| {
        let p = pt(px, py);
        content.line_to(p.0, p.1);
    };
    let curve = |content: &mut Content, c1: (f32, f32), c2: (f32, f32), to: (f32, f32)| {
        let (c1, c2, to) = (pt(c1.0, c1.1), pt(c2.0, c2.1), pt(to.0, to.1));
        content.cubic_to(c1.0, c1.1, c2.0, c2.1, to.0, to.1);
    };
    line(content, r - tr.h, y);
    curve(
        content,
        (r - tr.h * (1.0 - K), y),
        (r, y + tr.v * (1.0 - K)),
        (r, y + tr.v),
    );
    line(content, r, b - br.v);
    curve(
        content,
        (r, b - br.v * (1.0 - K)),
        (r - br.h * (1.0 - K), b),
        (r - br.h, b),
    );
    line(content, x + bl.h, b);
    curve(
        content,
        (x + bl.h * (1.0 - K), b),
        (x, b - bl.v * (1.0 - K)),
        (x, b - bl.v),
    );
    line(content, x, y + tl.v);
    curve(
        content,
        (x, y + tl.v * (1.0 - K)),
        (x + tl.h * (1.0 - K), y),
        (x + tl.h, y),
    );
    content.close_path();
}

/// Paint a single fragment's own content (no marked content).
#[cfg(not(feature = "pdf-ua"))]
fn paint_one(content: &mut Content, frag: &Fragment, ctx: &PaintCtx) {
    paint_content(content, frag, ctx);
}

/// `pdf-ua` variant: when a `marker` is present (tagged render) bracket each
/// real-content paint with `BDC`/`EMC` (an MCID for content, `/Artifact` for
/// decoration); with no marker, paint plain — byte-for-byte the untagged stream.
#[cfg(feature = "pdf-ua")]
fn paint_one(
    content: &mut Content,
    frag: &Fragment,
    ctx: &PaintCtx,
    marker: Option<&mut super::ua::Marker>,
    artifact_band: bool,
) {
    let Some(marker) = marker else {
        paint_content(content, frag, ctx);
        return;
    };
    match marker.wrap(frag, artifact_band) {
        super::ua::WrapKind::None => paint_content(content, frag, ctx),
        super::ua::WrapKind::Artifact => {
            super::ua::begin_artifact(content);
            paint_content(content, frag, ctx);
            content.end_marked_content();
        }
        super::ua::WrapKind::Content => {
            super::ua::begin_mcid(content, marker.next());
            paint_content(content, frag, ctx);
            content.end_marked_content();
        }
    }
}

/// Paint a text line. A line carried onto a neighbouring page ([`CARRIED`]) repeats text
/// its own page carries: it is shown inside a `/Span` whose empty `/ActualText` replaces
/// it for text extraction, so the text is extracted once (miaskiewicz/turbo-html2pdf#44).
fn paint_line(content: &mut Content, frag: &Fragment, ctx: &PaintCtx) {
    let carried = frag.node_id == CARRIED;
    if carried {
        content
            .begin_marked_content_with_properties(Name(b"Span"))
            .properties()
            .actual_text(TextStr(""));
    }
    paint_text(content, frag, ctx.fonts, ctx.page_height_pt, ctx.cmyk);
    if carried {
        content.end_marked_content();
    }
}

/// Dispatch a fragment's own paint by content kind.
fn paint_content(content: &mut Content, frag: &Fragment, ctx: &PaintCtx) {
    match &frag.content {
        FragmentContent::Box {
            background, border, ..
        } => {
            paint_box(
                content,
                frag,
                *background,
                border,
                ctx.page_height_pt,
                ctx.cmyk,
            );
        }
        FragmentContent::TextLine { .. } => paint_line(content, frag, ctx),
        FragmentContent::Image(placement) => {
            paint_image(content, frag, placement, ctx.images, ctx.page_height_pt);
        }
        FragmentContent::Tiles(pattern) => paint_tiles(content, frag, pattern, ctx),
        FragmentContent::Marker(_) => {}
    }
}

/// Paint a background pattern: its tile at each snapped tile rectangle meeting the
/// pattern fragment (the painting area; the layer container clips the rest).
fn paint_tiles(content: &mut Content, frag: &Fragment, pattern: &TilePattern, ctx: &PaintCtx) {
    let region = (frag.x, frag.y, frag.width, frag.height);
    for (x, y, width, height) in pattern.tiles(region) {
        let tile = Fragment::new(frag.node_id, x, y, width, height, (*pattern.tile).clone());
        paint_content(content, &tile, ctx);
    }
}
