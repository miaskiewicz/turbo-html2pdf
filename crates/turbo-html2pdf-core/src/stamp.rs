//! Overlay a watermark onto every page of an existing PLAINTEXT PDF (`stamp`
//! feature).
//!
//! turbo's emitter paints a watermark *during* the render pass (see
//! [`emit::watermark`](crate::emit)), straight into freshly emitted pages via
//! the write-only `pdf-writer`. That path cannot touch a PDF that already
//! exists as bytes. [`stamp`] is the post-emit counterpart: it parses finished
//! bytes with [`lopdf`] (the same reader `append` uses — `pdf-writer` cannot
//! parse) and injects a faded diagonal watermark into each page.
//!
//! The injection is **additive**. For every page it adds one Form XObject
//! holding the rotated, faded text and appends a single `q /<name> Do Q`
//! invocation to that page's `/Contents`. The original content streams, the
//! page objects and the page count are left untouched — the overlay only draws
//! on top. Wrapping the mark in a Form XObject keeps its font and transparency
//! `/Resources` self-contained, so the overlay never collides with names the
//! source PDF already uses.
//!
//! **Font.** The mark uses the PDF base-14 `Helvetica` (a standard font every
//! viewer has), declared inline in the form's `/Resources`. A watermark is a
//! faint background stamp, not body text, so embedding/subsetting turbo's own
//! faces just for it would add bytes and complexity for no visible gain.
//!
//! **Reuse.** The rotate-about-center matrix and the CSS-px→pt scale come from
//! the emitter ([`rotation_about`](crate::emit) / `px_to_pt`), so the render-
//! time mark and this post-emit mark rotate and scale identically.
//!
//! Encryption is deliberately out of scope here: this task stamps plaintext
//! PDFs. Opening/re-sealing an encrypted PDF around a stamp is a later task.

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use pdf_writer::{Content, Name, Str};
use thiserror::Error;

use crate::emit::{px_to_pt, rotation_about, set_fill, FADE_GS_NAME};
use crate::{TextWatermark, Watermark};

/// The `/XObject` resource name the per-page overlay is registered under. Chosen
/// to be distinct from any name turbo's emitter (or a foreign PDF) uses, so
/// adding it never clobbers an existing resource.
pub const WATERMARK_XOBJECT_NAME: &str = "TurboWmStamp";

/// The base-14 font resource name used inside the overlay's own `/Resources`.
const FONT_NAME: &str = "F1";

/// A4 dimensions in points, used only as a fallback when a page carries no
/// `/MediaBox` at all (turbo always writes one per page).
const A4_PT: (f32, f32) = (595.2756, 841.8898);

/// Rough average glyph advance for `Helvetica`, as a fraction of the em. Base-14
/// fonts ship no metrics through this path, so the mark is centered on this
/// estimate; a few points of drift is invisible on a faint background stamp.
const HELVETICA_AVG_ADVANCE_EM: f32 = 0.55;

