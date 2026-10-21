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

type NativeGlyphKey = (usize, u32, char, u16, bool);
type NativeGlyphCache = std::collections::HashMap<NativeGlyphKey, Option<FontFace>>;

/// A set of font faces: caller-supplied faces plus, when `bundled-fonts` is on,
/// the embedded fallback faces.
#[derive(Debug, Clone, Default)]
pub struct FontRegistry {
    /// Caller-registered faces, in registration order. Always preferred.
    faces: Vec<FontFace>,
    /// Embedded fallback faces (empty unless `bundled-fonts` is on). Consulted
    /// only after `faces`, so they never override a caller's font.
    bundled: Vec<FontFace>,
    platform_faces: Vec<FontFace>,
    platform_cascades:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<String>>>>,
    native_glyphs: std::sync::Arc<std::sync::Mutex<NativeGlyphCache>>,
}

fn html_generic_candidates<'a>(
    generic: &str,
    candidates: &'a [&'a str],
    prefer_native_mac_serif: bool,
) -> &'a [&'a str] {
    // Chrome's macOS serif default is the physical Times face. PDF callers
    // keep the prior Times New Roman preference and explicit families keep theirs.
    if prefer_native_mac_serif && generic == "serif" {
        &[
            "Times",
            "Times New Roman",
            "Georgia",
            "Liberation Serif",
            "DejaVu Serif",
        ]
    } else {
        candidates
    }
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
fn load_font_file(
    faces: &mut Vec<FontFace>,
    platform_faces: &mut Vec<FontFace>,
    path: &std::path::Path,
) {
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
    let bytes = std::sync::Arc::new(bytes);
    for i in 0..super::font::face_count(&bytes) {
        load_face(faces, platform_faces, &bytes, i);
    }
}

fn load_face(
    faces: &mut Vec<FontFace>,
    platform: &mut Vec<FontFace>,
    bytes: &std::sync::Arc<Vec<u8>>,
    index: u32,
) {
    let (target, description) = match super::font::describe(bytes, index) {
        Some(description) => (faces, Some(description)),
        None => (platform, super::font::describe_platform(bytes, index)),
    };
    let Some((family, weight, italic)) = description else {
        return;
    };
    if let Some(face) =
        FontFace::from_shared_bytes_index(bytes.clone(), index, family, weight, italic)
    {
        target.push(face);
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
            platform_faces: Vec::new(),
            platform_cascades: Default::default(),
            native_glyphs: Default::default(),
        }
    }

    pub fn add(&mut self, face: FontFace) {
        self.native_glyphs = Default::default();
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
        self.load_system_fonts_with_primary_policy(false);
    }

    /// Load installed fonts for continuous native HTML layout, including faces
    /// whose family exists only in platform name records. Choose generic aliases
    /// after admitting those real primary faces (for example Menlo on macOS).
    /// Ordinary PDF callers retain the policy of [`Self::load_system_fonts`].
    pub fn load_system_fonts_for_html(&mut self) {
        self.load_system_fonts_with_primary_policy(true);
    }

    fn load_system_fonts_with_primary_policy(&mut self, platform_primary: bool) {
        self.native_glyphs = Default::default();
        self.platform_cascades = Default::default();
        for dir in system_font_dirs() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                load_font_file(&mut self.faces, &mut self.platform_faces, &entry.path());
            }
        }
        if platform_primary {
            self.register_platform_primary();
            self.alias_generics_with_primary_policy(cfg!(target_os = "macos"));
        } else {
            self.alias_generics();
        }
    }

    fn register_platform_primary(&mut self) {
        self.faces.extend(self.platform_faces.iter().cloned());
    }

    /// Alias each CSS generic to the first installed candidate family (all its
    /// weights/styles), so `font-family: sans-serif` resolves to a system font.
    fn alias_generics(&mut self) {
        self.alias_generics_with_primary_policy(false);
    }

    fn alias_generics_with_primary_policy(&mut self, prefer_native_mac_serif: bool) {
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
            let candidates = html_generic_candidates(generic, candidates, prefer_native_mac_serif);
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

    fn native_families(&self, primary: &FontFace) -> Vec<String> {
        let mut cache = self
            .platform_cascades
            .lock()
            .expect("font cascade cache not poisoned");
        cache
            .entry(format!(
                "{}:{}",
                primary.data().as_ptr() as usize,
                primary.index()
            ))
            .or_insert_with(|| super::platform_fallback::families(primary))
            .clone()
    }

    pub(crate) fn native_fallback(
        &self,
        primary: &FontFace,
        ch: char,
        weight: u16,
        italic: bool,
    ) -> Option<FontFace> {
        let key = (
            primary.data().as_ptr() as usize,
            primary.index(),
            ch,
            weight,
            italic,
        );
        let mut cache = self
            .native_glyphs
            .lock()
            .expect("native font cache not poisoned");
        if let Some(face) = cache.get(&key) {
            return face.clone();
        }
        let face =
            self.glyph_for_native_families(&self.native_families(primary), ch, weight, italic);
        if cache.len() >= 512 {
            cache.clear();
        }
        cache.insert(key, face.clone());
        face
    }

    fn glyph_for_native_families(
        &self,
        families: &[String],
        ch: char,
        weight: u16,
        italic: bool,
    ) -> Option<FontFace> {
        families.iter().find_map(|family| {
            self.all()
                .chain(self.platform_faces.iter())
                .filter(|face| family_matches(face, family) && face.has_glyph(ch))
                .min_by_key(|face| score(face, weight, italic))
                .cloned()
        })
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
    fn native_glyph_fallback_cache_is_bounded_and_mutation_detaches_registry_clones() {
        let face = FontFace::from_bytes(
            include_bytes!("../../assets/fonts/Go-Regular.ttf").to_vec(),
            "Go",
            400,
            false,
        )
        .unwrap();
        let mut registry = FontRegistry::new();
        registry.add(face.clone());
        let expected = registry.native_fallback(&face, '\u{fffd}', 400, false);
        assert_eq!(
            registry
                .native_fallback(&face, '\u{fffd}', 400, false)
                .map(|font| font.family().to_owned()),
            expected.map(|font| font.family().to_owned())
        );
        for codepoint in 0x10000..0x10210 {
            let _ = registry.native_fallback(&face, char::from_u32(codepoint).unwrap(), 400, false);
        }
        assert!(registry.native_glyphs.lock().unwrap().len() <= 512);
        let cloned = registry.clone();
        let old_entries = cloned.native_glyphs.lock().unwrap().len();
        assert!(old_entries > 0);
        registry.add(face.with_family("alias"));
        assert_eq!(registry.native_glyphs.lock().unwrap().len(), 0);
        assert_eq!(cloned.native_glyphs.lock().unwrap().len(), old_entries);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn native_collection_fallback_uses_selected_physical_face_not_css_alias() {
        let bytes = std::fs::read("/System/Library/Fonts/Times.ttc").unwrap();
        let face = FontFace::from_bytes_index(bytes, 1, "an-author-alias", 400, false).unwrap();
        assert!(face.native_postscript_name().is_some());
        assert!(!super::super::platform_fallback::families(&face).is_empty());
        let go = FontFace::from_bytes(
            include_bytes!("../../assets/fonts/Go-Regular.ttf").to_vec(),
            "an-author-alias",
            400,
            false,
        )
        .unwrap();
        assert!(go.native_postscript_name().is_some());
        assert!(!super::super::platform_fallback::families(&go).is_empty());
    }
    #[test]
    fn platform_only_fonts_require_html_opt_in_and_keep_caller_priority() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut bytes =
            include_bytes!("../../assets/fonts/liberation-serif/LiberationSerif-Regular.ttf")
                .to_vec();
        let tables = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        let table = (0..tables)
            .map(|index| 12 + index * 16)
            .find(|&record| &bytes[record..record + 4] == b"name")
            .unwrap();
        let offset = u32::from_be_bytes(bytes[table + 8..table + 12].try_into().unwrap()) as usize;
        let count = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        for record in (0..count).map(|index| offset + 6 + index * 12) {
            let platform = u16::from_be_bytes([bytes[record], bytes[record + 1]]);
            let name = u16::from_be_bytes([bytes[record + 6], bytes[record + 7]]);
            if platform != 1 && matches!(name, 1 | 16) {
                bytes[record + 6..record + 8].copy_from_slice(&255u16.to_be_bytes());
            }
        }
        assert!(super::super::font::describe(&bytes, 0).is_none());
        let mut public = Vec::new();
        let mut private = Vec::new();
        load_face(&mut public, &mut private, &std::sync::Arc::new(bytes), 0);
        assert_eq!(public.len(), 0);
        assert_eq!(private.len(), 1);
        assert!(private[0].has_glyph('a'));
        let family = private[0].family().to_owned();
        let mut registry = FontRegistry::new();
        registry.platform_faces = private.clone();
        let actual = registry
            .glyph_for_native_families(&["absent".into(), family.clone()], 'a', 400, false)
            .unwrap();
        assert_eq!(actual.family(), family);
        assert_eq!(
            actual.measure("a", 64.0, 0.0),
            private[0].measure("a", 64.0, 0.0)
        );
        assert!(registry
            .glyph_for_native_families(&[family], '\u{10ffff}', 400, false)
            .is_none());
        let legacy = registry.select(&[private[0].family()], 400, false);
        assert!(legacy.is_none_or(|face| face.data() != private[0].data()));
        // A unique CSS alias keeps installed copies of Liberation Serif from
        // satisfying the request before the platform-only face is admitted.
        let alias = "Turbo HTML platform-only fixture";
        registry.platform_faces[0] = registry.platform_faces[0].with_family(alias);
        registry.load_system_fonts_for_html();
        let primary = registry.select(&[alias], 400, false).unwrap();
        assert_eq!(primary.family(), alias);
        assert_eq!(primary.data(), private[0].data());
        let caller = FontFace::from_bytes(
            include_bytes!("../../assets/fonts/Go-Regular.ttf").to_vec(),
            alias,
            400,
            false,
        )
        .unwrap();
        let mut with_caller = FontRegistry::new();
        with_caller.add(caller.clone());
        with_caller.platform_faces = vec![private[0].with_family(alias)];
        with_caller.load_system_fonts_for_html();
        assert_eq!(
            with_caller.select(&[alias], 400, false).unwrap().data(),
            caller.data()
        );
        load_face(
            &mut public,
            &mut private,
            &std::sync::Arc::new(b"invalid font".to_vec()),
            0,
        );
        assert_eq!(public.len(), 0);
        assert_eq!(private.len(), 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn html_system_generics_select_real_menlo_with_platform_only_family_records() {
        let _guard = ENV_LOCK.lock().unwrap();
        let bytes = std::fs::read("/System/Library/Fonts/Menlo.ttc").unwrap();
        assert!(super::super::font::describe(&bytes, 0).is_none());
        let mut registry = FontRegistry::new();
        registry.load_system_fonts_for_html();
        let explicit = registry.select(&["Menlo"], 400, false).unwrap();
        let generic = registry.select(&["monospace"], 400, false).unwrap();
        assert_eq!(explicit.data(), bytes);
        assert_eq!(generic.data(), explicit.data());
        assert_eq!(generic.index(), explicit.index());
        assert!(generic
            .native_postscript_name()
            .unwrap()
            .starts_with("Menlo"));
    }
    #[test]
    fn loaded_collection_shares_program_without_changing_registration_order() {
        let name = format!(
            "turbo-font-collection-{}-{}.ttc",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(name);
        let bytes = FontFace::collection_fixture_bytes();
        std::fs::write(&path, &bytes).unwrap();
        let mut faces = Vec::new();
        let mut platform = Vec::new();
        load_font_file(&mut faces, &mut platform, &path);
        std::fs::remove_file(path).unwrap();
        assert_eq!(faces.len(), 2);
        assert!(platform.is_empty());
        assert_eq!(faces[0].data().as_ptr(), faces[1].data().as_ptr());
        for (index, face) in faces.iter().enumerate() {
            let (family, weight, italic) =
                super::super::font::describe(&bytes, index as u32).unwrap();
            assert_eq!(face.index(), index as u32);
            assert_eq!(
                (face.family(), face.weight(), face.is_italic()),
                (family.as_str(), weight, italic)
            );
            assert_eq!(face.data(), bytes);
        }
        let mut registry = FontRegistry {
            faces,
            ..Default::default()
        };
        let cloned = registry.clone();
        registry.alias_generics();
        assert_eq!(
            cloned.faces[0].data().as_ptr(),
            registry.faces[0].data().as_ptr()
        );
        assert_eq!(registry.select(&["Roboto"], 400, false).unwrap().index(), 0);
        assert_eq!(registry.select(&["Go"], 400, false).unwrap().index(), 1);
    }

    #[test]
    fn load_font_file_covers_ext_read_and_parse() {
        // A real font file: extension ok, read ok, face parsed and pushed.
        let mut faces = Vec::new();
        load_font_file(
            &mut faces,
            &mut Vec::new(),
            std::path::Path::new(ROBOTO_PATH),
        );
        assert_eq!(faces.len(), 1, "one face loaded from Roboto");
        assert!(faces[0].family().eq_ignore_ascii_case("Roboto"));

        // A non-font extension is skipped before any read.
        let mut skipped = Vec::new();
        load_font_file(
            &mut skipped,
            &mut Vec::new(),
            std::path::Path::new("/some/where/readme.txt"),
        );
        assert!(skipped.is_empty());

        // A font-looking extension that cannot be read leaves the vec untouched.
        let mut unreadable = Vec::new();
        load_font_file(
            &mut unreadable,
            &mut Vec::new(),
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
    #[test]
    fn native_mac_serif_policy_selects_real_faces_without_changing_pdf_or_explicit_families() {
        let times = FontFace::from_bytes(ROBOTO.to_vec(), "Times", 400, false).unwrap();
        let tnr = FontFace::from_bytes(
            include_bytes!("../../assets/fonts/Go-Regular.ttf").to_vec(),
            "Times New Roman",
            400,
            false,
        )
        .unwrap();
        let mut original = FontRegistry::default();
        original.add(times.clone());
        original.add(tnr.clone());
        for (native, expected) in [(false, &tnr), (true, &times)] {
            let mut registry = original.clone();
            registry.alias_generics_with_primary_policy(native);
            assert_eq!(
                registry.select(&["serif"], 400, false).unwrap().data(),
                expected.data()
            );
            assert_eq!(
                registry
                    .select(&["Times New Roman"], 400, false)
                    .unwrap()
                    .data(),
                tnr.data()
            );
            assert_eq!(
                registry.select(&["Times"], 400, false).unwrap().data(),
                times.data()
            );
        }
        let caller = tnr.with_family("serif");
        original.add(caller.clone());
        original.alias_generics_with_primary_policy(true);
        assert_eq!(
            original.select(&["serif"], 400, false).unwrap().data(),
            caller.data()
        );
    }
}
