//! Boundary conversions between JS values and the core's Rust types: compile
//! options, the per-render inputs (CSS, fonts, metadata, clock), and the
//! diagnostics/error shapes returned to JS.
//!
//! Everything here is plain `serde` data shuttled across `serde-wasm-bindgen`,
//! plus `Uint8Array` font bytes. Each helper is kept small so the binding stays
//! under the project's cyclomatic-complexity gate.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use turbo_pdf_core::{
    CompileError, CompileOptions, Diagnostics, EmitOptions, FontFace, FontRegistry, Lint,
    MissingPolicy, RenderError, Span,
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
        }
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
    /// Serialize into a `JsValue` suitable for `Err(..)`, so the JS caller sees a
    /// structured `{ code, message, span }` object on the rejection path.
    pub fn into_jsvalue(self) -> JsValue {
        serde_wasm_bindgen::to_value(&self)
            .unwrap_or_else(|_| JsValue::from_str("turbo-pdf: error serialization failed"))
    }
}
