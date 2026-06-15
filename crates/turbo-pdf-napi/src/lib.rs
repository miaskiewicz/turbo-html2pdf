//! turbo-pdf N-API binding (Phase 10).
//!
//! Exposes the compile -> render -> emit pipeline of `turbo-pdf-core` to Node/JS.
//! A template is compiled once into a [`Program`] (a `Send + Sync` native handle)
//! and rendered against data as many times as needed; a one-shot [`render`]
//! convenience does both in a single call.
//!
//! ## Boundary contract
//! * Input data is an ordinary JS value (object/array/scalar), received as
//!   `serde_json::Value`.
//! * The rendered PDF crosses back as a `Buffer` built directly from the emitter's
//!   `Vec<u8>` (N-API takes ownership of the allocation — no extra copy).
//! * Fatal errors are thrown as a typed `TurboPdfError` (see `errors`); non-fatal
//!   lints are *returned* in the result's `diagnostics` array, never thrown.
//!
//! The product surface is this thin marshaling layer; all rendering logic lives
//! in the core crate. This crate is excluded from the coverage gate (a cdylib
//! addon cannot be line-instrumented), so it is kept deliberately minimal and
//! mechanical, with every branch pushed down into the covered core.

#![deny(clippy::all)]

mod convert;
mod errors;

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;
use serde_json::Value;

use std::sync::Arc;

use turbo_pdf_core::style::TokenSet;
use turbo_pdf_core::{
    build_cascade, compile as core_compile, emit_pdf, render_pages, style::parse_stylesheet,
    CompileOptions, Diagnostics, EmitOptions, FontRegistry, RenderInputs,
};

use convert::{build_registry, diagnostics_to_js, JsDiagnostic};

/// Options for a single render pass. All fields are optional; omit what you do
/// not need. `data` defaults to `null`, `css` to empty, `fonts`/`images` to empty.
#[napi(object)]
#[derive(Default)]
pub struct RenderOptions {
    /// The data object interpolated into the template (`{{ data.* }}`).
    pub data: Option<Value>,
    /// Author CSS. Also feeds `@page` geometry (size/margins) via the parser.
    pub css: Option<String>,
    /// Font programs (raw OpenType/TrueType bytes), one `Buffer` per face.
    pub fonts: Option<Vec<Buffer>>,
    /// Raster images. Accepted but not yet embedded (Phase 9b) — see note below.
    pub images: Option<Vec<Buffer>>,
    /// PDF document metadata written to the info dictionary.
    pub meta: Option<DocMeta>,
    /// Pins the `now()` clock (Unix seconds) for deterministic output.
    pub now: Option<i64>,
}

/// PDF document-info metadata. Every field is optional and omitted when unset.
#[napi(object)]
pub struct DocMeta {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    /// Creation date as Unix seconds. Omit for the reproducible sentinel date.
    pub creation_date: Option<i64>,
}

/// The result of a render: the PDF bytes plus the returned (non-fatal) lints and
/// the page count.
#[napi(object)]
pub struct RenderResult {
    /// The rendered PDF 1.7 document.
    pub pdf: Buffer,
    /// Non-fatal diagnostics collected during render (lints), never thrown.
    pub diagnostics: Vec<JsDiagnostic>,
    /// Number of pages in the document.
    pub page_count: u32,
}

/// A reusable, pre-parsed set of fonts. Build it ONCE (e.g. warm it at server
/// startup) with [`Fonts::load`] and pass the handle to every `render` call:
/// the registry is shared (cheap `Arc` clone), so font programs are parsed once
/// instead of on every request. Omit it to fall back to per-call `opts.fonts`.
#[napi]
pub struct Fonts {
    inner: Arc<FontRegistry>,
}

#[napi]
impl Fonts {
    /// Parse `fonts` (raw OpenType/TrueType byte buffers) once into a reusable
    /// handle. Do this at startup, then reuse it across renders.
    #[napi(factory)]
    pub fn load(fonts: Vec<Buffer>) -> Fonts {
        let blobs: Vec<Vec<u8>> = fonts.iter().map(|b| b.to_vec()).collect();
        Fonts {
            inner: Arc::new(build_registry(&blobs)),
        }
    }
}

/// A compiled, reusable template program. Compiling is the expensive parse step;
/// render it against many data sets. The handle is thread-safe.
#[napi]
pub struct Program {
    inner: turbo_pdf_core::Program,
}

