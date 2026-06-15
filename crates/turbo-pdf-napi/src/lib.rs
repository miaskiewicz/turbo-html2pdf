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

use std::collections::HashMap;

use turbo_pdf_core::style::TokenSet;
use turbo_pdf_core::{
    append_pdfs, build_cascade, compile as core_compile, emit_pdf_with_images, render_pages,
    style::parse_stylesheet, CompileOptions, Diagnostics, EmitOptions, Encryption, FontRegistry,
    ImageWatermark, MissingPolicy, NoImages, Permissions, RenderInputs, Rgba, TextWatermark,
    Watermark,
};

use convert::{build_registry, diagnostics_to_js, JsDiagnostic, JsFont, JsImage, MapResolver};

/// Options for a single render pass. All fields are optional; omit what you do
/// not need. `data` defaults to `null`, `css` to empty, `fonts`/`images` to empty.
#[napi(object)]
#[derive(Default)]
pub struct RenderOptions {
    /// The data object interpolated into the template (`{{ data.* }}`).
    pub data: Option<Value>,
    /// Author CSS. Also feeds `@page` geometry (size/margins) via the parser.
    pub css: Option<String>,
    /// Font faces, each `{ data, family, weight?, italic? }`. The `family`/weight
    /// drive CSS `font-family`/bold selection.
    pub fonts: Option<Vec<JsFont>>,
    /// Named raster images, each `{ name, data }`. A `<img src="name">` or
    /// `background-image: url(name)` in the template embeds the matching bytes.
    pub images: Option<Vec<JsImage>>,
    /// PDF document metadata written to the info dictionary.
    pub meta: Option<DocMeta>,
    /// A faded watermark stamped behind the body on every page. Either a text
    /// mark (`{ text?, color?, opacity?, angle? }`) or an image mark
    /// (`{ image, opacity?, tiled? }`, resolved against `images` by name).
    pub watermark: Option<JsWatermark>,
    /// Pins the `now()` clock (Unix seconds) for deterministic output.
    pub now: Option<i64>,
    /// Emit PDF/A-2b archival conformance for this render (sRGB `/OutputIntents`,
    /// an XMP `pdfaid` packet, a trailer `/ID`). Defaults to `false`.
    pub pdf_a: Option<bool>,
    /// Emit tagged / accessible PDF/UA-1 machinery for this render
    /// (`/StructTreeRoot`, `/MarkInfo`, per-face `/ToUnicode`, `/Lang`).
    /// Defaults to `false`.
    pub pdf_ua: Option<bool>,
    /// The document's natural-language tag (RFC 3066, e.g. `en-US`), written as
    /// the catalog `/Lang` for tagged PDF. Only meaningful with `pdfUa`.
    pub lang: Option<String>,
    /// Emit fills in DeviceCMYK instead of DeviceRGB for print workflows.
    /// Defaults to `false`.
    pub cmyk: Option<bool>,
    /// AES-256 password protection. Set `userPassword` (required to open) and
    /// optionally `ownerPassword`. Encrypted output is intentionally
    /// non-deterministic. Omit for an unencrypted, byte-deterministic PDF.
    pub encryption: Option<JsEncryption>,
    /// Foreign PDF blobs glued (page by page) AFTER the rendered pages, in order.
    /// Each entry is a complete PDF document. Omit to append nothing.
    pub append_pdfs: Option<Vec<Buffer>>,
}

/// AES-256 password-encryption settings. `userPassword` is required to open the
/// document; `ownerPassword` (when set) grants full permissions. Permission bits
/// default to all-granted; set a field to `false` to clear it for a user-password
/// open.
#[napi(object)]
pub struct JsEncryption {
    /// Password required to open the document (required).
    pub user_password: String,
    /// Owner password granting full permissions. Defaults to the user password.
    pub owner_password: Option<String>,
    /// Allow printing. Defaults to `true`.
    pub print: Option<bool>,
    /// Allow modifying contents. Defaults to `true`.
    pub modify: Option<bool>,
    /// Allow copying/extracting text and graphics. Defaults to `true`.
    pub copy: Option<bool>,
    /// Allow adding/modifying annotations. Defaults to `true`.
    pub annotate: Option<bool>,
    /// Allow filling existing form fields. Defaults to `true`.
    pub fill_forms: Option<bool>,
    /// Allow accessibility text extraction. Defaults to `true`.
    pub accessibility: Option<bool>,
    /// Allow document assembly (insert/rotate/delete pages). Defaults to `true`.
    pub assemble: Option<bool>,
    /// Allow full-resolution printing. Defaults to `true`.
    pub high_quality_print: Option<bool>,
}

