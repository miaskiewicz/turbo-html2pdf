//! Per-page content stream (§7). Walks every band of a [`Page`] in galley order,
//! painting box backgrounds/borders and text lines into a single content stream.
//! Bands beyond the body (header/footer/footnotes) are empty until Phases 7/8,
//! but we iterate them so they paint the moment they fill.

use pdf_writer::Content;

use crate::layout::fragment::{Fragment, FragmentContent};
use crate::paginate::Page;

use super::fonts::FontStore;
use super::graphics::paint_box;
use super::text::paint_text;
use super::unit::px_to_pt;

/// Build the content-stream bytes for one page.
pub fn content_stream(page: &Page, fonts: &FontStore) -> Vec<u8> {
    let page_height_pt = px_to_pt(page.geometry.height);
    let mut content = Content::new();
    for band in bands(page) {
        for frag in band {
            paint_fragment(&mut content, frag, fonts, page_height_pt);
        }
    }
    content.finish().to_vec()
}

/// The four paint bands in back-to-front order.
fn bands(page: &Page) -> [&[Fragment]; 4] {
    [&page.body, &page.header, &page.footer, &page.footnotes]
}

/// Paint one fragment (its own content first, then its children atop it).
fn paint_fragment(content: &mut Content, frag: &Fragment, fonts: &FontStore, page_height_pt: f32) {
    paint_content(content, frag, fonts, page_height_pt);
    for child in &frag.children {
        paint_fragment(content, child, fonts, page_height_pt);
    }
}

/// Dispatch a fragment's own paint by content kind.
fn paint_content(content: &mut Content, frag: &Fragment, fonts: &FontStore, page_height_pt: f32) {
    match &frag.content {
        FragmentContent::Box { background, border } => {
            paint_box(content, frag, *background, border, page_height_pt);
        }
        FragmentContent::TextLine { .. } => {
            paint_text(content, frag, fonts, page_height_pt);
        }
        FragmentContent::Directive(_) => {}
    }
}
