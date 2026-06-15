//! Font subsetting and embedding (§7, AC-7.2). Every glyph the document shows is
//! gathered per face; each face is then subset with `subsetter` to keep only
//! those glyphs and embedded once — TrueType (`glyf`) as a `FontFile2` in a
//! `CIDFontType2`, CFF/OTF as a `FontFile3` in a `CIDFontType0`. Both ride a
//! `Type0` font with `Identity-H` encoding, so the codes we show in the content
//! stream are simply the *new* (remapped) glyph ids as 2-byte values.

use std::collections::BTreeSet;

use pdf_writer::types::{CidFontType, FontFlags, SystemInfo};
use pdf_writer::{Chunk, Finish, Name, Rect, Ref, Str};
use subsetter::GlyphRemapper;

use crate::layout::fragment::{Fragment, FragmentContent, PositionedGlyph};
use crate::text::FontFace;

/// One embedded face: the source font and the set of original glyph ids the
/// document shows from it. Faces are kept in first-encounter order so the PDF's
/// font resources are laid out deterministically.
struct UsedFont {
    face: FontFace,
    glyphs: BTreeSet<u16>,
}

/// All faces used across the document, keyed by font-program identity. Built by
/// walking every page's fragments; consumed by [`FontStore::write`] to emit the
/// font objects and by the content streams to resolve a face to its resource
/// name and remapped glyph ids.
#[derive(Default)]
pub struct FontStore {
    fonts: Vec<UsedFont>,
}

/// Two faces are the same embedded font when they share the same font program.
/// Clones of a registry face share an `Arc`, so the data pointer is stable.
fn same_face(a: &FontFace, b: &FontFace) -> bool {
    std::ptr::eq(a.data().as_ptr(), b.data().as_ptr()) && a.data().len() == b.data().len()
}

impl FontStore {
    /// Build a store by collecting the glyphs used on every page.
    pub fn collect(pages: &[crate::paginate::Page]) -> FontStore {
        let mut store = FontStore::default();
        for page in pages {
            store.collect_page(page);
        }
        store
    }

    fn collect_page(&mut self, page: &crate::paginate::Page) {
        let bands = [&page.body, &page.header, &page.footer, &page.footnotes];
        for band in bands {
            for frag in band {
                self.collect_fragment(frag);
            }
        }
    }

    fn collect_fragment(&mut self, frag: &Fragment) {
        if let FragmentContent::TextLine { glyphs, face, .. } = &frag.content {
            self.record(face, glyphs);
        }
        for child in &frag.children {
            self.collect_fragment(child);
        }
    }

    fn record(&mut self, face: &FontFace, glyphs: &[PositionedGlyph]) {
        let idx = self.face_index(face);
        let used = &mut self.fonts[idx];
        for g in glyphs {
            used.glyphs.insert(g.glyph_id);
        }
    }

    /// Register a face plus a set of original glyph ids that aren't carried by
    /// any fragment (e.g. a watermark word), so they subset and embed exactly
    /// like body text. Must be called during the collect pass, before `write`.
    pub fn record_glyphs(&mut self, face: &FontFace, glyph_ids: &[u16]) {
        let idx = self.face_index(face);
        let used = &mut self.fonts[idx];
        for &gid in glyph_ids {
            used.glyphs.insert(gid);
        }
    }

    /// The index of `face` in the store, inserting it if first seen.
    fn face_index(&mut self, face: &FontFace) -> usize {
        if let Some(i) = self.fonts.iter().position(|f| same_face(&f.face, face)) {
            return i;
        }
        self.fonts.push(UsedFont {
            face: face.clone(),
            glyphs: BTreeSet::new(),
        });
        self.fonts.len() - 1
    }

    /// The PDF resource name for the `n`th font (`F0`, `F1`, …).
    pub fn resource_name(n: usize) -> String {
        format!("F{n}")
    }