impl JsEncryption {
    /// Lower into the core [`Encryption`], applying permission overrides onto the
    /// all-granted default.
    fn into_core(self) -> Encryption {
        let mut perms = Permissions::all();
        if let Some(v) = self.print {
            perms.print = v;
        }
        if let Some(v) = self.modify {
            perms.modify = v;
        }
        if let Some(v) = self.copy {
            perms.copy = v;
        }
        if let Some(v) = self.annotate {
            perms.annotate = v;
        }
        if let Some(v) = self.fill_forms {
            perms.fill_forms = v;
        }
        if let Some(v) = self.accessibility {
            perms.accessibility = v;
        }
        if let Some(v) = self.assemble {
            perms.assemble = v;
        }
        if let Some(v) = self.high_quality_print {
            perms.high_quality_print = v;
        }
        Encryption {
            user_password: self.user_password,
            owner_password: self.owner_password,
            permissions: perms,
        }
    }
}

/// A page watermark. Set `text` for a shaped-word mark (defaulting to `DRAFT`
/// when omitted) or `image` for a raster mark resolved by name through `images`;
/// the two are mutually exclusive (`image` wins if both are set). All other
/// fields are optional and take the core's `DRAFT` preset defaults.
#[napi(object)]
pub struct JsWatermark {
    /// The word to stamp (text mark). Defaults to `DRAFT` when omitted.
    pub text: Option<String>,
    /// Fill color `#rrggbb` for the text mark. Defaults to gray.
    pub color: Option<String>,
    /// The image name (image mark), resolved against `RenderOptions.images`.
    pub image: Option<String>,
    /// Fill opacity `0.0..=1.0`. Defaults to the preset (0.25 text / 1.0 image).
    pub opacity: Option<f64>,
    /// Rotation in degrees for the text mark. Defaults to 45.
    pub angle: Option<f64>,
    /// Tile the image mark across the page instead of centering it.
    pub tiled: Option<bool>,
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
    /// Parse `fonts` (each `{ data, family, weight?, italic? }`) once into a
    /// reusable handle. Do this at startup, then reuse it across renders.
    #[napi(factory)]
    pub fn load(fonts: Vec<JsFont>) -> Fonts {
        Fonts {
            inner: Arc::new(build_registry(fonts)),
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

/// Compile `template_html` into a reusable [`Program`]. `opts` is the optional
/// `{ partials?, missingPolicy?, includeMaxDepth? }` object (mirrors the wasm
/// binding); an unknown/omitted field falls back to the [`CompileOptions`]
/// default.
#[napi]
pub fn compile(template_html: String, opts: Option<Value>) -> napi::Result<Program> {
    let (program, _diags) =
        core_compile(&template_html, &compile_options(opts)).map_err(errors::from_compile)?;
    Ok(Program { inner: program })
}

/// The JS shape of the compile options: `{ partials?, missingPolicy?,
/// includeMaxDepth? }`. Every field is optional and defaulted.
#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct JsCompileOptions {
    partials: HashMap<String, String>,
    missing_policy: Option<String>,
    include_max_depth: Option<u32>,
}

/// Lower the optional JS compile-options value into core [`CompileOptions`],
/// treating a malformed object as the default rather than erroring.
fn compile_options(opts: Option<Value>) -> CompileOptions {
    let js: JsCompileOptions = opts
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let defaults = CompileOptions::default();
    CompileOptions {
        partials: js.partials,
        missing_policy: parse_missing_policy(js.missing_policy.as_deref()),
        include_max_depth: js.include_max_depth.unwrap_or(defaults.include_max_depth),
    }
}

/// Map a `missingPolicy` string to the core enum; unknown/absent stays strict.
fn parse_missing_policy(name: Option<&str>) -> MissingPolicy {
    match name {
        Some("empty") | Some("lenient") => MissingPolicy::Empty,
        _ => MissingPolicy::Strict,
    }
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

/// Glue one or more foreign PDF documents after `base`, page by page, returning
/// the merged PDF. Equivalent to `RenderOptions.appendPdfs` but usable on
/// already-emitted bytes. Throws `TurboPdfError` if any input fails to parse or
/// the inputs together contain no pages.
#[napi]
pub fn append_pdf(base: Buffer, extras: Vec<Buffer>) -> napi::Result<Buffer> {
    let refs: Vec<&[u8]> = extras.iter().map(|b| b.as_ref()).collect();
    let merged = append_pdfs(base.as_ref(), &refs).map_err(errors::from_append)?;
    Ok(merged.into())
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
            owned_registry = build_registry(opts.fonts.unwrap_or_default());
            &owned_registry
        }
    };

    // Build the name-keyed image resolver from this call's images. When empty we
    // keep the `&NoImages` path so a no-image render stays byte-identical.
    let resolver = MapResolver::new(opts.images.unwrap_or_default());

    let mut diags = Diagnostics::default();
    let conformance = Conformance {
        pdf_a: opts.pdf_a.unwrap_or(false),
        pdf_ua: opts.pdf_ua.unwrap_or(false),
        lang: opts.lang,
        cmyk: opts.cmyk.unwrap_or(false),
        encryption: opts.encryption.map(JsEncryption::into_core),
    };
    let append = opts.append_pdfs.unwrap_or_default();
    let emit = emit_options(opts.meta, opts.watermark, registry, conformance);

    let pages = {
        let inputs = RenderInputs {
            program,
            data: &data,
            cascade: &cascade,
            at_rules: &at_rules,
            fonts: registry,
            images: image_source(&resolver),
            now: opts.now,
        };
        render_pages(&inputs, &mut diags).map_err(errors::from_render)?
    };
    let page_count = pages.len() as u32;
    let pdf = emit_pdf_with_images(&pages, &emit, image_source(&resolver));
    let pdf = apply_append(pdf, &append)?;

    Ok(RenderResult {
        pdf: pdf.into(),
        diagnostics: diagnostics_to_js(&diags),
        page_count,
    })
}

/// The per-render conformance / encryption toggles lowered to core types,
/// extracted from [`RenderOptions`] before it is partly consumed.
struct Conformance {
    pdf_a: bool,
    pdf_ua: bool,
    lang: Option<String>,
    cmyk: bool,
    encryption: Option<Encryption>,
}

/// Glue each foreign PDF blob after the rendered `pdf`, page by page. A parse
/// failure on any input becomes a fatal `TurboPdfError`. With no extras the input
/// passes through unchanged.
fn apply_append(pdf: Vec<u8>, extras: &[Buffer]) -> napi::Result<Vec<u8>> {
    if extras.is_empty() {
        return Ok(pdf);
    }
    let refs: Vec<&[u8]> = extras.iter().map(|b| b.as_ref()).collect();
    append_pdfs(&pdf, &refs).map_err(errors::from_append)
}

/// The image source for layout/emit: the caller's resolver when it carries
/// images, else the zero-image [`NoImages`] so the no-image path is identical.
fn image_source(resolver: &MapResolver) -> &dyn turbo_pdf_core::ImageResolver {
    if resolver.is_empty() {
        &NoImages
    } else {
        resolver
    }
}

/// Translate optional [`DocMeta`] + watermark into core [`EmitOptions`]. A text
/// watermark is shaped with the registry's first face; with no face the text
/// mark is dropped (nothing to shape with).
fn emit_options(
    meta: Option<DocMeta>,
    watermark: Option<JsWatermark>,
    registry: &FontRegistry,
    conformance: Conformance,
) -> EmitOptions {
    let mut opts = EmitOptions::default();
    if let Some(m) = meta {
        opts.title = m.title;
        opts.author = m.author;
        opts.subject = m.subject;
        opts.keywords = m.keywords;
        opts.creation_date = m.creation_date;
    }
    opts.watermark = watermark.and_then(|w| build_watermark(w, registry));
    opts.cmyk = conformance.cmyk;
    opts.pdf_a = conformance.pdf_a;
    opts.pdf_ua = conformance.pdf_ua;
    opts.lang = conformance.lang;
    opts.encryption = conformance.encryption;
    opts
}

/// Build a core [`Watermark`] from the JS shape. `image` (if set) makes a raster
/// mark; otherwise a text mark seeded from the `DRAFT` preset of the registry's
/// first face. Returns `None` for a text mark with no face available.
fn build_watermark(w: JsWatermark, registry: &FontRegistry) -> Option<Watermark> {
    if let Some(name) = w.image {
        return Some(Watermark::Image(ImageWatermark {
            name,
            opacity: w.opacity.map(|o| o as f32).unwrap_or(1.0),
            tiled: w.tiled.unwrap_or(false),
        }));
    }
    let face = registry.select(&[], 400, false)?.clone();
    let mut mark = TextWatermark::draft(face);
    apply_text_overrides(&mut mark, w);
    Some(Watermark::Text(Box::new(mark)))
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
        mark.opacity = opacity as f32;
    }
    if let Some(angle) = w.angle {
        mark.angle_deg = angle as f32;
    }
}

/// Parse a `#rrggbb` (or `rrggbb`) hex color into an opaque [`Rgba`]. Returns
/// `None` for any malformed string, leaving the preset color in place.
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
