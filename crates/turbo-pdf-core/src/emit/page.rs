//! Per-page content stream (§7). Walks every band of a [`Page`] in galley order,
//! painting box backgrounds/borders and text lines into a single content stream.
//! Bands beyond the body (header/footer/footnotes) are empty until Phases 7/8,
//! but we iterate them so they paint the moment they fill.

use pdf_writer::Content;

use crate::layout::fragment::{Fragment, FragmentContent};
use crate::paginate::Page;

use super::fonts::FontStore;
use super::graphics::paint_box;
use super::image::{paint_image, ImageStore};
use super::text::paint_text;
use super::unit::px_to_pt;

/// The painter context threaded through a page's fragments: the resource stores
/// and the page height (points) used for the galley→PDF y-flip.
struct PaintCtx<'a> {
    fonts: &'a FontStore,
    images: &'a ImageStore,
    page_height_pt: f32,
}

/// Build the content-stream bytes for one page.
pub fn content_stream(page: &Page, fonts: &FontStore, images: &ImageStore) -> Vec<u8> {
    let ctx = PaintCtx {
        fonts,
        images,
        page_height_pt: px_to_pt(page.geometry.height),
    };
    let mut content = Content::new();
    for band in bands(page) {
        for frag in band {
            paint_fragment(&mut content, frag, &ctx);
        }
    }
    content.finish().to_vec()
}

/// The four paint bands in back-to-front order.
fn bands(page: &Page) -> [&[Fragment]; 4] {
    [&page.body, &page.header, &page.footer, &page.footnotes]
}

/// Paint one fragment (its own content first, then its children atop it).
fn paint_fragment(content: &mut Content, frag: &Fragment, ctx: &PaintCtx) {
    paint_content(content, frag, ctx);
    for child in &frag.children {
        paint_fragment(content, child, ctx);
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
