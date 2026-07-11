//! Font faces (§4.4): a caller-supplied face wrapping `ttf-parser` for metrics
//! and `rustybuzz` for shaping. The core embeds no fonts and does no system
//! lookup, so output is deterministic for identical inputs (AC-4.10).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rustybuzz::ttf_parser::Face as TtfFace;
use rustybuzz::{Face as RbFace, UnicodeBuffer};

/// The parsed faces borrowing the font program: `ttf-parser` for metrics/cmap
/// and `rustybuzz` for shaping. Parsing a font is the dominant per-call cost, so
/// both are built once and reused for every metric/coverage/shape query.
struct Parsed<'a> {
    ttf: TtfFace<'a>,
    rb: RbFace<'a>,
}

self_cell::self_cell!(
    /// Owns the font bytes and the faces parsed from them (a self-referential
    /// pair: the faces borrow the bytes). Built once per [`FontFace`].
    struct OwnedFaces {
        owner: Vec<u8>,
        #[covariant]
        dependent: Parsed,
    }
);

/// Parse the metric and shaping faces from font `bytes`; `Err` if the bytes are
/// not a valid font. `rustybuzz` accepts exactly what `ttf-parser` validated, so
/// once the metric face parses the shaping face is infallible.
/// Parse one face of a font (index `> 0` selects a face in a `.ttc` collection).
fn build_parsed_index(bytes: &[u8], index: u32) -> Result<Parsed<'_>, ()> {
    let ttf = TtfFace::parse(bytes, index).map_err(|_| ())?;
    let rb = RbFace::from_slice(bytes, index).expect("rustybuzz parses what ttf-parser validated");
    Ok(Parsed { ttf, rb })
}

/// Number of faces in a font file (`> 1` for a `.ttc` TrueType collection).
pub fn face_count(bytes: &[u8]) -> u32 {
    rustybuzz::ttf_parser::fonts_in_collection(bytes).unwrap_or(1)
}

/// Read a font face's own `(family, weight, italic)` from its name/OS-2 tables —
/// used to register installed system fonts under their real family. `None` if the
/// face (at `index`) doesn't parse or names no family.
pub fn describe(bytes: &[u8], index: u32) -> Option<(String, u16, bool)> {
    let face = TtfFace::parse(bytes, index).ok()?;
    let family = family_name(&face)?;
    Some((family, face.weight().to_number(), face.is_italic()))
}

/// The face's English family name: the typographic family (name id 16) if
/// present, else the legacy font family (name id 1).
fn family_name(face: &TtfFace) -> Option<String> {
    let mut legacy = None;
    for name in face.names() {
        if !name.is_unicode() {
            continue;
        }
        match (name.name_id, name.to_string()) {
            (16, Some(s)) => return Some(s),
            (1, Some(s)) => legacy = legacy.or(Some(s)),
            _ => {}
        }
    }
    legacy
}

/// Read the cached metrics `(units_per_em, ascent, descent, line_gap)`.
fn metrics(p: &Parsed) -> (u16, i16, i16, i16) {
    (
        p.ttf.units_per_em(),
        p.ttf.ascender(),
        p.ttf.descender(),
        p.ttf.line_gap(),
    )
}

/// A shaped glyph with positioning in font design units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShapedGlyph {
    pub glyph_id: u16,
    pub x_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
    pub cluster: u32,
}

/// A loaded font face. Cheap to clone (the parsed faces and the shape cache are
/// shared via `Arc`).
#[derive(Clone)]
pub struct FontFace {
    /// The font bytes parsed once into reusable `ttf-parser`/`rustybuzz` faces.
    faces: Arc<OwnedFaces>,
    /// The face index within the font program — `0` for a single-face `.ttf`/`.otf`,
    /// `> 0` for a face inside a `.ttc` collection (macOS ships Arial, Helvetica, …
    /// as collections). A consumer that re-parses `data()` to trace glyph outlines
    /// MUST parse at this index, else the glyph ids (which index the selected face's
    /// tables) draw the wrong sub-font's glyphs.
    index: u32,
    family: String,
    weight: u16,
    italic: bool,
    units_per_em: u16,
    ascent: i16,
    descent: i16,
    line_gap: i16,
    /// Memoizes shaping by run text. Shaping is real per-run work, and layout
    /// shapes each run at least twice (measuring its width, then emitting its
    /// glyphs) plus once more for every repeated string (table cells, labels);
    /// caching by text collapses those to one shape per *distinct* run.
    shape_cache: Arc<Mutex<HashMap<String, Vec<ShapedGlyph>>>>,
}

