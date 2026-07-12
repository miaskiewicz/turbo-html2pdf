//! Boundary conversions between JS values and the core's Rust types: compile
//! options, the per-render inputs (CSS, fonts, metadata, clock), and the
//! diagnostics/error shapes returned to JS.
//!
//! Everything here is plain `serde` data shuttled across `serde-wasm-bindgen`,
//! plus `Uint8Array` font bytes. Each helper is kept small so the binding stays
//! under the project's cyclomatic-complexity gate.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use turbo_html2pdf_core::{
    CompileError, CompileOptions, Diagnostics, EmitOptions, Encryption, FontFace, FontRegistry,
    ImageResolver, ImageWatermark, Lint, MissingPolicy, Permissions, RenderError, Rgba, Span,
    TextWatermark, Watermark,
};
use wasm_bindgen::prelude::*;

/// JS-side compile options: `{ partials?, missingPolicy?, includeMaxDepth? }`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JsCompileOptions {
    pub partials: HashMap<String, String>,
    pub missing_policy: Option<String>,
    pub include_max_depth: u32,
}

impl JsCompileOptions {
    /// Lower into the core's [`CompileOptions`]. An unknown `missingPolicy`
    /// string falls back to the strict default rather than erroring.
    pub fn into_core(self) -> CompileOptions {
        CompileOptions {
            partials: self.partials,
            missing_policy: parse_policy(self.missing_policy.as_deref()),
            include_max_depth: self.include_max_depth,
        }
    }
}

fn parse_policy(name: Option<&str>) -> MissingPolicy {
    match name {
        Some("empty") | Some("lenient") => MissingPolicy::Empty,
        _ => MissingPolicy::Strict,
    }
}

/// One caller-supplied font face: raw bytes plus selection metadata.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsFont {
    /// The font program bytes (`.ttf`/`.otf`), arriving as a `Uint8Array`.
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
    pub family: String,
    #[serde(default = "default_weight")]
    pub weight: u16,
    #[serde(default)]
    pub italic: bool,
}

fn default_weight() -> u16 {
    400
}

/// One caller-supplied named raster image: the template name plus its encoded
/// PNG/JPEG bytes (arriving as a `Uint8Array`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsImage {
    /// The name the template refers to this image by (`<img src>` / `url(...)`).
    pub name: String,
    /// The encoded image bytes (PNG or JPEG).
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

/// A name-keyed [`ImageResolver`] built from the JS image list. The core layout
/// and emit paths resolve every `<img src="X">` / `background-image:url(X)` by
/// the name `X`; this maps that name back to the bytes the caller supplied.
pub struct MapResolver(HashMap<String, Vec<u8>>);

impl MapResolver {
    /// Build the resolver from the JS image list (`{ name, data }` each).
    pub fn new(images: Vec<JsImage>) -> MapResolver {
        MapResolver(images.into_iter().map(|i| (i.name, i.data)).collect())
    }

    /// Whether any images were supplied; an empty resolver keeps the caller on
    /// the zero-image `&NoImages` path so a no-image render stays identical.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl ImageResolver for MapResolver {
    fn resolve(&self, name: &str) -> Option<&[u8]> {
        self.0.get(name).map(Vec::as_slice)
    }
}

/// The JS shape of a page watermark: a text mark
/// (`{ text?, color?, opacity?, angle? }`) or an image mark
/// (`{ image, opacity?, tiled? }`). `image` (when set) selects the raster mark;
/// otherwise a text mark seeded from the `DRAFT` preset is built.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JsWatermark {
    pub text: Option<String>,
    pub color: Option<String>,
    pub image: Option<String>,
    pub opacity: Option<f32>,
    pub angle: Option<f32>,
    pub tiled: Option<bool>,
}

impl JsWatermark {
    /// Lower into a core [`Watermark`]. An image mark resolves by name through
    /// the images you pass; a text mark is shaped with `face` (the registry's
    /// first face). Returns `None` for a text mark when no face is available.
    pub fn into_core(self, face: Option<FontFace>) -> Option<Watermark> {
        if let Some(name) = self.image {
            return Some(Watermark::Image(ImageWatermark {
                name,
                opacity: self.opacity.unwrap_or(1.0),
                tiled: self.tiled.unwrap_or(false),
            }));
        }
        let mut mark = TextWatermark::draft(face?);
        apply_text_overrides(&mut mark, self);
        Some(Watermark::Text(Box::new(mark)))
    }
}

/// Apply the optional JS overrides onto a preset text watermark, leaving any
/// omitted field at its `DRAFT`-preset default.
fn apply_text_overrides(mark: &mut TextWatermark, w: JsWatermark) {
    if let Some(text) = w.text {
        mark.text = text;
    }
    if let Some(color) = w.color.as_deref().and_then(parse_hex_color) {
        mark.color = color;
    }
    if let Some(opacity) = w.opacity {
        mark.opacity = opacity;
    }
    if let Some(angle) = w.angle {
        mark.angle_deg = angle;
    }
}

/// Parse a `#rrggbb` (or `rrggbb`) hex color into an opaque [`Rgba`], or `None`
/// for any malformed string (leaving the preset color in place).
fn parse_hex_color(s: &str) -> Option<Rgba> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Rgba::new(r, g, b, 255))
}

/// Document metadata `{ title?, author?, subject?, keywords?, creationDate? }`.
/// `creationDate` is a Unix timestamp in seconds; absent leaves the emitter's
/// deterministic sentinel in place (AC-8.6).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JsMeta {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creation_date: Option<i64>,
}

