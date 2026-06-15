//! The PDF emitter (§7, Stage 5): turns a paginated `[Page]` into the bytes of a
//! valid PDF 1.7. This is the spine's terminal stage — everything upstream
//! (template → cascade → layout → pagination) feeds a `Vec<Page>` in here.
//!
//! **Units.** The galley and page geometry are in CSS pixels at 96 dpi; PDF user
//! space is points (1/72 in). We scale every coordinate by `PX_TO_PT = 72/96`
//! ([`unit::px_to_pt`]). The galley is y-down with the origin at the page's top
//! left; PDF is y-up with the origin at the bottom left, so y is flipped against
//! the page height ([`unit::flip_y`]).
//!
//! **Determinism (AC-7.6).** Nothing here reads the clock or any entropy. Fonts
//! are collected in first-encounter order, the creation date falls back to a
//! fixed sentinel when the caller supplies none, and `pdf-writer` lays objects
//! out in a stable order — so identical inputs yield byte-identical output.
//!
//! The work is split across small modules: [`document`] (catalog + page tree),
//! [`page`] (per-page content stream), [`text`] (text-showing ops), [`graphics`]
//! (background/border rects), [`fonts`] (subsetting + embedding), [`color`]
//! (device RGB) and [`meta`] (the document info dict).

mod color;
mod document;
mod fonts;
mod graphics;
mod image;
mod meta;
mod page;
mod text;
mod unit;

use crate::image::{ImageResolver, NoImages};
use crate::paginate::Page;

pub use fonts::FontStore;
pub use image::ImageStore;

/// Document metadata plus the determinism knob for the creation date (§7, §14).
///
/// Every field is optional; an absent field is simply omitted from the PDF info
/// dictionary. When `creation_date` is `None` the emitter substitutes a fixed
/// sentinel ([`SENTINEL_DATE`]) so two renders of the same input are
/// byte-identical (AC-7.6).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmitOptions {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    /// Creation date as a Unix timestamp (seconds). `None` uses [`SENTINEL_DATE`].
    pub creation_date: Option<i64>,
}

/// The fixed creation-date sentinel: `2000-01-01T00:00:00Z`. Used whenever the
/// caller leaves [`EmitOptions::creation_date`] unset, keeping output
/// reproducible (AC-7.6).
pub const SENTINEL_DATE: i64 = 946_684_800;

/// Emit a paginated document as the bytes of a PDF 1.7 file.
///
/// The fragments in each [`Page`] are painted in galley order: boxes
/// (backgrounds + borders) and text lines, with fonts subset and embedded once
/// across the whole document. The result opens without a repair prompt in
/// conformant viewers (AC-7.1).
///
/// This convenience entry embeds no images; use [`emit_pdf_with_images`] to
/// supply a resolver for `<img>`/`background-image` content (§7.4).
pub fn emit_pdf(pages: &[Page], opts: &EmitOptions) -> Vec<u8> {
    emit_pdf_with_images(pages, opts, &NoImages)
}

/// Emit a paginated document, embedding every `Image` fragment whose name the
/// `resolver` supplies as a PDF image XObject (§7.4, Phase 9b). PNG decodes to
/// raw RGB(A) (alpha becomes an `SMask`); JPEG passes through as `DCTDecode`.
/// Images the resolver can't supply or decode are simply skipped.
pub fn emit_pdf_with_images(
    pages: &[Page],
    opts: &EmitOptions,
    resolver: &dyn ImageResolver,
) -> Vec<u8> {
    document::build(pages, opts, resolver)
}