#[napi]
impl Program {
    /// Render this program against `opts` to a PDF. Throws `TurboPdfError` on a
    /// fatal compile/render fault; lints come back in `result.diagnostics`.
    #[napi]
    pub fn render(
        &self,
        opts: Option<RenderOptions>,
        fonts: Option<&Fonts>,
    ) -> napi::Result<RenderResult> {
        run_pipeline(&self.inner, opts.unwrap_or_default(), fonts)
    }

    /// Whether the source declared a `<t:running-header>`.
    #[napi]
    pub fn has_header(&self) -> bool {
        self.inner.has_header()
    }

    /// Whether the source declared a `<t:running-footer>`.
    #[napi]
    pub fn has_footer(&self) -> bool {
        self.inner.has_footer()
    }

    // TODO(phase10b): `to_bytes()` / `from_bytes()`. The core `Program` holds a
    // live MiniJinja `Environment<'static>` and is not serializable today, so a
    // round-trippable handle is deferred rather than shipped as a throwing stub.
    // Compile is cheap enough that callers re-compile from source in the interim.
}

/// Compile `template_html` into a reusable [`Program`]. `_opts` is reserved for
/// future compile knobs (partials, missing-policy) and currently ignored; the
/// default [`CompileOptions`] is used.
#[napi]
pub fn compile(template_html: String, _opts: Option<Value>) -> napi::Result<Program> {
    let (program, _diags) =
        core_compile(&template_html, &CompileOptions::default()).map_err(errors::from_compile)?;
    Ok(Program { inner: program })
}

/// One-shot convenience: compile `template_html` and render it in a single call.
#[napi(js_name = "render")]
pub fn render_oneshot(
    template_html: String,
    opts: Option<RenderOptions>,
    fonts: Option<&Fonts>,
) -> napi::Result<RenderResult> {
    let (program, _diags) =
        core_compile(&template_html, &CompileOptions::default()).map_err(errors::from_compile)?;
    run_pipeline(&program, opts.unwrap_or_default(), fonts)
}

/// The shared render pipeline: cascade + geometry + fonts -> `render_pages` ->
/// `emit_pdf`. Diagnostics flow into the result; only fatal faults throw.
fn run_pipeline(
    program: &turbo_pdf_core::Program,
    opts: RenderOptions,
    fonts: Option<&Fonts>,
) -> napi::Result<RenderResult> {
    let css = opts.css.unwrap_or_default();
    let data = opts.data.unwrap_or(Value::Null);
    let cascade = build_cascade(&css, "", TokenSet::default());
    let at_rules = parse_stylesheet(&css).at_rules;
    // Reuse the prebuilt handle when given; otherwise build a one-off registry
    // from this call's `opts.fonts` (back-compat).
    let owned_registry;
    let registry: &FontRegistry = match fonts {
        Some(handle) => &handle.inner,
        None => {
            owned_registry = build_registry(&font_blobs(&opts.fonts));
            &owned_registry
        }
    };

    let inputs = RenderInputs {
        program,
        data: &data,
        cascade: &cascade,
        at_rules: &at_rules,
        fonts: registry,
        // Image embedding (§7.4) is not yet surfaced through the Node binding; a
        // future option will supply a resolver here. No images for now.
        images: &turbo_pdf_core::NoImages,
        now: opts.now,
    };

    let mut diags = Diagnostics::default();
    let pages = render_pages(&inputs, &mut diags).map_err(errors::from_render)?;
    let pdf = emit_pdf(&pages, &emit_options(opts.meta));

    Ok(RenderResult {
        pdf: pdf.into(),
        diagnostics: diagnostics_to_js(&diags),
        page_count: pages.len() as u32,
    })
}

/// Collect the raw bytes of each supplied font `Buffer`. Images are accepted but
/// dropped here: raster embedding is Phase 9b (the param is wired through so the
/// JS API is stable, but has no effect yet).
fn font_blobs(fonts: &Option<Vec<Buffer>>) -> Vec<Vec<u8>> {
    fonts
        .as_ref()
        .map(|fs| fs.iter().map(|b| b.to_vec()).collect())
        .unwrap_or_default()
}

/// Translate optional [`DocMeta`] into core [`EmitOptions`].
fn emit_options(meta: Option<DocMeta>) -> EmitOptions {
    match meta {
        None => EmitOptions::default(),
        Some(m) => EmitOptions {
            title: m.title,
            author: m.author,
            subject: m.subject,
            keywords: m.keywords,
            creation_date: m.creation_date,
            ..EmitOptions::default()
        },
    }
}