    /// Resolve a face to its font index, panicking if it was never collected
    /// (the collect pass visits the same fragments, so this never misses).
    pub fn index_of(&self, face: &FontFace) -> usize {
        self.fonts
            .iter()
            .position(|f| same_face(&f.face, face))
            .expect("face was collected before emission")
    }

    /// The remapped (subset-local) glyph id for an original glyph id on a face.
    pub fn remap(&self, face_index: usize, glyph_id: u16) -> u16 {
        self.fonts[face_index]
            .remapper()
            .get(glyph_id)
            .expect("glyph was collected for this face")
    }

    /// The number of distinct embedded faces.
    pub fn len(&self) -> usize {
        self.fonts.len()
    }

    /// Whether no face was collected (an all-graphics document needs no fonts).
    pub fn is_empty(&self) -> bool {
        self.fonts.is_empty()
    }

    /// Write every collected face into `chunk`, returning the Type0 font refs in
    /// resource order. `alloc` hands out fresh object ids.
    pub fn write(&self, chunk: &mut Chunk, alloc: &mut RefAlloc) -> Vec<Ref> {
        self.fonts
            .iter()
            .map(|font| write_font(font, chunk, alloc))
            .collect()
    }
}

impl UsedFont {
    /// The remapper that assigns subset-local glyph ids, `.notdef` first.
    fn remapper(&self) -> GlyphRemapper {
        let gids: Vec<u16> = self.glyphs.iter().copied().collect();
        GlyphRemapper::new_from_glyphs(&gids)
    }
}

/// A monotonic object-id allocator over a single `pdf-writer` document.
pub struct RefAlloc {
    next: i32,
}

impl RefAlloc {
    /// Start allocating at object id `first`.
    pub fn new(first: i32) -> RefAlloc {
        RefAlloc { next: first }
    }

    /// Hand out the next fresh object reference.
    pub fn bump(&mut self) -> Ref {
        let r = Ref::new(self.next);
        self.next += 1;
        r
    }
}

/// Subset, embed, and wire up one face; returns the Type0 font ref.
fn write_font(font: &UsedFont, chunk: &mut Chunk, alloc: &mut RefAlloc) -> Ref {
    let remapper = font.remapper();
    let subset = subset_bytes(&font.face, &remapper);
    let refs = FontRefs::alloc(alloc);
    write_type0(chunk, &refs, font);
    write_cid_font(chunk, &refs, font, &remapper);
    write_descriptor(chunk, &refs, &font.face);
    embed_program(chunk, &refs, &font.face, &subset);
    refs.type0
}

/// Run the subsetter, falling back to the original bytes if it declines the
/// font (still valid: a full embed, just larger).
fn subset_bytes(face: &FontFace, remapper: &GlyphRemapper) -> Vec<u8> {
    match subsetter::subset(face.data(), 0, remapper) {
        Ok(bytes) => bytes,
        Err(_) => face.data().to_vec(),
    }
}

/// The four object refs a single embedded font occupies.
struct FontRefs {
    type0: Ref,
    cid: Ref,
    descriptor: Ref,
    program: Ref,
}

impl FontRefs {
    fn alloc(alloc: &mut RefAlloc) -> FontRefs {
        FontRefs {
            type0: alloc.bump(),
            cid: alloc.bump(),
            descriptor: alloc.bump(),
            program: alloc.bump(),
        }
    }
}

/// A subset tag is a PDF-mandated 6-uppercase-letter prefix on the base font
/// name. We derive it deterministically from the face index via the program ref.
fn subset_tag(program: Ref) -> String {
    let mut n = program.get() as u32;
    let mut tag = String::with_capacity(7);
    for _ in 0..6 {
        let letter = b'A' + (n % 26) as u8;
        tag.push(char::from(letter));
        n /= 26;
    }
    tag.push('+');
    tag
}

/// The base font name for a face: a subset tag plus a sanitized family.
fn base_font_name(face: &FontFace, program: Ref) -> String {
    let family: String = face
        .family()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    format!("{}{}", subset_tag(program), family)
}

