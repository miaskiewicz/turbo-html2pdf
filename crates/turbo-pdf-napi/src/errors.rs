//! Error marshaling across the N-API boundary.
//!
//! Core fatal errors ([`CompileError`]/[`RenderError`]) carry a machine-readable
//! [`ErrorCode`] and a source [`Span`] (line/col/byte-offset). N-API's own error
//! type is only a `(Status, String)` pair, so we encode the structured payload as
//! a JSON object in the error `reason` behind a stable sentinel prefix. The thin
//! JS wrapper (`index.js`) detects the prefix and rethrows a typed
//! `TurboPdfError` whose `.code` and `.span` mirror this payload.

use turbo_pdf_core::{CompileError, ErrorCode, RenderError, Span};

/// Sentinel that marks a `reason` string as a structured turbo-pdf error. The JS
/// wrapper splits on this to recover the JSON payload.
pub const SENTINEL: &str = "TURBO_PDF_ERR:";

/// The stable string form of an [`ErrorCode`] (mirrors the variant name).
fn code_str(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::TemplateSyntax => "TemplateSyntax",
        ErrorCode::UnknownFilter => "UnknownFilter",
        ErrorCode::UndefinedValue => "UndefinedValue",
        ErrorCode::IncludeDepthExceeded => "IncludeDepthExceeded",
        ErrorCode::UnknownElement => "UnknownElement",
        ErrorCode::Render => "Render",
    }
}

/// Build the JSON payload `{code, message, span:{line,col,byteOffset}}`.
fn payload(code: ErrorCode, message: &str, span: Span) -> serde_json::Value {
    serde_json::json!({
        "code": code_str(code),
        "message": message,
        "span": { "line": span.line, "col": span.col, "byteOffset": span.byte_offset },
    })
}

/// Encode a structured payload into a sentinel-prefixed N-API error.
fn encode(code: ErrorCode, message: &str, span: Span) -> napi::Error {
    let body = payload(code, message, span).to_string();
    napi::Error::from_reason(format!("{SENTINEL}{body}"))
}

/// Map a fatal compile error to a typed N-API error.
pub fn from_compile(e: CompileError) -> napi::Error {
    encode(e.code, &e.message, e.span)
}

/// Map a fatal render error to a typed N-API error.
pub fn from_render(e: RenderError) -> napi::Error {
    encode(e.code, &e.message, e.span)
}
