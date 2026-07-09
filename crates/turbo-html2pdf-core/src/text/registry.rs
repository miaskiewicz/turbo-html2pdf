//! Font registry + fallback chain (§4.4). Selects a face for a `font-family`
//! list by weight/style closeness, and resolves per-character fallback so a
//! glyph missing from the primary face is found in a later one. A glyph absent
//! from every face yields `None`, which the caller turns into `.notdef` + a lint.
//!
//! ## Bundled fallbacks (`bundled-fonts`, default-on)
//!
//! When the `bundled-fonts` feature is on, the registry is born with a set of
//! embedded OFL faces (see [`super::bundled`]) so a document renders with zero
//! caller-supplied fonts. They are kept in a *separate* list consulted only after
//! every caller face, so a caller that registers its own faces always wins
//! (requirement: bundled faces are fallbacks, never overrides). The CSS generic
//! keywords (`sans-serif`/`serif`/`monospace`) are expanded to the bundled real
//! family names here, so `font-family: sans-serif` selects Inter then Roboto.
//!
//! When the feature is off, the bundled list is always empty and every code path
//! below behaves exactly as the no-bundled build.

use super::font::FontFace;

/// A set of font faces: caller-supplied faces plus, when `bundled-fonts` is on,
/// the embedded fallback faces.
#[derive(Debug, Clone, Default)]
pub struct FontRegistry {
    /// Caller-registered faces, in registration order. Always preferred.
    faces: Vec<FontFace>,
    /// Embedded fallback faces (empty unless `bundled-fonts` is on). Consulted
    /// only after `faces`, so they never override a caller's font.
    bundled: Vec<FontFace>,
}

fn family_matches(face: &FontFace, name: &str) -> bool {
    face.family().eq_ignore_ascii_case(name.trim())
}

/// The conventional installed-font directories for the host OS.
fn system_font_dirs() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let mut dirs = vec![
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/System/Library/Fonts/Supplemental"),
        PathBuf::from("/Library/Fonts"),
    ];
    #[cfg(target_os = "macos")]
    if let Some(h) = &home {
        dirs.push(h.join("Library/Fonts"));
    }
    #[cfg(target_os = "linux")]
    let mut dirs = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
    ];
    #[cfg(target_os = "linux")]
    if let Some(h) = &home {
        dirs.push(h.join(".fonts"));
        dirs.push(h.join(".local/share/fonts"));
    }
    #[cfg(target_os = "windows")]
    let dirs = vec![PathBuf::from(
        std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into()) + "\\Fonts",
    )];
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let dirs: Vec<PathBuf> = Vec::new();
    let _ = &home;
    dirs
}

