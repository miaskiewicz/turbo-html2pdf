//! Embed + transplant a bundled font for the `stamp` overlay (`stamp` feature,
//! Task 1 of 4).
//!
//! [`embed_watermark_font`] gives the post-emit `stamp` overlay a real,
//! embedded font instead of the unbundled base-14 `Helvetica` it uses today
//! (see `stamp.rs`'s module doc): it drives the SAME [`crate::emit::FontStore`]
//! subsetting/Type0/CID/`FontFile` machinery the render path uses, into a
//! throwaway [`pdf_writer::Pdf`], then transplants just the resulting font's
//! object closure into the caller's [`lopdf::Document`] with fresh ids — the
//! same renumber-and-merge idea `append.rs` uses for whole foreign PDFs, here
//! scoped to a handful of font objects.
//!
//! **Step 1 spike answers** (see `tests/stamp_font/mod.rs`'s `step1_spike`
//! module for the executable proof):
//! - (a) `Pdf::finish()`'s bytes load in `lopdf::Document::load_mem` with NO
//!   `/Catalog`/`/Root` at all. `Pdf::catalog` is documented "Required", but
//!   `finish()` only *omits* `/Root` from the trailer when none was set, and
//!   `load_mem` parses the object table/xref/trailer independently of whether
//!   a `/Root` is present — it's read lazily by callers (`get_pages`,
//!   `trailer.get("Root")`), never validated at load time. So no stub catalog
//!   is written here.
//! - (b) The font objects keep the exact id `FontStore::write`'s `Ref`s
//!   reported: `load_mem` builds its object table by parsing each `N 0 obj`
//!   header verbatim, it does not renumber. So the Type0 is located by
//!   `refs[0].get()`, never by scanning for `/Subtype /Type0`.
//!
use lopdf::{Document, Object, ObjectId};
use pdf_writer::Pdf;

use crate::emit::{EmitOptions, FontStore, RefAlloc};
use crate::text::FontFace;

/// A watermark font ready to reference from a `stamp`-overlay content stream.
pub(crate) struct EmbeddedWatermarkFont {
    /// The transplanted `Type0` font object, now living in `doc`.
    pub type0_id: ObjectId,
    /// The watermark text as 2-byte BE subset-local glyph ids (`Identity-H`).
    pub codes: Vec<u8>,
    /// Total shown advance at `font_size_px`, in points, for centering.
    pub advance_pt: f32,
}

/// Shape `text` in `face`, embed it via the existing [`FontStore`]
/// subsetting/CID/`FontFile` machinery, and transplant the resulting Type0
/// font's object closure into `doc` with fresh, non-colliding ids.
pub(crate) fn embed_watermark_font(
    doc: &mut Document,
    face: &FontFace,
    text: &str,
    font_size_px: f32,
) -> EmbeddedWatermarkFont {
    let glyphs = face.shape(text);
    let gids: Vec<u16> = glyphs.iter().map(|g| g.glyph_id).collect();

    let mut store = FontStore::default();
    store.record_glyphs(face, &gids);
    let mini = write_mini_font_pdf(&store);

    let type0_id = transplant_font_closure(doc, &mini.doc, mini.type0_id);
    let codes = remapped_codes(&store, face, &gids);
    let advance_pt = shown_advance_pt(&glyphs, face, font_size_px);

    EmbeddedWatermarkFont {
        type0_id,
        codes,
        advance_pt,
    }
}

/// A throwaway single-face PDF written purely to get `FontStore::write`'s
/// Type0/CID/descriptor/program objects onto the wire, so `lopdf` can read
/// them back for transplant.
struct MiniFontPdf {
    doc: Document,
    type0_id: ObjectId,
}

/// Write `store`'s one collected face into a fresh `pdf_writer::Pdf` (no
/// catalog — Step 1(a)) and read it back with `lopdf`, returning the Type0's
/// id at the exact object number `FontStore::write`'s `Ref` reported (Step
/// 1(b)).
fn write_mini_font_pdf(store: &FontStore) -> MiniFontPdf {
    let mut pdf = Pdf::new();
    let mut alloc = RefAlloc::new(1);
    let refs = store.write(&mut pdf, &mut alloc, &EmitOptions::default());
    let bytes = pdf.finish();

    let doc = Document::load_mem(&bytes).expect("FontStore::write output is a valid mini PDF");
    let type0_id: ObjectId = (refs[0].get() as u32, 0);
    MiniFontPdf { doc, type0_id }
}

/// Every object id reachable from `root`, transitively following
/// `Object::Reference`s within `mini` (dict/stream/array bodies).
fn font_closure(mini: &Document, root: ObjectId) -> Vec<ObjectId> {
    let mut ids = Vec::new();
    walk_closure(mini, root, &mut ids);
    ids
}

