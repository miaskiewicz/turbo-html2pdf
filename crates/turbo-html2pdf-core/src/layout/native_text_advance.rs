//! Native HTML glyph advances; ordinary PDF shaping keeps its existing units.
//!
//! Chromium's Skia glyph callback converts SkScalar advances to signed 16.16
//! HarfBuzz positions by a clamped integer cast (truncation toward zero).
//! Source: chromium/chromium tag 155.0.8059.80, third_party/blink/renderer/
//! platform/fonts/skia/skia_text_metrics.h::SkiaScalarToHarfBuzzPosition.
//! Independently captured headed Chrome 155 font controls verify these advances.

use crate::text::FontFace;

pub(super) fn glyph_advance(
    face: &FontFace,
    font_size: f32,
    design_advance: i32,
    letter_spacing: f32,
) -> f64 {
    let scalar = face.scale(font_size) * design_advance as f32;
    let fixed = (scalar * 65536.0) as i32;
    f64::from(fixed) / 65536.0 + f64::from(letter_spacing)
}

pub(super) fn measure(
    face: &FontFace,
    font_size: f32,
    letter_spacing: f32,
    text: &str,
    original: Option<&[u16]>,
) -> f64 {
    let glyphs = original.map_or_else(
        || face.shape(text),
        |units| face.shape_original_utf16(units),
    );
    glyphs
        .iter()
        .map(|glyph| glyph_advance(face, font_size, glyph.x_advance, letter_spacing))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ibm() -> FontFace {
        FontFace::from_bytes(
            include_bytes!("../../assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf").to_vec(),
            "IBM Plex Mono",
            400,
            false,
        )
        .unwrap()
    }

    #[test]
    fn physical_glyph_advances_match_headed_chrome_without_layout_rounding() {
        let ibm = ibm();
        let fira = FontFace::from_bytes(
            include_bytes!("../../assets/fonts/fira-code/FiraCode-Regular.ttf").to_vec(),
            "Fira Code",
            400,
            false,
        )
        .unwrap();
        assert_eq!(measure(&ibm, 16.0, 0.0, "a", None), 9.599990844726562);
        assert_eq!(measure(&fira, 16.0, 0.0, "a", None), 9.846145629882812);
        assert_eq!(measure(&ibm, 16.0, 0.0, "aaaaa", None), 47.99995422363281);
        assert_eq!(measure(&ibm, 16.0, 0.0, "abc", None), 28.799972534179688);
        assert_ne!(
            measure(&ibm, 16.0, 0.0, "abc", None),
            3.0 * (9.6_f64 * 128.0).ceil() / 128.0
        );
    }

    #[test]
    fn signed_conversion_spacing_and_original_utf16_preserve_shaper_semantics() {
        let face = ibm();
        let advance = face.shape("a")[0].x_advance;
        let positive = glyph_advance(&face, 16.0, advance, 0.0);
        assert_eq!(glyph_advance(&face, 16.0, -advance, 0.0), -positive);
        assert_eq!(glyph_advance(&face, 16.0, advance, 0.25), positive + 0.25);
        assert_eq!(glyph_advance(&face, 16.0, 0, -0.25), -0.25);
        let original = [0x61, 0x62, 0x63];
        assert_eq!(
            measure(&face, 16.0, 0.0, "ignored", Some(&original)),
            measure(&face, 16.0, 0.0, "abc", None)
        );
        assert_eq!(
            measure(&face, 16.0, 0.25, "abc", None),
            measure(&face, 16.0, 0.0, "abc", None) + 0.75
        );
        assert_eq!(measure(&face, 16.0, 0.0, "", None), 0.0);
    }
}
