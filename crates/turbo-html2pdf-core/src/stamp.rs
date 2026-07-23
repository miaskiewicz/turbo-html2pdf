//! Overlay a watermark onto every page of an existing PDF — plaintext, or
//! password-protected via the optional decrypt/re-encrypt round-trip (`stamp`
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
//! **Encryption.** [`stamp`] can also open a PASSWORD-PROTECTED input and hand
//! back a still-protected result: `password` decrypts the input (via
//! `lopdf::Document::decrypt`) before the overlay runs; `encryption` re-seals
//! the result afterwards. Both are independently optional, so plaintext in/out
//! is unchanged. Decrypting reuses exactly what the Task 1 spike
//! (`tests/encryption_roundtrip.rs`) proved against turbo's own V5/R6/AESV3
//! `encrypt` feature output — see [`normalize_encrypt_length`] for the one
//! byte-level workaround that reuse needed. Re-encrypting does NOT go through
//! lopdf's own `Document::encrypt` (its `/Encrypt` output fails qpdf
//! validation — no trailer `/ID`, a miscomputed `/Perms`); instead the stamped
//! document is serialized to plaintext bytes and handed to turbo's own
//! qpdf-validated [`emit::encrypt_pdf`](crate::emit), the same encryptor the
//! render path already relies on.

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use pdf_writer::{Content, Name, Str};
use thiserror::Error;

use crate::emit::{encrypt_pdf, px_to_pt, rotation_about, set_fill, Encryption, FADE_GS_NAME};
use crate::layout::value::Rgba;

/// The `/XObject` resource name the per-page overlay is registered under. Chosen
/// to be distinct from any name turbo's emitter (or a foreign PDF) uses, so
/// adding it never clobbers an existing resource.
pub const WATERMARK_XOBJECT_NAME: &str = "TurboWmStamp";

/// The base-14 font resource name used inside the overlay's own `/Resources`.
const FONT_NAME: &str = "F1";

/// A text watermark for the post-emit overlay. Face-less on purpose: the
/// overlay draws base-14 Helvetica (never shapes or embeds a font), unlike the
/// render-time [`crate::TextWatermark`], whose mark carries a [`crate::FontFace`]
/// for the emitter's glyph-subsetting path.
#[derive(Debug, Clone)]
pub struct StampWatermark {
    /// The word to stamp.
    pub text: String,
    /// Font size in CSS px (96 dpi), scaled to points via [`px_to_pt`] like the
    /// render-time mark.
    pub font_size: f32,
    /// Fill color (the alpha channel is ignored here; fade is via `opacity`).
    pub color: Rgba,
    /// Fill opacity `0.0..=1.0`, applied through the overlay's `/ca`+`/CA`
    /// `ExtGState`.
    pub opacity: f32,
    /// Rotation about the page center, in degrees (counter-clockwise).
    pub angle_deg: f32,
}

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
    /// `password` was `Some` but did not open the input (wrong password, or the
    /// input was not actually encrypted under the handler lopdf supports).
    #[error("failed to decrypt input PDF: {0}")]
    Decrypt(lopdf::Error),
    /// Retained for API stability; re-encrypting now goes through turbo's own
    /// [`emit::encrypt_pdf`](crate::emit), which is infallible, so `stamp` no
    /// longer produces this variant.
    #[error("failed to encrypt output PDF: {0}")]
    Encrypt(lopdf::Error),
}

/// Overlay `watermark` on EVERY page of `pdf`, returning new bytes.
///
/// The page count and every page's existing content are preserved; each page
/// gains an additive, faded, diagonal text overlay.
///
/// `password` and `encryption` are independently optional:
/// - `password` is `Some` to open a PASSWORD-PROTECTED `pdf` first; `None` reads
///   `pdf` as plaintext, unchanged from before.
/// - `encryption` is `Some` to re-seal the stamped result under a (possibly new)
///   password; `None` saves plaintext, unchanged from before.
///
/// With both `None` the output is deterministic (no clock, no entropy), exactly
/// as before this parameter was added; encrypting introduces the random
/// salts/IVs/file key encryption legitimately needs.
///
/// Returns [`StampError::Malformed`] if `pdf` does not parse, [`StampError::NoPages`]
/// if it has no pages, and [`StampError::Decrypt`] if `password` does not open
/// an encrypted `pdf`.
pub fn stamp(
    pdf: &[u8],
    watermark: &StampWatermark,
    password: Option<&str>,
    encryption: Option<&Encryption>,
) -> Result<Vec<u8>, StampError> {
    let mut doc = load_document(pdf, password)?;
    stamp_all_pages(&mut doc, watermark)?;

    if encryption.is_some() {
        close_object_number_gaps(&mut doc);
    }

    let mut plaintext = Vec::new();
    doc.save_to(&mut plaintext).expect("lopdf save to Vec");

    Ok(match encryption {
        Some(enc) => encrypt_pdf(&plaintext, enc),
        None => plaintext,
    })
}