fn walk_closure(mini: &Document, id: ObjectId, seen: &mut Vec<ObjectId>) {
    if seen.contains(&id) {
        return;
    }
    seen.push(id);
    let Ok(obj) = mini.get_object(id) else {
        return;
    };
    walk_references(mini, obj, seen);
}

/// Visit every value nested in `obj` (dict/stream-dict/array bodies,
/// recursively — `/DescendantFonts` is an ARRAY *containing* a reference, not
/// a reference itself, so a value must be walked, not just pattern-matched
/// once) and follow any `Object::Reference` found.
fn walk_references(mini: &Document, obj: &Object, seen: &mut Vec<ObjectId>) {
    match obj {
        Object::Reference(r) => walk_closure(mini, *r, seen),
        Object::Dictionary(dict) => {
            for (_, value) in dict.iter() {
                walk_references(mini, value, seen);
            }
        }
        Object::Stream(stream) => {
            for (_, value) in stream.dict.iter() {
                walk_references(mini, value, seen);
            }
        }
        Object::Array(arr) => {
            for value in arr {
                walk_references(mini, value, seen);
            }
        }
        _ => {}
    }
}

/// Copy the Type0 font's object closure from `mini` into `doc` under fresh,
/// non-colliding ids (`doc.max_id + 1..`), rewriting every internal
/// `Object::Reference` to the new numbering. Returns the new Type0 id. Only
/// the font closure is copied, so `mini`'s (nonexistent) catalog/pages never
/// enter `doc` — this function never even looks for one.
fn transplant_font_closure(doc: &mut Document, mini: &Document, type0_id: ObjectId) -> ObjectId {
    let old_ids = font_closure(mini, type0_id);
    let id_map = renumber_map(doc, &old_ids);
    for &old_id in &old_ids {
        let mut obj = mini
            .get_object(old_id)
            .expect("closure id resolves in mini")
            .clone();
        rewrite_references(&mut obj, &id_map);
        doc.objects.insert(id_map[&old_id], obj);
    }
    doc.max_id = id_map.values().map(|(n, _)| *n).max().unwrap_or(doc.max_id);
    id_map[&type0_id]
}

/// Assign each `old_ids` entry a fresh id above `doc.max_id`, preserving
/// order so the mapping is deterministic for a given closure walk order.
fn renumber_map(doc: &Document, old_ids: &[ObjectId]) -> IdMap {
    let mut next = doc.max_id;
    old_ids
        .iter()
        .map(|&old| {
            next += 1;
            (old, (next, 0))
        })
        .collect()
}

type IdMap = std::collections::HashMap<ObjectId, ObjectId>;

/// Rewrite every `Object::Reference` inside `obj` (recursively, through
/// dict/stream/array bodies) from its old id to `id_map`'s new one.
fn rewrite_references(obj: &mut Object, id_map: &IdMap) {
    match obj {
        Object::Dictionary(dict) => rewrite_dict(dict, id_map),
        Object::Stream(stream) => rewrite_dict(&mut stream.dict, id_map),
        Object::Array(arr) => rewrite_array(arr, id_map),
        Object::Reference(id) => rewrite_reference(id, id_map),
        _ => {}
    }
}

fn rewrite_dict(dict: &mut lopdf::Dictionary, id_map: &IdMap) {
    for (_, value) in dict.iter_mut() {
        rewrite_references(value, id_map);
    }
}

fn rewrite_array(arr: &mut [Object], id_map: &IdMap) {
    for value in arr.iter_mut() {
        rewrite_references(value, id_map);
    }
}

fn rewrite_reference(id: &mut ObjectId, id_map: &IdMap) {
    if let Some(&new_id) = id_map.get(id) {
        *id = new_id;
    }
}

/// The watermark text's shaped glyphs, remapped to their subset-local (CID)
/// ids and packed as 2-byte big-endian codes — exactly what `Identity-H`
/// expects a `Tj`/`show` operand to carry.
fn remapped_codes(store: &FontStore, face: &FontFace, gids: &[u16]) -> Vec<u8> {
    let face_index = store.index_of(face);
    let mut codes = Vec::with_capacity(gids.len() * 2);
    for &gid in gids {
        codes.extend_from_slice(&store.remap(face_index, gid).to_be_bytes());
    }
    codes
}

/// The shown text's total advance in points at `font_size_px`, matching the
/// render path's own scaling (`emit::watermark::paint_text`, read-only
/// reference — design-unit advance times the units-per-em → px → pt scale).
fn shown_advance_pt(
    glyphs: &[crate::text::ShapedGlyph],
    face: &FontFace,
    font_size_px: f32,
) -> f32 {
    let scale = crate::emit::px_to_pt(font_size_px) / f32::from(face.units_per_em());
    glyphs.iter().map(|g| g.x_advance as f32 * scale).sum()
}

#[cfg(test)]
#[path = "../tests/stamp_font/mod.rs"]
mod tests;
