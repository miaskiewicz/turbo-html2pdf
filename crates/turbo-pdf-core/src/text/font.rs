//! Font faces (§4.4): a caller-supplied face wrapping `ttf-parser` for metrics
//! and `rustybuzz` for shaping. The core embeds no fonts and does no system
//! lookup, so output is deterministic for identical inputs (AC-4.10).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rustybuzz::ttf_parser::Face as TtfFace;
use rustybuzz::{Face as RbFace, UnicodeBuffer};

/// A shaped glyph with positioning in font design units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShapedGlyph {
    pub glyph_id: u16,
    pub x_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
    pub cluster: u32,
}

/// A loaded font face. Cheap to clone (the font bytes and the glyph-lookup cache
/// are shared via `Arc`).
#[derive(Debug, Clone)]
pub struct FontFace {
    data: Arc<Vec<u8>>,
    family: String,
    weight: u16,
    italic: bool,
    units_per_em: u16,
    ascent: i16,
    descent: i16,
    line_gap: i16,
    /// Memoizes `char -> glyph id` lookups. Resolving a glyph requires parsing
    /// the font tables; layout queries coverage once per *character occurrence*
    /// (e.g. every glyph of every row), so without this each query re-parsed the
    /// whole face. The cache collapses that to one parse per *distinct* char and
    /// is shared across clones (all clones of a face share one font program).
    glyph_cache: Arc<Mutex<HashMap<char, Option<u16>>>>,
    /// Memoizes shaping by run text. Shaping re-parses the face, and layout
    /// shapes each run at least twice (measuring its width, then emitting its
    /// glyphs) plus once more for every repeated string (table cells, labels);
    /// caching by text collapses those to one shape per *distinct* run.
    shape_cache: Arc<Mutex<HashMap<String, Vec<ShapedGlyph>>>>,
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
        let face = TtfFace::parse(&data, 0).ok()?;
        let units_per_em = face.units_per_em();
        let (ascent, descent, line_gap) = (face.ascender(), face.descender(), face.line_gap());
        Some(FontFace {
            data: Arc::new(data),
            family: family.into(),
            weight,
            italic,
            units_per_em,
            ascent,
            descent,
            line_gap,
            glyph_cache: Arc::new(Mutex::new(HashMap::new())),
            shape_cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn family(&self) -> &str {
        &self.family
    }

    /// The raw font program bytes (the OpenType/TrueType file). The PDF emitter
    /// (Phase 9, §7) needs these to subset and embed the font program.
    pub fn data(&self) -> &[u8] {
        &self.data
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

    fn ttf(&self) -> TtfFace<'_> {
        TtfFace::parse(&self.data, 0).expect("font validated at construction")
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

    /// The glyph id for a character, if the face covers it. Memoized: the first
    /// query for a given char parses the face's cmap, subsequent queries are a
    /// hash lookup (layout asks per character, so this is the hot path). The lock
    /// is held only for the lookup/store, never across the parse, so concurrent
    /// renders on a shared face still proceed in parallel.
    pub fn glyph_index(&self, ch: char) -> Option<u16> {
        if let Some(hit) = self.cached_glyph(ch) {
            return hit;
        }
        let resolved = self.ttf().glyph_index(ch).map(|g| g.0);
        self.store_glyph(ch, resolved);
        resolved
    }

    fn cached_glyph(&self, ch: char) -> Option<Option<u16>> {
        self.glyph_cache
            .lock()
            .expect("glyph cache not poisoned")
            .get(&ch)
            .copied()
    }

    fn store_glyph(&self, ch: char, resolved: Option<u16>) {
        self.glyph_cache
            .lock()
            .expect("glyph cache not poisoned")
            .insert(ch, resolved);
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
        let face = RbFace::from_slice(&self.data, 0).expect("font validated at construction");
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(text);
        let shaped = rustybuzz::shape(&face, &[], buffer);
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