/// Load every face of one font file (`.ttf`/`.otf`/`.ttc`) into `faces`, each
/// under its own family/weight/style read from the font. Non-font files skipped.
fn load_font_file(faces: &mut Vec<FontFace>, path: &std::path::Path) {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    if !matches!(ext.as_deref(), Some("ttf" | "otf" | "ttc")) {
        return;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    for i in 0..super::font::face_count(&bytes) {
        if let Some((family, weight, italic)) = super::font::describe(&bytes, i) {
            if let Some(face) = FontFace::from_bytes_index(bytes.clone(), i, family, weight, italic)
            {
                faces.push(face);
            }
        }
    }
}

fn score(face: &FontFace, weight: u16, italic: bool) -> u32 {
    let weight_diff = (i32::from(face.weight()) - i32::from(weight)).unsigned_abs();
    let style_penalty = if face.is_italic() == italic { 0 } else { 1000 };
    weight_diff + style_penalty
}

/// Expand a CSS family name to the concrete family names to try. A generic
/// keyword (`sans-serif`/`serif`/`monospace`) expands to the bundled primary +
/// secondary real family names; anything else is itself. With the feature off
/// the bundled table is empty, so a generic keyword expands to nothing extra and
/// behaviour matches the no-bundled build.
fn expand_family(name: &str) -> Vec<&str> {
    #[cfg(feature = "bundled-fonts")]
    {
        let key = name.trim();
        for (generic, reals) in super::bundled::GENERICS {
            if key.eq_ignore_ascii_case(generic) {
                // Try a directly-registered generic first (system fonts loaded via
                // `load_system_fonts` alias `sans-serif` etc. to a system family),
                // then the bundled reals.
                let mut v = vec![name];
                v.extend_from_slice(reals);
                return v;
            }
        }
    }
    vec![name]
}

impl FontRegistry {
    /// A registry with no caller faces. Carries the bundled fallback faces when
    /// the `bundled-fonts` feature is on, so it can render without any caller
    /// font; identical to [`FontRegistry::default`] otherwise.
    pub fn new() -> Self {
        Self {
            faces: Vec::new(),
            bundled: bundled_faces(),
        }
    }

    pub fn add(&mut self, face: FontFace) {
        self.faces.push(face);
    }

    /// Register every installed system font (opt-in — **not** part of `new()`;
    /// default rendering, e.g. PDF generation, uses only the shipped/bundled
    /// faces). Walks the OS font directories, registering each face under its own
    /// family, and aliases the CSS generics to the conventional system families
    /// (`sans-serif`→Helvetica/Arial, `serif`→Times, `monospace`→Menlo/Courier) so
    /// a page that names an installed font — or a generic — renders in it, matching
    /// a browser on the same machine. Best-effort: unreadable dirs/files skipped.
    pub fn load_system_fonts(&mut self) {
        for dir in system_font_dirs() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                load_font_file(&mut self.faces, &entry.path());
            }
        }
        self.alias_generics();
    }

    /// Alias each CSS generic to the first installed candidate family (all its
    /// weights/styles), so `font-family: sans-serif` resolves to a system font.
    fn alias_generics(&mut self) {
        const GENERIC_CANDIDATES: &[(&str, &[&str])] = &[
            (
                "sans-serif",
                &[
                    "Helvetica",
                    "Helvetica Neue",
                    "Arial",
                    "Liberation Sans",
                    "DejaVu Sans",
                ],
            ),
            (
                "serif",
                &[
                    "Times New Roman",
                    "Times",
                    "Georgia",
                    "Liberation Serif",
                    "DejaVu Serif",
                ],
            ),
            (
                "monospace",
                &[
                    "Menlo",
                    "Courier New",
                    "Courier",
                    "Monaco",
                    "DejaVu Sans Mono",
                ],
            ),
        ];
        for (generic, candidates) in GENERIC_CANDIDATES {
            if let Some(cand) = candidates
                .iter()
                .find(|c| self.faces.iter().any(|f| family_matches(f, c)))
            {
                let aliased: Vec<FontFace> = self
                    .faces
                    .iter()
                    .filter(|f| family_matches(f, cand))
                    .map(|f| f.with_family(*generic))
                    .collect();
                self.faces.extend(aliased);
            }
        }
    }

    /// True when the registry has no usable face at all (neither caller nor
    /// bundled). With `bundled-fonts` on this is only true if the bundled set
    /// failed to load, which never happens for the shipped assets.
    pub fn is_empty(&self) -> bool {
        self.faces.is_empty() && self.bundled.is_empty()
    }

    /// The number of caller-supplied faces. Bundled fallbacks are not counted,
    /// so a caller can tell whether *it* registered anything.
    pub fn len(&self) -> usize {
        self.faces.len()
    }

    /// All faces in lookup order: caller faces first, then bundled fallbacks.
    fn all(&self) -> impl Iterator<Item = &FontFace> {
        self.faces.iter().chain(self.bundled.iter())
    }

    fn best_in_family(&self, name: &str, weight: u16, italic: bool) -> Option<&FontFace> {
        self.all()
            .filter(|f| family_matches(f, name))
            .min_by_key(|f| score(f, weight, italic))
    }

    /// Select the best face for a family list + weight/style, falling back to the
    /// first available face (caller, else bundled) if no family matches.
    pub fn select(&self, families: &[&str], weight: u16, italic: bool) -> Option<&FontFace> {
        families
            .iter()
            .flat_map(|fam| expand_family(fam))
            .find_map(|fam| self.best_in_family(fam, weight, italic))
            .or_else(|| self.all().next())
    }

    fn glyph_in_family(
        &self,
        name: &str,
        weight: u16,
        italic: bool,
        ch: char,
    ) -> Option<&FontFace> {
        self.all()
            .filter(|f| family_matches(f, name) && f.has_glyph(ch))
            .min_by_key(|f| score(f, weight, italic))
    }

    /// Resolve the face that should render `ch`, walking the (expanded) family
    /// list then any available face. Returns `None` if no face covers it.
    pub fn resolve_glyph(
        &self,
        families: &[&str],
        weight: u16,
        italic: bool,
        ch: char,
    ) -> Option<&FontFace> {
        for fam in families.iter().flat_map(|fam| expand_family(fam)) {
            if let Some(face) = self.glyph_in_family(fam, weight, italic, ch) {
                return Some(face);
            }
        }
        self.all().find(|f| f.has_glyph(ch))
    }
}

