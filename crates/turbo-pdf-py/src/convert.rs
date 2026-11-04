//! Pure conversions between core types and the Python wire shapes: diagnostics
//! (returned as a list of dicts) and the font registry assembled from
//! caller-supplied byte blobs. Mirrors the N-API `convert.rs` 1:1.

use pyo3::prelude::*;
use pyo3::types::PyDict;

use turbo_html2pdf_core::{Diagnostics, FontFace, FontRegistry, Lint, LintCode};

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

/// Convert one core lint into its Python dict `{code, message, line, col}`.
fn lint_to_py<'py>(py: Python<'py>, lint: &Lint) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("code", lint_code_str(lint.code))?;
    d.set_item("message", &lint.message)?;
    d.set_item("line", lint.span.line)?;
    d.set_item("col", lint.span.col)?;
    Ok(d)
}

/// Convert the collected diagnostics into the Python list returned to callers.
pub fn diagnostics_to_py<'py>(
    py: Python<'py>,
    diags: &Diagnostics,
) -> PyResult<Vec<Bound<'py, PyDict>>> {
    diags.lints.iter().map(|l| lint_to_py(py, l)).collect()
}

/// Build a [`FontRegistry`] from caller-supplied font byte blobs. Each blob is
/// parsed via [`FontFace::from_web_font_bytes`]; the family is a synthetic per-index tag
/// so author CSS `font-family` still selects via the registry's fallback chain.
/// Unparseable blobs are skipped (the registry simply has fewer faces).
pub fn build_registry(fonts: &[Vec<u8>]) -> FontRegistry {
    let mut reg = FontRegistry::new();
    for (i, bytes) in fonts.iter().enumerate() {
        register_one(&mut reg, bytes, i);
    }
    reg
}

/// Parse and register a single font blob, ignoring blobs that fail to parse.
fn register_one(reg: &mut FontRegistry, bytes: &[u8], index: usize) {
    let family = font_family(index);
    if let Some(face) = FontFace::from_web_font_bytes(bytes.to_vec(), family, 400, false) {
        reg.add(face);
    }
}

/// A stable family tag for a registered font, by registration index; the
/// registry falls back to the first face when no CSS family matches.
fn font_family(index: usize) -> String {
    format!("font{index}")
}
