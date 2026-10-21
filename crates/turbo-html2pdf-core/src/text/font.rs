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
    /// pair: the faces borrow the bytes). Collection indexes share the immutable
    /// program allocation while retaining separate parsed faces and shape caches.
    struct OwnedFaces {
        owner: Arc<Vec<u8>>,
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

/// Names exposed only through native platform fallback, avoiding a change to
/// the existing scalar/PDF registry's family selection.
pub(super) fn describe_platform(bytes: &[u8], index: u32) -> Option<(String, u16, bool)> {
    let face = TtfFace::parse(bytes, index).ok()?;
    Some((
        platform_family_name(&face)?,
        face.weight().to_number(),
        face.is_italic(),
    ))
}

fn platform_family_name(face: &TtfFace) -> Option<String> {
    let family = face.names().into_iter().find(|name| {
        name.name_id == 1
            && name.platform_id == rustybuzz::ttf_parser::name::PlatformId::Macintosh
            && name.encoding_id == 0
    })?;
    let (name, _, _) = encoding_rs::MACINTOSH.decode(family.name);
    Some(name.into_owned())
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

/// Native continuous-HTML font metrics, distinct from fractional PDF metrics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HtmlFontMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
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
    mac_ascent_adjustment: bool,
    /// Memoizes shaping by run text. Shaping is real per-run work, and layout
    /// shapes each run at least twice (measuring its width, then emitting its
    /// glyphs) plus once more for every repeated string (table cells, labels);
    /// caching by text collapses those to one shape per *distinct* run.
    shape_cache: Arc<Mutex<ShapeCache>>,
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

fn html_legacy_ascent(ascent: f32, descent: f32, enabled: bool) -> f32 {
    if enabled {
        ascent + ((ascent + descent) * 0.15 + 0.5).floor()
    } else {
        ascent
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
        Self::from_shared_bytes_index(Arc::new(data), index, family, weight, italic)
    }

    pub(super) fn from_shared_bytes_index(
        data: Arc<Vec<u8>>,
        index: u32,
        family: impl Into<String>,
        weight: u16,
        italic: bool,
    ) -> Option<FontFace> {
        crate::hot!("font.face.build");
        let faces = OwnedFaces::try_new(data, |bytes| build_parsed_index(bytes, index)).ok()?;
        let (units_per_em, ascent, descent, line_gap) = metrics(faces.borrow_dependent());
        let mac_ascent_adjustment = family_name(&faces.borrow_dependent().ttf)
            .or_else(|| platform_family_name(&faces.borrow_dependent().ttf))
            .is_some_and(|name| matches!(name.as_str(), "Times" | "Helvetica" | "Courier"));
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
            mac_ascent_adjustment,
            shape_cache: Arc::new(Mutex::new(ShapeCache::default())),
        })
    }

    /// Resolve actual font-table metrics using continuous HTML's integer ascent/
    /// descent policy. Blink155 FontMetrics::AscentDescentWithHacks rounds each
    /// independently; macOS's three named legacy families also adjust ascent.
    /// Attribution/source pins are in html-font-metrics-provenance.json and
    /// Blink-font-metrics.LICENSE. Existing PDF/tiny-font canvas metrics stay intact.
    pub fn html_metrics(&self, size: f32) -> HtmlFontMetrics {
        let ascent = self.ascent_px(size).round();
        let descent = self.descent_px(size).round();
        let ascent = html_legacy_ascent(
            ascent,
            descent,
            cfg!(target_os = "macos") && self.mac_ascent_adjustment,
        );
        HtmlFontMetrics {
            ascent,
            descent,
            line_gap: (f32::from(self.line_gap) * self.scale(size)).round(),
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn native_postscript_name(&self) -> Option<String> {
        self.ttf()
            .names()
            .into_iter()
            .filter(|name| name.name_id == 6)
            .find_map(|name| {
                if name.is_unicode() {
                    name.to_string()
                } else if name.platform_id == rustybuzz::ttf_parser::name::PlatformId::Macintosh
                    && name.encoding_id == 0
                {
                    Some(encoding_rs::MACINTOSH.decode(name.name).0.into_owned())
                } else {
                    None
                }
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
    /// given used glyphs, for the `/ToUnicode` CMap needed for text extraction.
    /// The first codepoint mapping to a glyph wins (deterministic: subtables and codepoints
    /// are iterated in table order). Glyphs with no Unicode mapping are omitted.
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
            .entries
            .get(text)
            .cloned()
    }

    fn store_shape(&self, text: &str, glyphs: &[ShapedGlyph]) {
        self.shape_cache
            .lock()
            .expect("shape cache not poisoned")
            .insert(text, glyphs);
    }

    /// Measure the advance width of `text` in pixels at `font_size`, adding
    /// `letter_spacing` pixels after each glyph.
    pub fn measure(&self, text: &str, font_size: f32, letter_spacing: f32) -> f32 {
        let glyphs = self.shape(text);
        let advance: i64 = glyphs.iter().map(|g| i64::from(g.x_advance)).sum();
        advance as f32 * self.scale(font_size) + letter_spacing * glyphs.len() as f32
    }
}

#[path = "font_utf16.rs"]
mod utf16;

const SHAPE_CACHE_ENTRIES: usize = 128;
const SHAPE_CACHE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Default)]
struct ShapeCache {
    entries: HashMap<String, Vec<ShapedGlyph>>,
    units_entries: HashMap<Vec<u16>, Vec<ShapedGlyph>>,
    bytes: usize,
}
impl ShapeCache {
    fn insert(&mut self, text: &str, glyphs: &[ShapedGlyph]) {
        let Some(bytes) = shape_payload_bytes(text, glyphs) else {
            return;
        };
        if self.entries.contains_key(text) {
            return;
        }
        self.make_room(bytes);
        self.entries.insert(text.to_string(), glyphs.to_vec());
        self.bytes += bytes;
    }
    fn insert_units(&mut self, units: &[u16], glyphs: &[ShapedGlyph]) {
        let Some(bytes) = unit_payload_bytes(units, glyphs) else {
            return;
        };
        if self.units_entries.contains_key(units) {
            return;
        }
        self.make_room(bytes);
        self.units_entries.insert(units.to_vec(), glyphs.to_vec());
        self.bytes += bytes;
    }
    fn make_room(&mut self, bytes: usize) {
        if self.entries.len() + self.units_entries.len() >= SHAPE_CACHE_ENTRIES
            || self.bytes + bytes > SHAPE_CACHE_BYTES
        {
            self.entries.clear();
            self.units_entries.clear();
            self.bytes = 0;
        }
    }
}
fn shape_payload_bytes(text: &str, glyphs: &[ShapedGlyph]) -> Option<usize> {
    let bytes = glyphs
        .len()
        .checked_mul(std::mem::size_of::<ShapedGlyph>())?
        .checked_add(text.len())?;
    (bytes <= SHAPE_CACHE_BYTES).then_some(bytes)
}

fn unit_payload_bytes(units: &[u16], glyphs: &[ShapedGlyph]) -> Option<usize> {
    let bytes = glyphs
        .len()
        .checked_mul(std::mem::size_of::<ShapedGlyph>())?
        .checked_add(units.len().checked_mul(2)?)?;
    (bytes <= SHAPE_CACHE_BYTES).then_some(bytes)
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
impl FontFace {
    pub(crate) fn collection_fixture_bytes() -> Vec<u8> {
        let fonts: [&[u8]; 2] = [
            include_bytes!("../../assets/fonts/roboto/Roboto-Regular.ttf"),
            include_bytes!("../../assets/fonts/Go-Regular.ttf"),
        ];
        let mut bytes = b"ttcf\0\x01\0\0\0\0\0\x02\0\0\0\0\0\0\0\0".to_vec();
        for (index, font) in fonts.into_iter().enumerate() {
            bytes.resize(bytes.len().next_multiple_of(4), 0);
            let start = bytes.len();
            bytes[12 + index * 4..16 + index * 4].copy_from_slice(&(start as u32).to_be_bytes());
            bytes.extend_from_slice(font);
            relocate_fixture_tables(&mut bytes[start..], start as u32);
        }
        bytes
    }

    pub(crate) fn collection_fixture_faces() -> [Self; 2] {
        let bytes = Arc::new(Self::collection_fixture_bytes());
        [0, 1].map(|index| {
            Self::from_shared_bytes_index(bytes.clone(), index, "same CSS alias", 400, false)
                .unwrap()
        })
    }
}

#[cfg(test)]
fn relocate_fixture_tables(font: &mut [u8], start: u32) {
    let count = u16::from_be_bytes([font[4], font[5]]) as usize;
    for table in (0..count).map(|index| 12 + index * 16 + 8) {
        let offset = u32::from_be_bytes(font[table..table + 4].try_into().unwrap());
        font[table..table + 4].copy_from_slice(&(offset + start).to_be_bytes());
    }
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
    fn public_vec_constructor_keeps_its_original_allocation() {
        let bytes = ROBOTO.to_vec();
        let pointer = bytes.as_ptr();
        let face = FontFace::from_bytes(bytes, "Roboto", 400, false).unwrap();
        assert_eq!(face.data().as_ptr(), pointer);
        assert!(face.has_glyph('A'));
    }

    #[test]
    fn shared_collection_retains_distinct_faces_and_original_font_results() {
        let faces = FontFace::collection_fixture_faces();
        assert_eq!(face_count(faces[0].data()), 2);
        assert_eq!(faces[0].data().as_ptr(), faces[1].data().as_ptr());
        assert_eq!([faces[0].index(), faces[1].index()], [0, 1]);
        let originals: [&[u8]; 2] = [ROBOTO, include_bytes!("../../assets/fonts/Go-Regular.ttf")];
        for (face, bytes) in faces.iter().zip(originals) {
            let expected = FontFace::from_bytes(bytes.to_vec(), "reference", 400, false).unwrap();
            assert_eq!(face.units_per_em(), expected.units_per_em());
            assert_eq!(face.html_metrics(18.72), expected.html_metrics(18.72));
            assert_eq!(face.bbox(), expected.bbox());
            assert_eq!(face.shape("AV office ffi"), expected.shape("AV office ffi"));
            assert_eq!(
                face.measure("AV office ffi", 18.72, 0.0),
                expected.measure("AV office ffi", 18.72, 0.0)
            );
        }
        assert_ne!(
            faces[0].shape("AV office ffi"),
            faces[1].shape("AV office ffi")
        );
        assert!(FontFace::from_shared_bytes_index(
            Arc::new(faces[0].data().to_vec()),
            2,
            "missing",
            400,
            false
        )
        .is_none());
        assert!(!Arc::ptr_eq(&faces[0].shape_cache, &faces[1].shape_cache));
        let alias = faces[1].with_family("retained alias");
        drop(faces);
        assert_eq!(alias.index(), 1);
        assert!(alias.has_glyph('A'));
        assert!(FontFace::from_shared_bytes_index(
            Arc::new(b"invalid".to_vec()),
            0,
            "bad",
            400,
            false
        )
        .is_none());
    }

    #[test]
    fn face_count_valid_and_garbage() {
        // A single-face .ttf reports one face.
        assert_eq!(face_count(ROBOTO), 1);
        // Non-font bytes: `fonts_in_collection` yields None → the `unwrap_or(1)`.
        assert_eq!(face_count(b"not a font"), 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_collection_name_decoding_accepts_unicode_and_rejects_other_legacy_encodings() {
        let face = FontFace::from_bytes(ROBOTO.to_vec(), "author alias", 400, false).unwrap();
        assert!(face.native_postscript_name().unwrap().contains("Roboto"));
        let mut bytes = ROBOTO.to_vec();
        let tables = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        let table = (0..tables)
            .map(|index| 12 + index * 16)
            .find(|&record| &bytes[record..record + 4] == b"name")
            .unwrap();
        let offset = u32::from_be_bytes(bytes[table + 8..table + 12].try_into().unwrap()) as usize;
        let count = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        for record in (0..count).map(|index| offset + 6 + index * 12) {
            if u16::from_be_bytes([bytes[record + 6], bytes[record + 7]]) == 6 {
                bytes[record..record + 2].copy_from_slice(&1u16.to_be_bytes());
                bytes[record + 2..record + 4].copy_from_slice(&1u16.to_be_bytes());
            }
        }
        let unsupported = FontFace::from_bytes(bytes, "author alias", 400, false).unwrap();
        assert_eq!(unsupported.native_postscript_name(), None);
    }
    #[test]
    fn platform_family_names_decode_real_legacy_font_tables_on_every_host() {
        let description = describe_platform(LIBERATION, 0).unwrap();
        assert!(description.0.to_ascii_lowercase().contains("liberation"));
        assert_eq!(description.1, 400);
        assert!(!description.2);
        assert!(describe_platform(b"not a font", 0).is_none());
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
    #[test]
    fn shared_shape_cache_releases_churn_without_changing_native_results() {
        let face = FontFace::from_bytes(ROBOTO.to_vec(), "Roboto", 400, false).unwrap();
        let alias = face.with_family("sans-serif");
        let expected = face.shape("retained glyph output");
        for index in 0..512 {
            alias.shape(&format!("unique actual run {index}"));
            let cache = face.shape_cache.lock().unwrap();
            assert!(cache.entries.len() <= SHAPE_CACHE_ENTRIES);
            assert!(cache.bytes <= SHAPE_CACHE_BYTES);
        }
        assert!(face.cached_shape("retained glyph output").is_none());
        assert_eq!(face.shape("retained glyph output"), expected);
    }

    #[test]
    fn oversized_real_shaping_results_are_returned_without_persistent_memoization() {
        let face = FontFace::from_bytes(ROBOTO.to_vec(), "Roboto", 400, false).unwrap();
        let input = "a".repeat(450_000);
        let glyphs = face.shape(&input);
        assert_eq!(glyphs.len(), input.len());
        assert!(glyphs.iter().all(|glyph| glyph.glyph_id != 0));
        assert!(face.cached_shape(&input).is_none());
        assert_eq!(face.shape_cache.lock().unwrap().bytes, 0);
    }
}

#[cfg(test)]
mod html_metric_policy_contracts {
    #[test]
    fn mac_legacy_policy_and_unmodified_platform_metrics_use_the_same_real_font_tables() {
        let face = super::FontFace::from_bytes(
            include_bytes!("../../assets/fonts/Go-Regular.ttf").to_vec(),
            "Go",
            400,
            false,
        )
        .unwrap();
        let ascent = face.ascent_px(64.0).round();
        let descent = face.descent_px(64.0).round();
        assert_eq!(super::html_legacy_ascent(ascent, descent, false), ascent);
        assert_eq!((ascent, descent), (60.0, 14.0));
        assert_eq!(super::html_legacy_ascent(ascent, descent, true), 71.0);
        assert_eq!(
            face.html_metrics(64.0).ascent,
            ascent,
            "Go is not a physical legacy-family face"
        );
    }
    #[test]
    fn duplicate_native_shaping_results_do_not_double_account_shared_cache_payloads() {
        let face = super::FontFace::from_bytes(
            include_bytes!("../../assets/fonts/Go-Regular.ttf").to_vec(),
            "Go",
            400,
            false,
        )
        .unwrap();
        let glyphs = face.shape("cache race");
        let original_bytes = face.shape_cache.lock().unwrap().bytes;
        face.shape_cache
            .lock()
            .unwrap()
            .insert("cache race", &glyphs);
        let cache = face.shape_cache.lock().unwrap();
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.bytes, original_bytes);
        drop(cache);
        assert_eq!(face.shape("cache race"), glyphs);
    }
}