impl std::fmt::Debug for FontFace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontFace")
            .field("family", &self.family)
            .field("weight", &self.weight)
            .field("italic", &self.italic)
            .finish()
    }
}

impl FontFace {
    /// Load a face from font bytes, tagging it with a family/weight/style so the
    /// registry can select it. Returns `None` if the bytes are not a valid font.
    pub fn from_bytes(
        data: Vec<u8>,
        family: impl Into<String>,
        weight: u16,
        italic: bool,
    ) -> Option<FontFace> {
        FontFace::from_bytes_index(data, 0, family, weight, italic)
    }

    /// Like [`FontFace::from_bytes`] but selects face `index` of a `.ttc`
    /// collection (0 for a single-face `.ttf`/`.otf`).
    pub fn from_bytes_index(
        data: Vec<u8>,
        index: u32,
        family: impl Into<String>,
        weight: u16,
        italic: bool,
    ) -> Option<FontFace> {
        crate::hot!("font.face.build");
        let faces = OwnedFaces::try_new(data, |bytes| build_parsed_index(bytes, index)).ok()?;
        let (units_per_em, ascent, descent, line_gap) = metrics(faces.borrow_dependent());
        Some(FontFace {
            faces: Arc::new(faces),
            index,
            family: family.into(),
            weight,
            italic,
            units_per_em,
            ascent,
            descent,
            line_gap,
            shape_cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn family(&self) -> &str {
        &self.family
    }

    /// This face re-tagged with a different family name (cheap — the parsed font
    /// and shape cache are shared via `Arc`). Used to register a system font under
    /// a CSS generic (`sans-serif` → Helvetica).
    pub fn with_family(&self, family: impl Into<String>) -> FontFace {
        FontFace {
            family: family.into(),
            ..self.clone()
        }
    }

    /// The raw font program bytes (the OpenType/TrueType file). The PDF emitter
    /// (Phase 9, §7) needs these to subset and embed the font program.
    pub fn data(&self) -> &[u8] {
        self.faces.borrow_owner()
    }

    /// The face index within [`data()`](Self::data) — pass to `ttf-parser`/`rustybuzz`
    /// when re-parsing the program, so a `.ttc` collection face traces its own glyphs.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Design units per em, for scaling glyph metrics into PDF text space.
    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// Signed font bounding-box and metrics in design units, for the embedded
    /// `FontDescriptor` (§7). Returns `(x_min, y_min, x_max, y_max)`.
    pub fn bbox(&self) -> (i16, i16, i16, i16) {
        let r = self.ttf().global_bounding_box();
        (r.x_min, r.y_min, r.x_max, r.y_max)
    }

    /// The font ascender in design units (for the `FontDescriptor`).
    pub fn ascent_units(&self) -> i16 {
        self.ascent
    }

    /// The font descender in design units (negative; for the `FontDescriptor`).
    pub fn descent_units(&self) -> i16 {
        self.descent
    }

    /// The horizontal advance of a glyph in design units (for the CIDFont `/W`
    /// array). Falls back to 0 for a glyph the face has no advance for.
    pub fn glyph_advance(&self, glyph_id: u16) -> u16 {
        use rustybuzz::ttf_parser::GlyphId;
        self.ttf().glyph_hor_advance(GlyphId(glyph_id)).unwrap_or(0)
    }

    /// Reverse the font's Unicode `cmap` into a glyph-id → codepoint map for the
    /// given used glyphs, for the `/ToUnicode` CMap a tagged PDF needs so screen
    /// readers can extract text (`pdf-ua`, ISO 14289-1 §7.21.7). The first
    /// codepoint mapping to a glyph wins (deterministic: subtables and codepoints
    /// are iterated in table order). Glyphs with no Unicode mapping are omitted.
    #[cfg(feature = "pdf-ua")]
    pub fn glyph_to_unicode(&self, glyphs: &[u16]) -> Vec<(u16, u32)> {
        let map = crate::emit::tounicode::reverse_cmap(self.ttf());
        glyphs
            .iter()
            .filter_map(|&g| map.get(&g).map(|&cp| (g, cp)))
            .collect()
    }

    /// Whether the font program carries a CFF/CFF2 outline table (an OpenType/CFF
    /// font), as opposed to TrueType `glyf` outlines. The emitter embeds CFF as a
    /// `FontFile3` and TrueType as a `FontFile2` (§7).
    pub fn is_cff(&self) -> bool {
        let face = self.ttf();
        face.tables().cff.is_some()
    }

    pub fn weight(&self) -> u16 {
        self.weight
    }

    pub fn is_italic(&self) -> bool {
        self.italic
    }

    fn ttf(&self) -> &TtfFace<'_> {
        &self.faces.borrow_dependent().ttf
    }

    /// Scale factor converting design units to pixels at `font_size`.
    pub fn scale(&self, font_size: f32) -> f32 {
        font_size / f32::from(self.units_per_em)
    }

    /// Distance from baseline to the top of the line box, in pixels.
    pub fn ascent_px(&self, font_size: f32) -> f32 {
        f32::from(self.ascent) * self.scale(font_size)
    }

    /// Distance from baseline to the bottom (positive), in pixels.
    pub fn descent_px(&self, font_size: f32) -> f32 {
        -f32::from(self.descent) * self.scale(font_size)
    }

    /// Default line height (ascent + |descent| + line gap), in pixels.
    pub fn line_height_px(&self, font_size: f32) -> f32 {
        f32::from(self.ascent - self.descent + self.line_gap) * self.scale(font_size)
    }

    /// The glyph id for a character, if the face covers it. A direct cmap lookup
    /// on the parsed (cached) face — no per-call font parse, no lock.
    pub fn glyph_index(&self, ch: char) -> Option<u16> {
        self.ttf().glyph_index(ch).map(|g| g.0)
    }

    /// Whether the face has a glyph for `ch`.
    pub fn has_glyph(&self, ch: char) -> bool {
        self.glyph_index(ch).is_some()
    }

    /// Shape a run of text into positioned glyphs (design units). Memoized by
    /// text: shaping re-parses the face, and the same run is shaped repeatedly
    /// (width measurement, then emission, plus duplicate strings). The lock is
    /// held only around the cache get/put, never across shaping itself.
    pub fn shape(&self, text: &str) -> Vec<ShapedGlyph> {
        if let Some(hit) = self.cached_shape(text) {
            return hit;
        }
        let glyphs = self.shape_uncached(text);
        self.store_shape(text, &glyphs);
        glyphs
    }

    fn shape_uncached(&self, text: &str) -> Vec<ShapedGlyph> {
        crate::hot!("font.shape");
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(text);
        let shaped = rustybuzz::shape(&self.faces.borrow_dependent().rb, &[], buffer);
        collect_glyphs(&shaped)
    }

    fn cached_shape(&self, text: &str) -> Option<Vec<ShapedGlyph>> {
        self.shape_cache
            .lock()
            .expect("shape cache not poisoned")
            .get(text)
            .cloned()
    }

    fn store_shape(&self, text: &str, glyphs: &[ShapedGlyph]) {
        self.shape_cache
            .lock()
            .expect("shape cache not poisoned")
            .insert(text.to_string(), glyphs.to_vec());
    }

    /// Measure the advance width of `text` in pixels at `font_size`, adding
    /// `letter_spacing` pixels after each glyph.
    pub fn measure(&self, text: &str, font_size: f32, letter_spacing: f32) -> f32 {
        let glyphs = self.shape(text);
        let advance: i64 = glyphs.iter().map(|g| i64::from(g.x_advance)).sum();
        advance as f32 * self.scale(font_size) + letter_spacing * glyphs.len() as f32
    }
}

fn collect_glyphs(shaped: &rustybuzz::GlyphBuffer) -> Vec<ShapedGlyph> {
    let infos = shaped.glyph_infos();
    let positions = shaped.glyph_positions();
    infos
        .iter()
        .zip(positions)
        .map(|(info, pos)| ShapedGlyph {
            glyph_id: info.glyph_id as u16,
            x_advance: pos.x_advance,
            x_offset: pos.x_offset,
            y_offset: pos.y_offset,
            cluster: info.cluster,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROBOTO: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/fonts/roboto/Roboto-Regular.ttf"
    ));
    // Liberation Serif carries non-Unicode (Macintosh) name records, so it drives
    // the `!name.is_unicode()` skip branch in `family_name`.
    const LIBERATION: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/fonts/liberation-serif/LiberationSerif-Regular.ttf"
    ));

