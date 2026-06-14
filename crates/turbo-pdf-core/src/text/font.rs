//! Font faces (§4.4): a caller-supplied face wrapping `ttf-parser` for metrics
//! and `rustybuzz` for shaping. The core embeds no fonts and does no system
//! lookup, so output is deterministic for identical inputs (AC-4.10).

use std::sync::Arc;

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

/// A loaded font face. Cheap to clone (the font bytes are shared).
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

    /// The glyph id for a character, if the face covers it.
    pub fn glyph_index(&self, ch: char) -> Option<u16> {
        self.ttf().glyph_index(ch).map(|g| g.0)
    }

    /// Whether the face has a glyph for `ch`.
    pub fn has_glyph(&self, ch: char) -> bool {
        self.glyph_index(ch).is_some()
    }

    /// Shape a run of text into positioned glyphs (design units).
    pub fn shape(&self, text: &str) -> Vec<ShapedGlyph> {
        let face = RbFace::from_slice(&self.data, 0).expect("font validated at construction");
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(text);
        let shaped = rustybuzz::shape(&face, &[], buffer);
        collect_glyphs(&shaped)
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
