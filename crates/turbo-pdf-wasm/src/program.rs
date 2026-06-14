//! The `Program` handle and the `compile`/`render` entry points: this is where
//! the binding wires the core pipeline together exactly as the native
//! `tests/render.rs` + `tests/emit.rs` do — `compile` →
//! `render::render_pages(&RenderInputs{ .. })` → `emit::emit_pdf(&pages, &opts)`.

use serde::Deserialize;
use turbo_pdf_core::style::{parse_stylesheet, AtRule, TokenSet};
use turbo_pdf_core::{
    build_cascade, compile as core_compile, emit_pdf, render_pages, Cascade, Diagnostics,
    FontRegistry, RenderInputs,
};
use wasm_bindgen::prelude::*;

use crate::convert::{
    build_registry, diagnostics_to_js, JsCompileOptions, JsDiagnostic, JsFont, JsMeta,
};
use crate::DEFAULT_NOW;

/// A compiled, reusable template program. Compile once, then call
/// [`Program::render`] repeatedly with different data — the heavy parse work is
/// done at compile time (§8.1).
#[wasm_bindgen]
pub struct Program {
    inner: turbo_pdf_core::Program,
}

/// JS-side per-render input: `{ data, css?, fonts?, images?, meta?, now? }`.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct JsRenderArgs {
    data: serde_json::Value,
    css: String,
    fonts: Vec<JsFont>,
    /// Raster images (`Uint8Array` each). Accepted but not yet embedded — raster
    /// support is Phase 9b. See [`note_images`].
    images: Vec<serde_bytes::ByteBuf>,
    meta: JsMeta,
    /// Render-clock override (Unix seconds). Omit for the deterministic default.
    now: Option<i64>,
}

/// Compile `template_html` into a reusable [`Program`]. `opts` is the optional
/// `{ partials?, missingPolicy?, includeMaxDepth? }` object. A fatal syntax
/// error rejects with a structured `{ code, message, span }`.
#[wasm_bindgen]
pub fn compile(template_html: &str, opts: JsValue) -> Result<Program, JsValue> {
    let opts = parse_compile_opts(opts)?;
    let (inner, _diags) = core_compile(template_html, &opts.into_core())
        .map_err(|e| crate::convert::JsError::from(e).into_jsvalue())?;
    Ok(Program { inner })
}

/// Deserialize the compile-options argument, treating `undefined`/`null` as the
/// default option set.
fn parse_compile_opts(opts: JsValue) -> Result<JsCompileOptions, JsValue> {
    if opts.is_undefined() || opts.is_null() {
        return Ok(JsCompileOptions::default());
    }
    serde_wasm_bindgen::from_value(opts).map_err(|e| JsValue::from_str(&e.to_string()))
}

#[wasm_bindgen]
impl Program {
    /// Render this program against `args` (`{ data, css?, fonts?, images?, meta?,
    /// now? }`) and return `{ pdf: Uint8Array, diagnostics, pageCount }`.
    ///
    /// Diagnostics (lints) are returned in the result, not thrown; only a fatal
    /// render error rejects with `{ code, message, span }`.
    pub fn render(&self, args: JsValue) -> Result<JsValue, JsValue> {
        let args = parse_render_args(args)?;
        let outcome = self.run(args)?;
        serde_wasm_bindgen::to_value(&outcome).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Whether the source carried a `<t:running-header>`.
    #[wasm_bindgen(js_name = hasHeader)]
    pub fn has_header(&self) -> bool {
        self.inner.has_header()
    }

    /// Whether the source carried a `<t:running-footer>`.
    #[wasm_bindgen(js_name = hasFooter)]
    pub fn has_footer(&self) -> bool {
        self.inner.has_footer()
    }

    /// Wire the core pipeline for one render: build the cascade + registry +
    /// at-rules, drive `render_pages`, then `emit_pdf`. Lints are collected and
    /// returned; a fatal render error becomes the `Err`.
    fn run(&self, args: JsRenderArgs) -> Result<RenderOutcome, JsValue> {
        note_images(&args.images);
        let cascade: Cascade = build_cascade(&args.css, "", TokenSet::default());
        let at_rules: Vec<AtRule> = parse_stylesheet(&args.css).at_rules;
        let registry: FontRegistry = build_registry(args.fonts);
        let now = Some(args.now.unwrap_or(DEFAULT_NOW));

        let mut diags = Diagnostics::default();
        let inputs = RenderInputs {
            program: &self.inner,
            data: &args.data,
            cascade: &cascade,
            at_rules: &at_rules,
            fonts: &registry,
            now,
        };
        let pages = render_pages(&inputs, &mut diags)
            .map_err(|e| crate::convert::JsError::from(e).into_jsvalue())?;
        let pdf = emit_pdf(&pages, &args.meta.into_core());
        Ok(RenderOutcome {
            pdf,
            diagnostics: diagnostics_to_js(&diags),
            page_count: pages.len(),
        })
    }
}

/// Deserialize the render argument, treating `undefined`/`null` as empty inputs.
fn parse_render_args(args: JsValue) -> Result<JsRenderArgs, JsValue> {
    if args.is_undefined() || args.is_null() {
        return Ok(JsRenderArgs::default());
    }
    serde_wasm_bindgen::from_value(args).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Raster image embedding is Phase 9b: images are accepted across the boundary so
/// the API is stable, but not yet embedded. This is the documented no-op seam.
fn note_images(_images: &[serde_bytes::ByteBuf]) {
    // No-op: see Phase 9b. Images are validated only as `Uint8Array` shape by
    // the deserializer; nothing consumes them yet.
}

/// The successful render result serialized back to JS: `{ pdf, diagnostics,
/// pageCount }`. `pdf` serializes to a `Uint8Array` via `serde-wasm-bindgen`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RenderOutcome {
    #[serde(with = "serde_bytes")]
    pdf: Vec<u8>,
    diagnostics: Vec<JsDiagnostic>,
    page_count: usize,
}