impl JsMeta {
    /// Lower into the emitter's [`EmitOptions`].
    pub fn into_core(self) -> EmitOptions {
        EmitOptions {
            title: self.title,
            author: self.author,
            subject: self.subject,
            keywords: self.keywords,
            creation_date: self.creation_date,
            ..EmitOptions::default()
        }
    }
}

/// AES-256 password-encryption settings: `{ userPassword, ownerPassword?,
/// print?, modify?, copy?, annotate?, fillForms?, accessibility?, assemble?,
/// highQualityPrint? }`. Permission flags default to all-granted.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JsEncryption {
    pub user_password: String,
    pub owner_password: Option<String>,
    pub print: Option<bool>,
    pub modify: Option<bool>,
    pub copy: Option<bool>,
    pub annotate: Option<bool>,
    pub fill_forms: Option<bool>,
    pub accessibility: Option<bool>,
    pub assemble: Option<bool>,
    pub high_quality_print: Option<bool>,
}

impl JsEncryption {
    /// Lower into the core [`Encryption`], applying permission overrides onto the
    /// all-granted default (an omitted flag keeps the granted default).
    pub fn into_core(self) -> Encryption {
        let d = Permissions::all();
        let perms = Permissions {
            print: self.print.unwrap_or(d.print),
            modify: self.modify.unwrap_or(d.modify),
            copy: self.copy.unwrap_or(d.copy),
            annotate: self.annotate.unwrap_or(d.annotate),
            fill_forms: self.fill_forms.unwrap_or(d.fill_forms),
            accessibility: self.accessibility.unwrap_or(d.accessibility),
            assemble: self.assemble.unwrap_or(d.assemble),
            high_quality_print: self.high_quality_print.unwrap_or(d.high_quality_print),
        };
        Encryption {
            user_password: self.user_password,
            owner_password: self.owner_password,
            permissions: perms,
        }
    }
}

/// The per-render conformance / encryption toggles, deserialized inline on
/// [`JsRenderArgs`](crate::program). Lowered onto [`EmitOptions`] after the
/// metadata fields.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JsConformance {
    pub pdf_a: bool,
    pub pdf_ua: bool,
    pub to_unicode: bool,
    pub lang: Option<String>,
    pub cmyk: bool,
    pub encryption: Option<JsEncryption>,
}

impl JsConformance {
    /// Apply the conformance toggles onto an [`EmitOptions`] already carrying the
    /// metadata and watermark.
    pub fn apply(self, opts: &mut EmitOptions) {
        opts.cmyk = self.cmyk;
        opts.pdf_a = self.pdf_a;
        opts.pdf_ua = self.pdf_ua;
        opts.to_unicode = self.to_unicode;
        opts.lang = self.lang;
        opts.encryption = self.encryption.map(JsEncryption::into_core);
    }
}

/// Build a [`FontRegistry`] from the JS font list, skipping any face whose bytes
/// fail to parse (its glyphs simply fall through the fallback chain to `.notdef`
/// + a lint, exactly as the core handles a missing glyph).
pub fn build_registry(fonts: Vec<JsFont>) -> FontRegistry {
    let mut registry = FontRegistry::new();
    for font in fonts {
        if let Some(face) = FontFace::from_bytes(font.data, font.family, font.weight, font.italic) {
            registry.add(face);
        }
    }
    registry
}

/// The JS shape of a single span: `{ line, col, byteOffset }`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JsSpan {
    pub line: u32,
    pub col: u32,
    pub byte_offset: u32,
}

impl From<Span> for JsSpan {
    fn from(s: Span) -> Self {
        JsSpan {
            line: s.line,
            col: s.col,
            byte_offset: s.byte_offset,
        }
    }
}

/// The JS shape of one diagnostic/lint: `{ code, message, span }`.
#[derive(Debug, Serialize)]
pub struct JsDiagnostic {
    pub code: String,
    pub message: String,
    pub span: JsSpan,
}

impl From<&Lint> for JsDiagnostic {
    fn from(l: &Lint) -> Self {
        JsDiagnostic {
            code: format!("{:?}", l.code),
            message: l.message.clone(),
            span: l.span.into(),
        }
    }
}

/// Lower collected [`Diagnostics`] into the JS array shape returned in the
/// render result.
pub fn diagnostics_to_js(diags: &Diagnostics) -> Vec<JsDiagnostic> {
    diags.lints.iter().map(JsDiagnostic::from).collect()
}

/// The JS shape of a fatal error, thrown as a JS exception: `{ code, message,
/// span }`. Compile and render errors share it.
#[derive(Debug, Serialize)]
pub struct JsError {
    pub code: String,
    pub message: String,
    pub span: JsSpan,
}

impl From<CompileError> for JsError {
    fn from(e: CompileError) -> Self {
        JsError {
            code: format!("{:?}", e.code),
            message: e.message,
            span: e.span.into(),
        }
    }
}

impl From<RenderError> for JsError {
    fn from(e: RenderError) -> Self {
        JsError {
            code: format!("{:?}", e.code),
            message: e.message,
            span: e.span.into(),
        }
    }
}

impl JsError {
    /// Build an error for a PDF append/merge failure. Append errors carry no
    /// source span, so a zeroed span is used under a stable `Append` code.
    pub fn append(message: String) -> JsError {
        JsError {
            code: "Append".to_string(),
            message,
            span: Span::default().into(),
        }
    }

    /// Serialize into a `JsValue` suitable for `Err(..)`, so the JS caller sees a
    /// structured `{ code, message, span }` object on the rejection path.
    pub fn into_jsvalue(self) -> JsValue {
        serde_wasm_bindgen::to_value(&self)
            .unwrap_or_else(|_| JsValue::from_str("turbo-pdf: error serialization failed"))
    }
}
