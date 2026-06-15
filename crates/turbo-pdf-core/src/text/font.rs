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
fn build_parsed(bytes: &[u8]) -> Result<Parsed<'_>, ()> {
    let ttf = TtfFace::parse(bytes, 0).map_err(|_| ())?;
    let rb = RbFace::from_slice(bytes, 0).expect("rustybuzz parses what ttf-parser validated");
    Ok(Parsed { ttf, rb })
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
        crate::hot!("font.face.build");
        let faces = OwnedFaces::try_new(data, |bytes| build_parsed(bytes)).ok()?;
        let (units_per_em, ascent, descent, line_gap) = metrics(faces.borrow_dependent());
        Some(FontFace {
            faces: Arc::new(faces),
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

    /// The raw font program bytes (the OpenType/TrueType file). The PDF emitter
    /// (Phase 9, §7) needs these to subset and embed the font program.
    pub fn data(&self) -> &[u8] {
        self.faces.borrow_owner()
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
        use std::collections::BTreeMap;
        let mut map: BTreeMap<u16, u32> = BTreeMap::new();
        self.fill_cmap_reverse(&mut map);
        glyphs
            .iter()
            .filter_map(|&g| map.get(&g).map(|&cp| (g, cp)))
            .collect()
    }

    /// Populate `out` with glyph-id → codepoint by walking every Unicode cmap
    /// subtable. Keeps the lowest codepoint per glyph (insert-if-absent in the
    /// ascending `codepoints` walk).
    #[cfg(feature = "pdf-ua")]
    fn fill_cmap_reverse(&self, out: &mut std::collections::BTreeMap<u16, u32>) {
        let face = self.ttf();
        let Some(cmap) = face.tables().cmap else {
            return;
        };
        for sub in cmap.subtables {
            if sub.is_unicode() {
                sub.codepoints(|cp| record_codepoint(&sub, cp, out));
            }
        }
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

/// Record `cp → glyph` reversed into `out`, keeping the first (lowest) codepoint
/// seen for each glyph so the `/ToUnicode` mapping is deterministic (`pdf-ua`).
#[cfg(feature = "pdf-ua")]
fn record_codepoint(
    sub: &rustybuzz::ttf_parser::cmap::Subtable,
    cp: u32,
    out: &mut std::collections::BTreeMap<u16, u32>,
) {
    if let Some(gid) = sub.glyph_index(cp) {
        out.entry(gid.0).or_insert(cp);
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
