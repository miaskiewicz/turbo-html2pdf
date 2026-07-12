//! The `Program` handle and the `compile`/`render` entry points: this is where
//! the binding wires the core pipeline together exactly as the native
//! `tests/render.rs` + `tests/emit.rs` do — `compile` →
//! `render::render_pages(&RenderInputs{ .. })` → `emit::emit_pdf(&pages, &opts)`.

use serde::Deserialize;
use turbo_html2pdf_core::style::{parse_stylesheet, AtRule, TokenSet};
use turbo_html2pdf_core::{
    append_pdfs, build_cascade, compile as core_compile, emit_pdf_with_images, render_pages,
    Cascade, Diagnostics, EmitOptions, FontRegistry, ImageResolver, NoImages, RenderInputs,
};
use wasm_bindgen::prelude::*;

use crate::convert::{
    build_registry, diagnostics_to_js, JsCompileOptions, JsConformance, JsDiagnostic, JsFont,
    JsImage, JsMeta, JsWatermark, MapResolver,
};
use crate::DEFAULT_NOW;

/// A compiled, reusable template program. Compile once, then call
/// [`Program::render`] repeatedly with different data — the heavy parse work is
/// done at compile time (§8.1).
#[wasm_bindgen]
pub struct Program {
    inner: turbo_html2pdf_core::Program,
}

/// JS-side per-render input:
/// `{ data, css?, fonts?, images?, meta?, watermark?, now? }`.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct JsRenderArgs {
    data: serde_json::Value,
    css: String,
    fonts: Vec<JsFont>,
    /// Named raster images, each `{ name, data: Uint8Array }`. A
    /// `<img src="name">` / `background-image: url(name)` embeds the matching
    /// bytes as an image XObject.
    images: Vec<JsImage>,
    meta: JsMeta,
    /// An optional page watermark stamped behind the body on every page.
    watermark: Option<JsWatermark>,
    /// Render-clock override (Unix seconds). Omit for the deterministic default.
    now: Option<i64>,
    /// Per-render conformance / encryption toggles: `{ pdfA?, pdfUa?,
    /// toUnicode?, lang?, cmyk?, encryption? }`. Flattened so the toggles live
    /// at the top level of the render-args object alongside `meta`/`watermark`.
    #[serde(flatten)]
    conformance: JsConformance,
    /// Foreign PDF documents (each a `Uint8Array`) glued page-by-page AFTER the
    /// rendered pages, in order.
    #[serde(default)]
    append_pdfs: Vec<serde_bytes::ByteBuf>,
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
        let mut args = parse_render_args(args)?;
        let registry = build_registry(std::mem::take(&mut args.fonts));
        serialize_outcome(self.run(args, &registry)?)
    }

    /// Render reusing a prebuilt [`Fonts`] handle (parse fonts once at startup,
    /// reuse across renders). `args.fonts` is ignored when a handle is given.
    #[wasm_bindgen(js_name = renderWithFonts)]
    pub fn render_with_fonts(&self, args: JsValue, fonts: &Fonts) -> Result<JsValue, JsValue> {
        let args = parse_render_args(args)?;
        serialize_outcome(self.run(args, &fonts.inner)?)
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
    fn run(
        &self,
        mut args: JsRenderArgs,
        registry: &FontRegistry,
    ) -> Result<RenderOutcome, JsValue> {
        let resolver = MapResolver::new(std::mem::take(&mut args.images));
        let cascade: Cascade = build_cascade(&args.css, "", TokenSet::default());
        let at_rules: Vec<AtRule> = parse_stylesheet(&args.css).at_rules;
        let now = Some(args.now.unwrap_or(DEFAULT_NOW));
        let meta = std::mem::take(&mut args.meta);
        let watermark = args.watermark.take().and_then(|w| {
            let face = registry.select(&[], 400, false).cloned();
            w.into_core(face)
        });
        let conformance = std::mem::take(&mut args.conformance);
        let append = std::mem::take(&mut args.append_pdfs);
        let mut emit = EmitOptions {
            watermark,
            ..meta.into_core()
        };
        conformance.apply(&mut emit);

        let mut diags = Diagnostics::default();
        let pages = {
            let inputs = RenderInputs {
                program: &self.inner,
                data: &args.data,
                cascade: &cascade,
                at_rules: &at_rules,
                fonts: registry,
                images: image_source(&resolver),
                now,
            };
            render_pages(&inputs, &mut diags)
                .map_err(|e| crate::convert::JsError::from(e).into_jsvalue())?
        };
        let page_count = pages.len();
        let pdf = emit_pdf_with_images(&pages, &emit, image_source(&resolver));
        let pdf = merge_appended(pdf, &append)?;
        Ok(RenderOutcome {
            pdf,
            diagnostics: diagnostics_to_js(&diags),
            page_count,
        })
    }
}

