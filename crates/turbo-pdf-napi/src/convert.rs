//! Pure conversions between core types and the N-API wire shapes: diagnostics,
//! lint codes, the font registry assembled from caller-supplied faces, and the
//! name-keyed image resolver assembled from caller-supplied rasters.

use std::collections::HashMap;

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

use turbo_html2pdf_core::{Diagnostics, FontFace, FontRegistry, ImageResolver, Lint, LintCode};

/// A non-fatal diagnostic (lint) returned in the render result, never thrown.
#[napi(object)]
pub struct JsDiagnostic {
    /// Stable lint code (e.g. `"UnsupportedCss"`, `"RegionOverflow"`).
    pub code: String,
    /// Human-readable description.
    pub message: String,
    /// 1-based source line (0 when unknown).
    pub line: u32,
    /// 1-based source column (0 when unknown).
    pub col: u32,
}

/// The stable string form of a [`LintCode`] (mirrors the variant name).
fn lint_code_str(code: LintCode) -> &'static str {
    match code {
        LintCode::UnsupportedCss => "UnsupportedCss",
        LintCode::NonScalarInterpolation => "NonScalarInterpolation",
        LintCode::RawOutput => "RawOutput",
        LintCode::RegionOverflow => "RegionOverflow",
        LintCode::NotdefGlyph => "NotdefGlyph",
        LintCode::FootnoteConvergence => "FootnoteConvergence",
    }
}

/// Convert one core lint into its JS wire shape.
fn lint_to_js(lint: &Lint) -> JsDiagnostic {
    JsDiagnostic {
        code: lint_code_str(lint.code).to_string(),
        message: lint.message.clone(),
        line: lint.span.line,
        col: lint.span.col,
    }
}

/// Convert the collected diagnostics into the JS array returned to the caller.
pub fn diagnostics_to_js(diags: &Diagnostics) -> Vec<JsDiagnostic> {
    diags.lints.iter().map(lint_to_js).collect()
}

/// One caller-supplied font face: the raw program bytes plus the selection
/// metadata (`family`, `weight`, `italic`) the cascade matches against. Without
/// it every face was tagged `font0/1` and author CSS `font-family`/bold could
/// not select a specific face.
#[napi(object)]
pub struct JsFont {
    /// The font program bytes (`.ttf`/`.otf`).
    pub data: Buffer,
    /// The CSS `font-family` name this face answers to.
    pub family: String,
    /// The CSS `font-weight` (100..=900); defaults to 400 (normal) when omitted.
    pub weight: Option<u16>,
    /// Whether this is the italic/oblique face; defaults to `false`.
    pub italic: Option<bool>,
}

/// One caller-supplied raster image: its template name (the `<img src>` /
/// `background-image: url(name)` key) and its encoded PNG/JPEG bytes.
#[napi(object)]
pub struct JsImage {
    /// The name the template refers to this image by.
    pub name: String,
    /// The encoded image bytes (PNG or JPEG).
    pub data: Buffer,
}

/// A name-keyed [`ImageResolver`] built from caller-supplied rasters. The core
/// layout/emit path resolves every `<img src="X">` / `background-image:url(X)`
/// by the name `X`; this maps that name back to the bytes the caller passed.
pub struct MapResolver(HashMap<String, Vec<u8>>);

impl MapResolver {
    /// Build the resolver from the JS image list (`{ name, data }` each).
    pub fn new(images: Vec<JsImage>) -> MapResolver {
        let map = images
            .into_iter()
            .map(|img| (img.name, img.data.to_vec()))
            .collect();
        MapResolver(map)
    }

    /// Whether any images were supplied. When empty the caller keeps the
    /// zero-image `&NoImages` path so that render stays byte-identical.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl ImageResolver for MapResolver {
    fn resolve(&self, name: &str) -> Option<&[u8]> {
        self.0.get(name).map(Vec::as_slice)
    }
}

/// Build a [`FontRegistry`] from caller-supplied faces. Each face is parsed via
/// [`FontFace::from_web_font_bytes`] (Chrome 156 web-font kerning, turbo-dom#240)
/// and tagged with its `family`/`weight`/`italic`, so
/// author CSS `font-family`/bold selects the right face via the cascade.
///
/// Unparseable blobs are skipped (the registry simply has fewer faces); the
/// caller sees a `NotdefGlyph` lint downstream if nothing maps a needed glyph.
pub fn build_registry(fonts: Vec<JsFont>) -> FontRegistry {
    let mut reg = FontRegistry::new();
    for font in fonts {
        let weight = font.weight.unwrap_or(400);
        let italic = font.italic.unwrap_or(false);
        if let Some(face) =
            FontFace::from_web_font_bytes(font.data.to_vec(), font.family, weight, italic)
        {
            reg.add(face);
        }
    }
    reg
}