/// Compact `doc`'s object numbers to a contiguous `1..=N` run, remapping every
/// reference throughout the graph (`lopdf`'s own [`Document::renumber_objects`]
/// — reused, not reimplemented).
///
/// [`encrypt_pdf`]'s classic-xref writer assumes exactly that shape: one
/// contiguous run with no holes. `load_document`'s decrypt step removes the
/// input's `/Encrypt` dictionary object (`lopdf::Document::decrypt` drops it
/// from both the trailer and the object map, see its source), which otherwise
/// leaves a gap at that object's old number — invisible to lopdf's own
/// multi-subsection xref writer (which happily skips gaps), but not to
/// `encrypt_pdf`'s, which writes one subsection for the whole `0..size` range.
/// Renumbering before encrypting closes that gap unconditionally, so it also
/// covers a "foreign" input PDF that already carried a gap of its own.
/// Skipped for plaintext output so the no-encryption path stays exactly as
/// before (byte-identical, no gap ever introduced there).
fn close_object_number_gaps(doc: &mut Document) {
    doc.renumber_objects();
}

/// Overlay `text` on every page of `doc`, or [`StampError::NoPages`] if it has
/// none.
fn stamp_all_pages(doc: &mut Document, text: &StampWatermark) -> Result<(), StampError> {
    let page_ids: Vec<ObjectId> = doc.get_pages().into_values().collect();
    if page_ids.is_empty() {
        return Err(StampError::NoPages);
    }
    for page_id in page_ids {
        overlay_page(doc, page_id, text);
    }
    Ok(())
}

/// Parse `pdf`, decrypting it with `password` first when given.
///
/// A parse failure (either before or after decrypting) is [`StampError::Malformed`];
/// a decrypt failure (wrong password, or not actually encrypted) is
/// [`StampError::Decrypt`] — the same split the Task 1 spike proved, now the
/// permanent shape of `stamp`'s input path.
fn load_document(pdf: &[u8], password: Option<&str>) -> Result<Document, StampError> {
    let Some(password) = password else {
        return Ok(Document::load_mem(pdf)?);
    };
    let normalized = normalize_encrypt_length(pdf);
    let mut doc = Document::load_mem(&normalized)?;
    doc.decrypt(password).map_err(StampError::Decrypt)?;
    Ok(doc)
}

/// Work around a confirmed lopdf 0.36 bug (not a missing capability, see the
/// Task 1 spike's module doc for the full root-cause trace): `PasswordAlgorithm
/// ::try_from` unconditionally rejects any `/Encrypt` dict whose top-level
/// `/Length` falls outside `40..=128`, even though that field is read only by
/// the legacy R2-R4 key derivation and is dead for R6/AESV3. turbo's own
/// `encrypt` feature writes `/Length 256` for spec/Acrobat compatibility (ISO
/// 32000-2 Table 20 does not require it for V5, but real-world V5 encoders
/// commonly still write it), which trips lopdf's over-strict check.
///
/// Rewriting the ASCII digits after that exact, unique `/Length 256` substring
/// to any in-range value is offset-preserving (same byte length, so no xref
/// offset in the file shifts) and provably inert for R6 (the field is unused by
/// the AESV3 key derivation lopdf actually runs) — general input tolerance, not
/// a test-only patch. When the substring is absent (plaintext input, or an
/// encrypted input that doesn't carry the quirk), the bytes pass through
/// unchanged.
fn normalize_encrypt_length(pdf: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    let needle = b"/Length 256";
    match pdf.windows(needle.len()).position(|w| w == needle) {
        Some(at) => {
            let mut patched = pdf.to_vec();
            patched[at..at + needle.len()].copy_from_slice(b"/Length 128");
            std::borrow::Cow::Owned(patched)
        }
        None => std::borrow::Cow::Borrowed(pdf),
    }
}

/// Add the watermark Form XObject to one page and invoke it from that page's
/// content, leaving the page's original streams untouched.
fn overlay_page(doc: &mut Document, page_id: ObjectId, text: &StampWatermark) {
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
    let n: Vec<f32> = (0..4)
        .map(|i| rect.get(i).and_then(|o| o.as_float().ok()))
        .collect::<Option<Vec<f32>>>()?;
    Some(((n[2] - n[0]).abs(), (n[3] - n[1]).abs()))
}

/// Build the watermark's Form XObject: a page-sized bounding box, an identity
/// matrix, self-contained `/Resources` (Helvetica + the fade `/ExtGState`) and
/// the rotated, faded, centered text as its content stream.
fn watermark_form(text: &StampWatermark, width: f32, height: f32) -> Stream {
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
fn form_content(text: &StampWatermark, width: f32, height: f32) -> Vec<u8> {
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
