//! Preserve original DOM units in cache keys while using the selected fallback
//! face’s real replacement glyph, as the UTF-16 shaping buffer does.
use super::{FontFace, ShapedGlyph};

impl FontFace {
    pub(crate) fn shape_original_utf16(&self, units: &[u16]) -> Vec<ShapedGlyph> {
        if let Some(hit) = self
            .shape_cache
            .lock()
            .expect("shape cache not poisoned")
            .units_entries
            .get(units)
            .cloned()
        {
            return hit;
        }
        let glyphs = self.shape_units_uncached(units);
        self.shape_cache
            .lock()
            .expect("shape cache not poisoned")
            .insert_units(units, &glyphs);
        glyphs
    }

    fn shape_units_uncached(&self, units: &[u16]) -> Vec<ShapedGlyph> {
        let mut output = Vec::new();
        let mut chunk = String::new();
        let mut byte_offset = 0;
        for decoded in char::decode_utf16(units.iter().copied()) {
            match decoded {
                Ok(character) => chunk.push(character),
                Err(error) => {
                    self.flush_units_chunk(&mut output, &mut chunk, &mut byte_offset);
                    output.push(self.unit_glyph(error.unpaired_surrogate(), byte_offset));
                    byte_offset += char::REPLACEMENT_CHARACTER.len_utf8();
                }
            }
        }
        self.flush_units_chunk(&mut output, &mut chunk, &mut byte_offset);
        output
    }

    fn flush_units_chunk(
        &self,
        output: &mut Vec<ShapedGlyph>,
        chunk: &mut String,
        offset: &mut usize,
    ) {
        output.extend(self.shape(chunk).into_iter().map(|mut glyph| {
            glyph.cluster += *offset as u32;
            glyph
        }));
        *offset += chunk.len();
        chunk.clear();
    }

    fn unit_glyph(&self, unit: u16, offset: usize) -> ShapedGlyph {
        // UTF-16 buffer decoding replaces the invalid scalar before shaping;
        // the host fallback hint used the original source identity separately.
        debug_assert!((0xd800..=0xdfff).contains(&unit));
        let mut glyph = self.shape("\u{fffd}")[0];
        glyph.cluster = offset as u32;
        glyph
    }

    pub(crate) fn measure_original_utf16(&self, units: &[u16], size: f32, spacing: f32) -> f32 {
        let glyphs = self.shape_original_utf16(units);
        let advance: i64 = glyphs.iter().map(|glyph| i64::from(glyph.x_advance)).sum();
        advance as f32 * self.scale(size) + spacing * glyphs.len() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn go() -> FontFace {
        FontFace::from_bytes(
            include_bytes!("../../assets/fonts/Go-Regular.ttf").to_vec(),
            "Go",
            400,
            false,
        )
        .unwrap()
    }
    #[test]
    fn original_units_keep_real_glyphs_and_distinct_replacement_cache_keys() {
        let face = go();
        let missing = face.shape_original_utf16(&[0xd800]);
        let replacement = face.shape_original_utf16(&[0xfffd]);
        assert_eq!(missing, replacement);
        assert_eq!(
            missing[0].x_advance,
            i32::from(
                face.ttf()
                    .glyph_hor_advance(rustybuzz::ttf_parser::GlyphId(missing[0].glyph_id))
                    .unwrap()
            )
        );
        assert_eq!(face.shape_original_utf16(&[0xd800]), missing);
        assert_eq!(face.shape_original_utf16(&[0xfffd]), replacement);
        assert!(face
            .shape_cache
            .lock()
            .unwrap()
            .units_entries
            .contains_key(&vec![0xd800]));
        assert!(face
            .shape_cache
            .lock()
            .unwrap()
            .units_entries
            .contains_key(&vec![0xfffd]));
    }
    #[test]
    fn valid_scalar_chunks_keep_shaper_offsets_around_distinct_original_lones() {
        let face = go();
        let actual = face.shape_original_utf16(&[0x61, 0xd800, 0x62, 0xdcff, 0x63]);
        assert_eq!(
            actual.iter().map(|glyph| glyph.cluster).collect::<Vec<_>>(),
            vec![0, 1, 4, 5, 8]
        );
        assert_eq!(face.measure_original_utf16(&[], 13.0, 0.0), 0.0);
        assert_eq!(
            face.shape_original_utf16(&[0x61, 0xd83d, 0xde00, 0x62]),
            face.shape("a😀b")
        );
    }
    #[test]
    fn raw_unit_cache_churn_and_oversize_release_payloads_without_changing_glyphs() {
        let face = go();
        let expected = face.shape_original_utf16(&[0xd800]);
        for unit in 0xd800..0xd890 {
            assert_eq!(face.shape_original_utf16(&[unit]), expected);
        }
        assert!(face.shape_cache.lock().unwrap().units_entries.len() <= 128);
        let original = vec![0x61; 450_000];
        let glyphs = face.shape_original_utf16(&original);
        assert_eq!(glyphs.len(), original.len());
        assert!(!face
            .shape_cache
            .lock()
            .unwrap()
            .units_entries
            .contains_key(&original));
        assert_eq!(face.shape_original_utf16(&[0xd800]), expected);
    }
    #[test]
    fn repeated_raw_shape_insertion_does_not_double_count_shared_payload() {
        let face = go();
        let units = [0x61, 0xd800, 0x62];
        let glyphs = face.shape_original_utf16(&units);
        let mut cache = face.shape_cache.lock().unwrap();
        let bytes = cache.bytes;
        cache.insert_units(&units, &glyphs);
        assert_eq!(cache.bytes, bytes);
        assert_eq!(cache.units_entries.len(), 1);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn original_units_follow_actual_host_fallback_glyph_tables() {
        let mut fonts = crate::FontRegistry::new();
        fonts.load_system_fonts();
        for family in ["Times New Roman", "Arial", "Courier New"] {
            let primary = fonts.select(&[family], 400, false).unwrap();
            let fallback = fonts
                .native_fallback(primary, char::REPLACEMENT_CHARACTER, 400, false)
                .unwrap();
            eprintln!(
                "{family}: {} {}",
                fallback.family(),
                fallback.measure_original_utf16(&[0xd800], 64.0, 0.0)
            );
            assert!(fallback.has_glyph(char::REPLACEMENT_CHARACTER));
            assert_eq!(
                fallback.shape_original_utf16(&[0xd800]),
                fallback.shape("\u{fffd}")
            );
        }
    }
}