    /// Relabel the first Unicode `name id 1` (legacy family) record to `name id
    /// 16` (typographic family), so `family_name` takes its `(16, Some(_))` arm.
    /// None of the bundled faces ship a name id 16, so we synthesize one.
    fn relabel_family_to_typographic(mut bytes: Vec<u8>) -> Vec<u8> {
        let num_tables = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        let name_off = (0..num_tables)
            .map(|i| 12 + i * 16)
            .find(|&rec| &bytes[rec..rec + 4] == b"name")
            .map(|rec| {
                u32::from_be_bytes([
                    bytes[rec + 8],
                    bytes[rec + 9],
                    bytes[rec + 10],
                    bytes[rec + 11],
                ]) as usize
            })
            .expect("font has a name table");
        let count = u16::from_be_bytes([bytes[name_off + 2], bytes[name_off + 3]]) as usize;
        // The first Unicode(0)/Windows(3)-platform name-id-1 (family) record.
        let rec = (0..count)
            .map(|r| name_off + 6 + r * 12)
            .find(|&rec| {
                let platform = u16::from_be_bytes([bytes[rec], bytes[rec + 1]]);
                let name_id = u16::from_be_bytes([bytes[rec + 6], bytes[rec + 7]]);
                name_id == 1 && (platform == 0 || platform == 3)
            })
            .expect("no Unicode name id 1 record to relabel");
        bytes[rec + 6] = 0x00;
        bytes[rec + 7] = 16; // relabel name-id 1 → 16 (typographic family)
        bytes
    }