/// A reusable, pre-parsed set of fonts. Build it ONCE (e.g. warm it at startup)
/// with [`Fonts::load`] and pass it to [`Program::render_with_fonts`] so font
/// programs are parsed once instead of on every render.
#[wasm_bindgen]
pub struct Fonts {
    inner: FontRegistry,
}

#[wasm_bindgen]
impl Fonts {
    /// Parse `fonts` (an array of `{ data: Uint8Array, family?, weight?, italic? }`)
    /// once into a reusable handle.
    #[wasm_bindgen(js_name = load)]
    pub fn load(fonts: JsValue) -> Result<Fonts, JsValue> {
        let faces: Vec<JsFont> =
            serde_wasm_bindgen::from_value(fonts).map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(Fonts {
            inner: build_registry(faces),
        })
    }
}

/// Serialize a render outcome back to a JS `{ pdf, diagnostics, pageCount }`.
fn serialize_outcome(outcome: RenderOutcome) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(&outcome).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Deserialize the render argument, treating `undefined`/`null` as empty inputs.
fn parse_render_args(args: JsValue) -> Result<JsRenderArgs, JsValue> {
    if args.is_undefined() || args.is_null() {
        return Ok(JsRenderArgs::default());
    }
    serde_wasm_bindgen::from_value(args).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Glue each foreign PDF blob after the rendered `pdf`, page by page. A parse
/// failure becomes a structured JS error. With no extras the input is unchanged.
fn merge_appended(pdf: Vec<u8>, extras: &[serde_bytes::ByteBuf]) -> Result<Vec<u8>, JsValue> {
    if extras.is_empty() {
        return Ok(pdf);
    }
    let refs: Vec<&[u8]> = extras.iter().map(|b| b.as_ref()).collect();
    append_pdfs(&pdf, &refs).map_err(append_error)
}

/// Translate an [`turbo_html2pdf_core::AppendError`] into a structured JS error value.
fn append_error(e: turbo_html2pdf_core::AppendError) -> JsValue {
    crate::convert::JsError::append(e.to_string()).into_jsvalue()
}

/// Glue one or more foreign PDF documents after `base`, page by page, returning
/// the merged PDF as a `Uint8Array`. Mirrors `render`'s `appendPdfs` option but
/// runs on already-emitted bytes. Rejects (structured `{ code, message, span }`)
/// if any input fails to parse.
#[wasm_bindgen(js_name = appendPdf)]
pub fn append_pdf(base: &[u8], extras: JsValue) -> Result<JsValue, JsValue> {
    let extras: Vec<serde_bytes::ByteBuf> =
        serde_wasm_bindgen::from_value(extras).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let refs: Vec<&[u8]> = extras.iter().map(|b| b.as_ref()).collect();
    let merged = append_pdfs(base, &refs).map_err(append_error)?;
    serde_wasm_bindgen::to_value(&serde_bytes::Bytes::new(&merged))
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// The image source for layout/emit: the caller's resolver when it carries
/// images, else the zero-image [`NoImages`] so the no-image path is identical.
fn image_source(resolver: &MapResolver) -> &dyn ImageResolver {
    if resolver.is_empty() {
        &NoImages
    } else {
        resolver
    }
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
