//! Pure conversions between core types and the N-API wire shapes: diagnostics,
//! lint codes, and the font registry assembled from caller-supplied byte blobs.

use napi_derive::napi;

use turbo_pdf_core::{Diagnostics, FontFace, FontRegistry, Lint, LintCode};

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

/// Build a [`FontRegistry`] from caller-supplied font byte blobs. Each blob is
/// parsed via [`FontFace::from_bytes`]; the family/weight/style are read from the
/// font program itself by tagging with a synthetic family derived from index, so
/// author CSS `font-family` still selects via the registry's fallback chain.
///
/// Unparseable blobs are skipped (the registry simply has fewer faces); the
/// caller sees a `NotdefGlyph` lint downstream if nothing maps a needed glyph.
pub fn build_registry(fonts: &[Vec<u8>]) -> FontRegistry {
    let mut reg = FontRegistry::new();
    for (i, bytes) in fonts.iter().enumerate() {
        register_one(&mut reg, bytes, i);
    }
    reg
}

/// Parse and register a single font blob, ignoring blobs that fail to parse.
fn register_one(reg: &mut FontRegistry, bytes: &[u8], index: usize) {
    let family = font_family(bytes, index);
    if let Some(face) = FontFace::from_bytes(bytes.to_vec(), family, 400, false) {
        reg.add(face);
    }
}

/// A stable family tag for a registered font. The face's intrinsic family is not
/// exposed by the loader, so we tag by registration index; the registry falls
/// back to the first registered face when no CSS family matches (§ text layout).
fn font_family(_bytes: &[u8], index: usize) -> String {
    format!("font{index}")
}
