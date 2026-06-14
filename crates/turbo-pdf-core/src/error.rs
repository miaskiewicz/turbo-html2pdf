//! Error, span, and diagnostic types shared across the pipeline (§9).
//!
//! Every fatal error carries a [`Span`] pointing back into the template source.
//! Non-fatal problems are collected as [`Lint`]s in [`Diagnostics`].

use std::ops::Range;

use thiserror::Error;

/// A location in the template source. `byte_offset` is the start offset of the
/// offending construct; `line`/`col` are 1-based where known (0 when the
/// underlying engine could not supply them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub line: u32,
    pub col: u32,
    pub byte_offset: u32,
}

impl Span {
    /// Construct a span from a 1-based line and an optional byte range.
    pub fn new(line: u32, range: Option<Range<usize>>) -> Self {
        let byte_offset = range.map_or(0, |r| r.start as u32);
        Span {
            line,
            col: 0,
            byte_offset,
        }
    }
}

/// Stable machine-readable code for a fatal error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// Malformed Jinja syntax (unbalanced tags, bad expression).
    TemplateSyntax,
    /// Reference to a filter/function/test that is not registered.
    UnknownFilter,
    /// An undefined variable or out-of-range access under the strict policy.
    UndefinedValue,
    /// `include`/`import`/macro recursion exceeded the configured depth.
    IncludeDepthExceeded,
    /// A `t:` element whose name is not a recognized directive (§9.1).
    UnknownElement,
    /// Any other render-time evaluation failure.
    Render,
}

/// A fatal error produced while compiling a template.
#[derive(Debug, Error)]
#[error("{code:?} at line {}: {message}", span.line)]
pub struct CompileError {
    pub code: ErrorCode,
    pub message: String,
    pub span: Span,
}

/// A fatal error produced while rendering a compiled program.
#[derive(Debug, Error)]
#[error("{code:?} at line {}: {message}", span.line)]
pub struct RenderError {
    pub code: ErrorCode,
    pub message: String,
    pub span: Span,
}

/// Stable machine-readable code for a non-fatal lint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintCode {
    /// A CSS property turbo-pdf does not implement was encountered.
    UnsupportedCss,
    /// Interpolating a non-scalar value (a map/seq) — almost always a mistake.
    NonScalarInterpolation,
    /// Raw/unescaped output via the `safe` filter.
    RawOutput,
    /// Region content was clipped to its declared extent.
    RegionOverflow,
    /// A glyph absent from all provided fonts rendered as `.notdef`.
    NotdefGlyph,
    /// The footnote/body fixpoint did not converge within the iteration cap.
    FootnoteConvergence,
}

/// A non-fatal diagnostic collected during compile or render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lint {
    pub code: LintCode,
    pub message: String,
    pub span: Span,
}

/// Collected non-fatal diagnostics returned alongside successful output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnostics {
    pub lints: Vec<Lint>,
}

impl Diagnostics {
    /// Append a lint to the collection.
    pub fn push(&mut self, code: LintCode, message: impl Into<String>, span: Span) {
        self.lints.push(Lint {
            code,
            message: message.into(),
            span,
        });
    }

    /// True when no lints have been collected.
    pub fn is_empty(&self) -> bool {
        self.lints.is_empty()
    }
}