/// The bundled fallback faces for a fresh registry: the embedded set when
/// `bundled-fonts` is on, empty otherwise.
#[cfg(feature = "bundled-fonts")]
fn bundled_faces() -> Vec<FontFace> {
    super::bundled::bundled_faces()
}

#[cfg(not(feature = "bundled-fonts"))]
fn bundled_faces() -> Vec<FontFace> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::font::FontFace;

    const ROBOTO_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/fonts/roboto/Roboto-Regular.ttf"
    );
    const ROBOTO: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/fonts/roboto/Roboto-Regular.ttf"
    ));

    // `system_font_dirs`, `load_system_fonts` and `system_font_dirs` all read the
    // `HOME` env var; serialize the tests that mutate it to avoid a data race.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn system_font_dirs_lists_platform_dirs() {
        let _g = ENV_LOCK.lock().unwrap();
        let dirs = system_font_dirs();
        // Every supported target contributes at least one directory; the fallback
        // arm for other targets is empty, but tests run on mac/linux/windows.
        assert!(!dirs.is_empty(), "expected some system font dirs");
    }

    #[test]
    fn load_font_file_covers_ext_read_and_parse() {
        // A real font file: extension ok, read ok, face parsed and pushed.
        let mut faces = Vec::new();
        load_font_file(&mut faces, std::path::Path::new(ROBOTO_PATH));
        assert_eq!(faces.len(), 1, "one face loaded from Roboto");
        assert!(faces[0].family().eq_ignore_ascii_case("Roboto"));

        // A non-font extension is skipped before any read.
        let mut skipped = Vec::new();
        load_font_file(&mut skipped, std::path::Path::new("/some/where/readme.txt"));
        assert!(skipped.is_empty());

        // A font-looking extension that cannot be read leaves the vec untouched.
        let mut unreadable = Vec::new();
        load_font_file(
            &mut unreadable,
            std::path::Path::new("/nonexistent-dir/missing-font.ttf"),
        );
        assert!(unreadable.is_empty());
    }

    #[test]
    fn load_system_fonts_walks_dirs_and_aliases() {
        let _g = ENV_LOCK.lock().unwrap();
        // Point HOME at a directory with no `Library/Fonts` (or `.fonts`) child so
        // that at least one candidate dir fails `read_dir` → the `continue` arm.
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", "/nonexistent-home-for-turbo-html2pdf-tests");
        let mut reg = FontRegistry::new();
        reg.load_system_fonts();
        match prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        // Best-effort: it must not panic. On CI the system dirs may be empty, so we
        // only assert it ran (the registry is still usable via bundled fallbacks).
        let _ = reg.is_empty();
    }

    #[test]
    fn alias_generics_maps_generic_to_installed_family() {
        // Register a face under a family a generic-candidate list names, then alias.
        let helvetica =
            FontFace::from_bytes(ROBOTO.to_vec(), "Helvetica", 400, false).expect("load");
        let mut reg = FontRegistry::default();
        reg.add(helvetica);
        let before = reg.len();
        reg.alias_generics();
        // `sans-serif` matched "Helvetica" → an aliased face was appended; `serif`
        // and `monospace` matched nothing → their `if let Some` arms were skipped.
        assert!(reg.len() > before, "an aliased generic face was added");
        assert!(
            reg.select(&["sans-serif"], 400, false).is_some(),
            "sans-serif now resolves to the aliased Helvetica face"
        );
    }
}