fn write_type0(chunk: &mut Chunk, refs: &FontRefs, font: &UsedFont) {
    let name = base_font_name(&font.face, refs.program);
    let mut type0 = chunk.type0_font(refs.type0);
    type0.base_font(Name(name.as_bytes()));
    type0.encoding_predefined(Name(b"Identity-H"));
    type0.descendant_font(refs.cid);
    type0.finish();
}

fn cid_subtype(face: &FontFace) -> CidFontType {
    if face.is_cff() {
        CidFontType::Type0
    } else {
        CidFontType::Type2
    }
}

fn write_cid_font(chunk: &mut Chunk, refs: &FontRefs, font: &UsedFont, remapper: &GlyphRemapper) {
    let name = base_font_name(&font.face, refs.program);
    let mut cid = chunk.cid_font(refs.cid);
    cid.subtype(cid_subtype(&font.face));
    cid.base_font(Name(name.as_bytes()));
    cid.system_info(SystemInfo {
        registry: Str(b"Adobe"),
        ordering: Str(b"Identity"),
        supplement: 0,
    });
    cid.font_descriptor(refs.descriptor);
    cid.cid_to_gid_map_predefined(Name(b"Identity"));
    cid.default_width(0.0);
    write_widths(&mut cid, font, remapper);
    cid.finish();
}

/// Write the `/W` array: each subset glyph's advance in 1000-unit text space,
/// indexed by its new (remapped) glyph id, which equals its CID under
/// `Identity-H` + `CIDToGIDMap = Identity`.
fn write_widths(cid: &mut pdf_writer::writers::CidFont, font: &UsedFont, remapper: &GlyphRemapper) {
    let scale = 1000.0 / f32::from(font.face.units_per_em());
    let widths: Vec<f32> = remapper
        .remapped_gids()
        .map(|old| f32::from(font.face.glyph_advance(old)) * scale)
        .collect();
    cid.widths().consecutive(0, widths);
}

fn write_descriptor(chunk: &mut Chunk, refs: &FontRefs, face: &FontFace) {
    let name = base_font_name(face, refs.program);
    let scale = 1000.0 / f32::from(face.units_per_em());
    let (x0, y0, x1, y1) = face.bbox();
    let mut d = chunk.font_descriptor(refs.descriptor);
    d.name(Name(name.as_bytes()));
    d.flags(FontFlags::NON_SYMBOLIC);
    d.bbox(scaled_rect((x0, y0, x1, y1), scale));
    d.italic_angle(0.0);
    d.ascent(f32::from(face.ascent_units()) * scale);
    d.descent(f32::from(face.descent_units()) * scale);
    d.cap_height(f32::from(face.ascent_units()) * scale);
    d.stem_v(80.0);
    attach_program(&mut d, refs.program, face);
    d.finish();
}

fn scaled_rect(bbox: (i16, i16, i16, i16), scale: f32) -> Rect {
    Rect::new(
        f32::from(bbox.0) * scale,
        f32::from(bbox.1) * scale,
        f32::from(bbox.2) * scale,
        f32::from(bbox.3) * scale,
    )
}

/// Point the descriptor at the right font-file key for the program's flavor.
fn attach_program(d: &mut pdf_writer::writers::FontDescriptor, program: Ref, face: &FontFace) {
    if face.is_cff() {
        d.font_file3(program);
    } else {
        d.font_file2(program);
    }
}

/// Embed the subset program as a stream, tagging CFF subsets with the
/// `/Subtype /OpenType` that a `FontFile3` requires.
fn embed_program(chunk: &mut Chunk, refs: &FontRefs, face: &FontFace, subset: &[u8]) {
    let mut stream = chunk.stream(refs.program, subset);
    if face.is_cff() {
        stream.pair(Name(b"Subtype"), Name(b"OpenType"));
    }
    stream.finish();
}