    #[test]
    fn face_count_valid_and_garbage() {
        // A single-face .ttf reports one face.
        assert_eq!(face_count(ROBOTO), 1);
        // Non-font bytes: `fonts_in_collection` yields None → the `unwrap_or(1)`.
        assert_eq!(face_count(b"not a font"), 1);
    }

    #[test]
    fn describe_reads_family_weight_italic() {
        let (family, weight, italic) = describe(ROBOTO, 0).expect("Roboto describes");
        assert!(family.eq_ignore_ascii_case("Roboto"), "family = {family}");
        assert!(weight > 0);
        assert!(!italic);
        // Liberation Serif exercises the non-Unicode name-record skip.
        let (lib_family, _, _) = describe(LIBERATION, 0).expect("Liberation describes");
        assert!(lib_family.to_ascii_lowercase().contains("liberation"));
        // Garbage bytes: parse fails → None (the `?` short-circuit).
        assert!(describe(b"still not a font", 0).is_none());
    }

    #[test]
    fn describe_prefers_typographic_family_name_id_16() {
        // With a synthesized name id 16, `family_name` returns via its (16, _) arm.
        let patched = relabel_family_to_typographic(ROBOTO.to_vec());
        let (family, _, _) = describe(&patched, 0).expect("patched font describes");
        assert!(family.eq_ignore_ascii_case("Roboto"), "family = {family}");
    }

    #[test]
    fn with_family_retags_same_program() {
        let face = FontFace::from_bytes(ROBOTO.to_vec(), "Roboto", 400, false).expect("load");
        let retagged = face.with_family("sans-serif");
        assert_eq!(retagged.family(), "sans-serif");
        // Same underlying program bytes are shared.
        assert_eq!(retagged.data(), face.data());
        // The face index (0 for a single-face .ttf) survives the retag, so a consumer
        // re-parsing the shared bytes selects the same face.
        assert_eq!(face.index(), 0);
        assert_eq!(retagged.index(), face.index());
    }
}
