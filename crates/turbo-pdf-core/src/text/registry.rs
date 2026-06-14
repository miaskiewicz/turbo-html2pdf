//! Font registry + fallback chain (§4.4). Selects a face for a `font-family`
//! list by weight/style closeness, and resolves per-character fallback so a
//! glyph missing from the primary face is found in a later one. A glyph absent
//! from every face yields `None`, which the caller turns into `.notdef` + a lint.

use super::font::FontFace;

/// A set of caller-supplied font faces.
#[derive(Debug, Clone, Default)]
pub struct FontRegistry {
    faces: Vec<FontFace>,
}

fn family_matches(face: &FontFace, name: &str) -> bool {
    face.family().eq_ignore_ascii_case(name.trim())
}

fn score(face: &FontFace, weight: u16, italic: bool) -> u32 {
    let weight_diff = (i32::from(face.weight()) - i32::from(weight)).unsigned_abs();
    let style_penalty = if face.is_italic() == italic { 0 } else { 1000 };
    weight_diff + style_penalty
}

impl FontRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, face: FontFace) {
        self.faces.push(face);
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    pub fn len(&self) -> usize {
        self.faces.len()
    }

    fn best_in_family(&self, name: &str, weight: u16, italic: bool) -> Option<&FontFace> {
        self.faces
            .iter()
            .filter(|f| family_matches(f, name))
            .min_by_key(|f| score(f, weight, italic))
    }

    /// Select the best face for a family list + weight/style, falling back to the
    /// first registered face if no family matches.
    pub fn select(&self, families: &[&str], weight: u16, italic: bool) -> Option<&FontFace> {
        families
            .iter()
            .find_map(|fam| self.best_in_family(fam, weight, italic))
            .or_else(|| self.faces.first())
    }

    fn glyph_in_family(
        &self,
        name: &str,
        weight: u16,
        italic: bool,
        ch: char,
    ) -> Option<&FontFace> {
        self.faces
            .iter()
            .filter(|f| family_matches(f, name) && f.has_glyph(ch))
            .min_by_key(|f| score(f, weight, italic))
    }

    /// Resolve the face that should render `ch`, walking the family list then any
    /// registered face. Returns `None` if no face covers the character.
    pub fn resolve_glyph(
        &self,
        families: &[&str],
        weight: u16,
        italic: bool,
        ch: char,
    ) -> Option<&FontFace> {
        for fam in families {
            if let Some(face) = self.glyph_in_family(fam, weight, italic, ch) {
                return Some(face);
            }
        }
        self.faces.iter().find(|f| f.has_glyph(ch))
    }
}