/// Why a [`stamp`] call failed.
#[derive(Debug, Error)]
pub enum StampError {
    /// The input could not be parsed as a PDF.
    #[error("malformed PDF input: {0}")]
    Malformed(#[from] lopdf::Error),
    /// The input parsed but contains no pages, so there is nothing to stamp.
    #[error("no pages to stamp: input contains no pages")]
    NoPages,
    /// An image watermark was requested. Only text watermarks are supported by
    /// the post-emit overlay (an image mark needs the emit-time raster pipeline).
    #[error("unsupported watermark: an image watermark cannot be stamped post-emit")]
    UnsupportedWatermark,
}

/// Overlay `watermark` on EVERY page of a PLAINTEXT `pdf`, returning new bytes.
///
/// The page count and every page's existing content are preserved; each page
/// gains an additive, faded, diagonal text overlay. Output is deterministic
/// (no clock, no entropy).
///
/// Returns [`StampError::Malformed`] if `pdf` does not parse, [`StampError::NoPages`]
/// if it has no pages, and [`StampError::UnsupportedWatermark`] for an image
/// watermark (only text is supported post-emit).
pub fn stamp(pdf: &[u8], watermark: &Watermark) -> Result<Vec<u8>, StampError> {
    let text = match watermark {
        Watermark::Text(text) => text.as_ref(),
        Watermark::Image(_) => return Err(StampError::UnsupportedWatermark),
    };

    let mut doc = Document::load_mem(pdf)?;
    let page_ids: Vec<ObjectId> = doc.get_pages().into_values().collect();
    if page_ids.is_empty() {
        return Err(StampError::NoPages);
    }

    for page_id in page_ids {
        overlay_page(&mut doc, page_id, text);
    }

    let mut out = Vec::new();
    doc.save_to(&mut out).expect("lopdf save to Vec");
    Ok(out)
}

/// Add the watermark Form XObject to one page and invoke it from that page's
/// content, leaving the page's original streams untouched.
fn overlay_page(doc: &mut Document, page_id: ObjectId, text: &TextWatermark) {
    let (width, height) = media_box_size(doc, page_id);
    let form = watermark_form(text, width, height);
    let form_id = doc.add_object(Object::Stream(form));
    let _ = doc.add_xobject(page_id, WATERMARK_XOBJECT_NAME.as_bytes(), form_id);
    let invocation = format!("\nq /{WATERMARK_XOBJECT_NAME} Do Q\n").into_bytes();
    let _ = doc.add_page_contents(page_id, invocation);
}

/// A page's `/MediaBox` size in points, walking up to the page's parent once if
/// the box is inherited, and falling back to A4 if absent everywhere.
fn media_box_size(doc: &Document, page_id: ObjectId) -> (f32, f32) {
    doc.get_dictionary(page_id)
        .ok()
        .and_then(|page| media_box_of(doc, page))
        .unwrap_or(A4_PT)
}

/// Read a `/MediaBox` from `dict` directly, else from its `/Parent`.
fn media_box_of(doc: &Document, dict: &Dictionary) -> Option<(f32, f32)> {
    if let Some(size) = dict.get(b"MediaBox").ok().and_then(rect_size) {
        return Some(size);
    }
    let parent_id = dict.get(b"Parent").and_then(Object::as_reference).ok()?;
    let parent = doc.get_dictionary(parent_id).ok()?;
    parent.get(b"MediaBox").ok().and_then(rect_size)
}

/// The width/height of a `[x0 y0 x1 y1]` rectangle object.
fn rect_size(obj: &Object) -> Option<(f32, f32)> {
    let rect = obj.as_array().ok()?;
    let n = |i: usize| rect.get(i).and_then(|o| o.as_float().ok());
    Some(((n(2)? - n(0)?).abs(), (n(3)? - n(1)?).abs()))
}

/// Build the watermark's Form XObject: a page-sized bounding box, an identity
/// matrix, self-contained `/Resources` (Helvetica + the fade `/ExtGState`) and
/// the rotated, faded, centered text as its content stream.
fn watermark_form(text: &TextWatermark, width: f32, height: f32) -> Stream {
    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"XObject".to_vec()));
    dict.set("Subtype", Object::Name(b"Form".to_vec()));
    dict.set("FormType", Object::Integer(1));
    dict.set("BBox", rect(0.0, 0.0, width, height));
    dict.set("Matrix", Object::Array(identity_matrix()));
    dict.set(
        "Resources",
        Object::Dictionary(form_resources(text.opacity)),
    );
    Stream::new(dict, form_content(text, width, height))
}

/// The overlay's `/Resources`: the base-14 Helvetica font and a `/ca`+`/CA`
/// transparency state set to the watermark's opacity, both inline.
fn form_resources(opacity: f32) -> Dictionary {
    let mut font = Dictionary::new();
    font.set("Type", Object::Name(b"Font".to_vec()));
    font.set("Subtype", Object::Name(b"Type1".to_vec()));
    font.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
    let mut fonts = Dictionary::new();
    fonts.set(FONT_NAME, Object::Dictionary(font));

    let mut gs = Dictionary::new();
    gs.set("Type", Object::Name(b"ExtGState".to_vec()));
    gs.set("ca", Object::Real(opacity));
    gs.set("CA", Object::Real(opacity));
    let mut states = Dictionary::new();
    states.set(FADE_GS_NAME, Object::Dictionary(gs));

    let mut resources = Dictionary::new();
    resources.set("Font", Object::Dictionary(fonts));
    resources.set("ExtGState", Object::Dictionary(states));
    resources
}

/// The overlay content stream: apply the fade, set the fill colour, rotate about
/// the page centre, then show the word centred on that centre. Built with the
/// same [`pdf_writer::Content`] builder the render-time emitter uses (see
/// `emit::watermark::paint_text`, this overlay's render-time twin) so operator
/// formatting and text-string escaping have one implementation.
fn form_content(text: &TextWatermark, width: f32, height: f32) -> Vec<u8> {
    let size_pt = px_to_pt(text.font_size);
    let advance = text_advance_pt(&text.text, size_pt);
    let (cx, cy) = (width / 2.0, height / 2.0);
    let m = rotation_about(cx, cy, text.angle_deg);

    let mut content = Content::new();
    content.save_state();
    content.set_parameters(Name(FADE_GS_NAME.as_bytes()));
    set_fill(&mut content, text.color, false);
    content.transform(m);
    content.begin_text();
    content.set_font(Name(FONT_NAME.as_bytes()), size_pt);
    content.next_line(cx - advance / 2.0, cy);
    content.show(Str(text.text.as_bytes()));
    content.end_text();
    content.restore_state();
    content.finish().to_vec()
}

/// Estimated shown width of `text` in points at `size_pt`, from the base-14
/// average advance (no glyph metrics are available on this path).
fn text_advance_pt(text: &str, size_pt: f32) -> f32 {
    text.chars().count() as f32 * size_pt * HELVETICA_AVG_ADVANCE_EM
}

/// A four-number PDF rectangle array.
fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Object {
    Object::Array(vec![
        Object::Real(x0),
        Object::Real(y0),
        Object::Real(x1),
        Object::Real(y1),
    ])
}

/// The identity transformation matrix as PDF real operands.
fn identity_matrix() -> Vec<Object> {
    [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
        .into_iter()
        .map(Object::Real)
        .collect()
}
