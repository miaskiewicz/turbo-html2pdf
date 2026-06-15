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

use pdf_writer::Content;

use crate::layout::fragment::{Fragment, FragmentContent};
use crate::paginate::Page;

use super::fonts::FontStore;
use super::graphics::paint_box;
use super::image::{paint_image, ImageStore};
use super::text::paint_text;
use super::unit::px_to_pt;
use super::watermark::{self, Watermark};

/// The painter context threaded through a page's fragments: the resource stores
/// and the page height (points) used for the galley→PDF y-flip.
struct PaintCtx<'a> {
    fonts: &'a FontStore,
    images: &'a ImageStore,
    page_height_pt: f32,
}

/// Build the content-stream bytes for one page. A watermark, when present, is
/// painted first so the body bands draw on top of it (behind-body ordering).
pub fn content_stream(
    page: &Page,
    fonts: &FontStore,
    images: &ImageStore,
    watermark: Option<&Watermark>,
    #[cfg(feature = "pdf-ua")] tags: &super::ua::PageTags,
) -> Vec<u8> {
    let ctx = PaintCtx {
        fonts,
        images,
        page_height_pt: px_to_pt(page.geometry.height),
    };
    let mut content = Content::new();
    #[cfg(feature = "pdf-ua")]
    let mut marker = super::ua::Marker::new(tags);
    paint_watermark(&mut content, page, watermark, fonts, images);
    paint_bands(
        &mut content,
        page,
        &ctx,
        #[cfg(feature = "pdf-ua")]
        &mut marker,
    );
    content.finish().to_vec()
}

/// Paint the page watermark behind the body, wrapped as an artifact under
/// `pdf-ua` so it is skipped by assistive tech.
fn paint_watermark(
    content: &mut Content,
    page: &Page,
    watermark: Option<&Watermark>,
    fonts: &FontStore,
    images: &ImageStore,
) {
    let Some(mark) = watermark else {
        return;
    };
    #[cfg(feature = "pdf-ua")]
    super::ua::begin_artifact(content);
    watermark::paint(content, mark, page, fonts, images);
    #[cfg(feature = "pdf-ua")]
    content.end_marked_content();
}

/// Paint every band of a page in back-to-front order. Without `pdf-ua` the band
/// identity is irrelevant; with it, the header/footer bands paint as artifacts.
#[cfg(not(feature = "pdf-ua"))]
fn paint_bands(content: &mut Content, page: &Page, ctx: &PaintCtx) {
    for band in bands(page) {
        for frag in band {
            paint_fragment(content, frag, ctx);
        }
    }
}

/// `pdf-ua` variant: thread the marked-content marker and tag each band.
#[cfg(feature = "pdf-ua")]
fn paint_bands(content: &mut Content, page: &Page, ctx: &PaintCtx, marker: &mut super::ua::Marker) {
    for (index, band) in bands(page).into_iter().enumerate() {
        for frag in band {
            paint_fragment(content, frag, ctx, marker, is_artifact_band(index));
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

/// Paint one fragment (its own content first, then its children atop it).
fn paint_fragment(
    content: &mut Content,
    frag: &Fragment,
    ctx: &PaintCtx,
    #[cfg(feature = "pdf-ua")] marker: &mut super::ua::Marker,
    #[cfg(feature = "pdf-ua")] artifact_band: bool,
) {
    paint_one(
        content,
        frag,
        ctx,
        #[cfg(feature = "pdf-ua")]
        marker,
        #[cfg(feature = "pdf-ua")]
        artifact_band,
    );
    for child in &frag.children {
        paint_fragment(
            content,
            child,
            ctx,
            #[cfg(feature = "pdf-ua")]
            marker,
            #[cfg(feature = "pdf-ua")]
            artifact_band,
        );
    }
}

/// Paint a single fragment's own content (no marked content).
#[cfg(not(feature = "pdf-ua"))]
fn paint_one(content: &mut Content, frag: &Fragment, ctx: &PaintCtx) {
    paint_content(content, frag, ctx);
}

/// `pdf-ua` variant: bracket each real-content paint with `BDC`/`EMC` (an MCID
/// for content, `/Artifact` for decoration).
#[cfg(feature = "pdf-ua")]
fn paint_one(
    content: &mut Content,
    frag: &Fragment,
    ctx: &PaintCtx,
    marker: &mut super::ua::Marker,
    artifact_band: bool,
) {
    match super::ua::wrap_kind(frag, artifact_band) {
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

/// Dispatch a fragment's own paint by content kind.
fn paint_content(content: &mut Content, frag: &Fragment, ctx: &PaintCtx) {
    match &frag.content {
        FragmentContent::Box { background, border } => {
            paint_box(content, frag, *background, border, ctx.page_height_pt);
        }
        FragmentContent::TextLine { .. } => {
            paint_text(content, frag, ctx.fonts, ctx.page_height_pt);
        }
        FragmentContent::Image(placement) => {
            paint_image(content, frag, placement, ctx.images, ctx.page_height_pt);
        }
        FragmentContent::Directive(_) => {}
    }
}
