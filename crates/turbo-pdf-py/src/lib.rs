//! turbo-pdf PyO3 binding (PyPI: `turbo-html2pdf`).
//!
//! Exposes the compile -> render -> emit pipeline of `turbo-html2pdf-core` to Python,
//! mirroring the Node N-API binding 1:1. A template is compiled once into a
//! [`Program`] (a `Send + Sync` native handle) and rendered against data as many
//! times as needed; a one-shot [`render`] convenience does both in a single call.
//!
//! ## Boundary contract
//! * Input `data` is an ordinary Python value (dict/list/scalar), bridged to
//!   `serde_json::Value` via `pythonize`.
//! * The rendered PDF crosses back as Python `bytes`.
//! * Fatal errors are raised as a typed `TurboPdfError` (see `errors`) carrying
//!   `.code`/`.span`; non-fatal lints are *returned* in the result's
//!   `diagnostics` list, never raised.
//!
//! The product surface is this thin marshaling layer; all rendering logic lives
//! in the core crate. This crate is a cdylib that tarpaulin cannot
//! line-instrument, so it is excluded from the coverage gate and kept
//! deliberately minimal and mechanical, with every branch pushed into the core.

#![deny(clippy::all)]

mod convert;
mod errors;

use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use serde_json::Value;

use turbo_html2pdf_core::style::{parse_stylesheet, TokenSet};
use turbo_html2pdf_core::{
    append_pdfs, build_cascade, compile as core_compile, emit_pdf, render_pages, CompileOptions,
    Diagnostics, EmitOptions, Encryption, FontRegistry, Permissions, RenderInputs,
};

use convert::{build_registry, diagnostics_to_py};
use errors::TurboPdfError;

/// A reusable, pre-parsed set of fonts. Build it ONCE (e.g. warm it at server
/// startup) with [`Fonts::load`] and pass the handle to every `render` call: the
/// registry is shared (cheap `Arc` clone), so font programs are parsed once
/// instead of on every request. Omit it to fall back to per-call `fonts=`.
#[pyclass(frozen, module = "turbo_html2pdf")]
pub struct Fonts {
    inner: Arc<FontRegistry>,
}

#[pymethods]
impl Fonts {
    /// Parse `fonts` (raw OpenType/TrueType byte blobs) once into a reusable
    /// handle. Do this at startup, then reuse it across renders.
    #[staticmethod]
    pub fn load(fonts: Vec<Vec<u8>>) -> Fonts {
        Fonts {
            inner: Arc::new(build_registry(&fonts)),
        }
    }
}

/// A compiled, reusable template program. Compiling is the expensive parse step;
/// render it against many data sets. The handle is thread-safe.
#[pyclass(frozen, module = "turbo_html2pdf")]
pub struct Program {
    inner: turbo_html2pdf_core::Program,
}

#[pymethods]
impl Program {
    /// Render this program to a PDF, returned as `bytes`. Raises `TurboPdfError`
    /// on a fatal compile/render fault; lints come back via [`Program::render`]'s
    /// companion — here only the PDF bytes are returned to match the N-API
    /// `program.render(...) -> bytes` shape. See [`render_full`] for diagnostics.
    #[pyo3(signature = (data=None, css=String::new(), fonts=None, images=None, meta=None, now=None, pdf_a=false, pdf_ua=false, to_unicode=false, lang=None, cmyk=false, encryption=None, append_pdfs=None))]
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        py: Python<'_>,
        data: Option<Bound<'_, PyAny>>,
        css: String,
        fonts: Option<&Fonts>,
        images: Option<Vec<Vec<u8>>>,
        meta: Option<Bound<'_, PyDict>>,
        now: Option<i64>,
        pdf_a: bool,
        pdf_ua: bool,
        to_unicode: bool,
        lang: Option<String>,
        cmyk: bool,
        encryption: Option<Bound<'_, PyDict>>,
        append_pdfs: Option<Vec<Vec<u8>>>,
    ) -> PyResult<Py<PyBytes>> {
        let conf = Conformance::new(pdf_a, pdf_ua, to_unicode, lang, cmyk, encryption)?;
        let opts = RenderArgs::new(py, data, css, fonts_handle(fonts), images, meta, now, conf)?;
        let out = run_pipeline(py, &self.inner, opts, append_pdfs)?;
        Ok(out.pdf)
    }

    /// Like [`Program::render`] but returns `(pdf_bytes, diagnostics, page_count)`
    /// so callers can inspect the non-fatal lints and page total.
    #[pyo3(signature = (data=None, css=String::new(), fonts=None, images=None, meta=None, now=None, pdf_a=false, pdf_ua=false, to_unicode=false, lang=None, cmyk=false, encryption=None, append_pdfs=None))]
    #[allow(clippy::too_many_arguments)]
    pub fn render_full<'py>(
        &self,
        py: Python<'py>,
        data: Option<Bound<'py, PyAny>>,
        css: String,
        fonts: Option<&Fonts>,
        images: Option<Vec<Vec<u8>>>,
        meta: Option<Bound<'py, PyDict>>,
        now: Option<i64>,
        pdf_a: bool,
        pdf_ua: bool,
        to_unicode: bool,
        lang: Option<String>,
        cmyk: bool,
        encryption: Option<Bound<'py, PyDict>>,
        append_pdfs: Option<Vec<Vec<u8>>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let conf = Conformance::new(pdf_a, pdf_ua, to_unicode, lang, cmyk, encryption)?;
        let opts = RenderArgs::new(py, data, css, fonts_handle(fonts), images, meta, now, conf)?;
        let out = run_pipeline(py, &self.inner, opts, append_pdfs)?;
        out.into_py_tuple(py)
    }

    /// Whether the source declared a `<t:running-header>`.
    pub fn has_header(&self) -> bool {
        self.inner.has_header()
    }

    /// Whether the source declared a `<t:running-footer>`.
    pub fn has_footer(&self) -> bool {
        self.inner.has_footer()
    }
}

/// Compile `template_html` into a reusable [`Program`]. `opts` is reserved for
/// future compile knobs (partials, missing-policy) and currently ignored; the
/// default [`CompileOptions`] is used.
#[pyfunction]
#[pyo3(signature = (template_html, opts=None))]
pub fn compile(
    py: Python<'_>,
    template_html: &str,
    opts: Option<Bound<'_, PyAny>>,
) -> PyResult<Program> {
    let _ = opts;
    let (program, _diags) = core_compile(template_html, &CompileOptions::default())
        .map_err(|e| errors::from_compile(py, e))?;
    Ok(Program { inner: program })
}

/// One-shot convenience: compile `template_html` and render it in a single call,
/// returning the PDF `bytes`.
#[pyfunction]
#[pyo3(signature = (template_html, data=None, css=String::new(), fonts=None, images=None, meta=None, now=None, pdf_a=false, pdf_ua=false, to_unicode=false, lang=None, cmyk=false, encryption=None, append_pdfs=None))]
#[allow(clippy::too_many_arguments)]
pub fn render(
    py: Python<'_>,
    template_html: &str,
    data: Option<Bound<'_, PyAny>>,
    css: String,
    fonts: Option<&Fonts>,
    images: Option<Vec<Vec<u8>>>,
    meta: Option<Bound<'_, PyDict>>,
    now: Option<i64>,
    pdf_a: bool,
    pdf_ua: bool,
    to_unicode: bool,
    lang: Option<String>,
    cmyk: bool,
    encryption: Option<Bound<'_, PyDict>>,
    append_pdfs: Option<Vec<Vec<u8>>>,
) -> PyResult<Py<PyBytes>> {
    let (program, _diags) = core_compile(template_html, &CompileOptions::default())
        .map_err(|e| errors::from_compile(py, e))?;
    let conf = Conformance::new(pdf_a, pdf_ua, to_unicode, lang, cmyk, encryption)?;
    let opts = RenderArgs::new(py, data, css, fonts_handle(fonts), images, meta, now, conf)?;
    let out = run_pipeline(py, &program, opts, append_pdfs)?;
    Ok(out.pdf)
}

/// Glue one or more foreign PDF documents after `base`, page by page, returning
/// the merged PDF as `bytes`. Equivalent to the `append_pdfs=` render kwarg but
/// usable on already-emitted bytes. Raises `TurboPdfError` if any input fails to
/// parse or the inputs together contain no pages.
#[pyfunction]
pub fn append_pdf(py: Python<'_>, base: Vec<u8>, extras: Vec<Vec<u8>>) -> PyResult<Py<PyBytes>> {
    let merged = merge_appended(base, &extras).map_err(|e| errors::from_append(py, e))?;
    Ok(PyBytes::new(py, &merged).unbind())
}

/// The per-render conformance / encryption toggles lowered to core types.
#[derive(Default)]
struct Conformance {
    pdf_a: bool,
    pdf_ua: bool,
    to_unicode: bool,
    lang: Option<String>,
    cmyk: bool,
    encryption: Option<Encryption>,
}

impl Conformance {
    /// Parse the conformance kwargs, lowering the optional `encryption` dict into
    /// the core [`Encryption`] type.
    fn new(
        pdf_a: bool,
        pdf_ua: bool,
        to_unicode: bool,
        lang: Option<String>,
        cmyk: bool,
        encryption: Option<Bound<'_, PyDict>>,
    ) -> PyResult<Conformance> {
        Ok(Conformance {
            pdf_a,
            pdf_ua,
            to_unicode,
            lang,
            cmyk,
            encryption: encryption.map(|d| encryption_dict(&d)).transpose()?,
        })
    }

    /// Apply the toggles onto an [`EmitOptions`] already carrying metadata.
    fn apply(self, opts: &mut EmitOptions) {
        opts.cmyk = self.cmyk;
        opts.pdf_a = self.pdf_a;
        opts.pdf_ua = self.pdf_ua;
        opts.to_unicode = self.to_unicode;
        opts.lang = self.lang;
        opts.encryption = self.encryption;
    }
}

/// Lower an `encryption=` dict into the core [`Encryption`]. Recognized keys:
/// `user_password` (str, required), `owner_password` (str) and the permission
/// flags (`print`, `modify`, `copy`, `annotate`, `fill_forms`, `accessibility`,
/// `assemble`, `high_quality_print`) — each a bool defaulting to granted.
fn encryption_dict(d: &Bound<'_, PyDict>) -> PyResult<Encryption> {
    Ok(Encryption {
        user_password: require_user_password(d)?,
        owner_password: opt_str(d, "owner_password")?,
        permissions: permissions_dict(d)?,
    })
}

/// Extract the required `user_password` string off the `encryption` dict.
fn require_user_password(d: &Bound<'_, PyDict>) -> PyResult<String> {
    match d.get_item("user_password")? {
        Some(v) => v.extract(),
        None => Err(TurboPdfError::new_err(
            "encryption requires a 'user_password' string",
        )),
    }
}

/// Read the permission-flag keys off the `encryption` dict onto an all-granted
/// default; an omitted flag stays granted.
fn permissions_dict(d: &Bound<'_, PyDict>) -> PyResult<Permissions> {
    let mut perms = Permissions::all();
    let fields: [(&str, &mut bool); 8] = [
        ("print", &mut perms.print),
        ("modify", &mut perms.modify),
        ("copy", &mut perms.copy),
        ("annotate", &mut perms.annotate),
        ("fill_forms", &mut perms.fill_forms),
        ("accessibility", &mut perms.accessibility),
        ("assemble", &mut perms.assemble),
        ("high_quality_print", &mut perms.high_quality_print),
    ];
    for (key, slot) in fields {
        if let Some(v) = opt_bool_key(d, key)? {
            *slot = v;
        }
    }
    Ok(perms)
}

/// Borrow the shared registry out of an optional warm [`Fonts`] handle.
fn fonts_handle(fonts: Option<&Fonts>) -> Option<Arc<FontRegistry>> {
    fonts.map(|f| f.inner.clone())
}

/// Normalized, already-marshaled render arguments. Built once at the boundary so
/// the pipeline itself stays free of Python-type handling.
struct RenderArgs {
    data: Value,
    css: String,
    registry: Arc<FontRegistry>,
    meta: EmitOptions,
    now: Option<i64>,
}

impl RenderArgs {
    /// Marshal raw Python inputs into native render arguments. `images` is
    /// accepted for API parity but not yet embedded (see [`run_pipeline`]). The
    /// conformance/encryption toggles are applied onto the emitter options here.
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        data: Option<Bound<'_, PyAny>>,
        css: String,
        warm: Option<Arc<FontRegistry>>,
        images: Option<Vec<Vec<u8>>>,
        meta: Option<Bound<'_, PyDict>>,
        now: Option<i64>,
        conformance: Conformance,
    ) -> PyResult<RenderArgs> {
        let _ = images;
        let mut emit = emit_options(py, meta)?;
        conformance.apply(&mut emit);
        Ok(RenderArgs {
            data: data_value(data)?,
            css,
            registry: warm.unwrap_or_else(|| Arc::new(build_registry(&[]))),
            meta: emit,
            now,
        })
    }
}

/// The outcome of a render pass, ready to hand back to Python.
struct RenderOutput {
    pdf: Py<PyBytes>,
    diagnostics: Diagnostics,
    page_count: usize,
}

impl RenderOutput {
    /// Build the `(bytes, [diagnostic...], page_count)` tuple for `render_full`.
    fn into_py_tuple<'py>(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let diags = PyList::new(py, diagnostics_to_py(py, &self.diagnostics)?)?;
        let tuple = (self.pdf, diags, self.page_count);
        Ok(tuple.into_pyobject(py)?.into_any())
    }
}

/// Convert the optional Python `data` object to a `serde_json::Value`, defaulting
/// to JSON `null` when omitted.
fn data_value(data: Option<Bound<'_, PyAny>>) -> PyResult<Value> {
    match data {
        None => Ok(Value::Null),
        Some(obj) => pythonize::depythonize(&obj).map_err(PyErr::from),
    }
}

/// The shared render pipeline: cascade + geometry + fonts -> `render_pages` ->
/// `emit_pdf`. Diagnostics flow into the result; only fatal faults raise.
fn run_pipeline(
    py: Python<'_>,
    program: &turbo_html2pdf_core::Program,
    opts: RenderArgs,
    append: Option<Vec<Vec<u8>>>,
) -> PyResult<RenderOutput> {
    let cascade = build_cascade(&opts.css, "", TokenSet::default());
    let at_rules = parse_stylesheet(&opts.css).at_rules;
    let inputs = RenderInputs {
        program,
        data: &opts.data,
        cascade: &cascade,
        at_rules: &at_rules,
        fonts: &opts.registry,
        // Image embedding (§7.4) is not yet surfaced through the Python binding;
        // the `images` arg is accepted so the API is stable but has no effect.
        images: &turbo_html2pdf_core::NoImages,
        now: opts.now,
    };

    let mut diags = Diagnostics::default();
    let pages = render_pages(&inputs, &mut diags).map_err(|e| errors::from_render(py, e))?;
    let page_count = pages.len();
    let pdf = emit_pdf(&pages, &opts.meta);
    let pdf =
        merge_appended(pdf, &append.unwrap_or_default()).map_err(|e| errors::from_append(py, e))?;
    Ok(RenderOutput {
        page_count,
        pdf: PyBytes::new(py, &pdf).unbind(),
        diagnostics: diags,
    })
}

/// Translate an optional `meta` dict into core [`EmitOptions`]. Recognized keys:
/// `title`, `author`, `subject`, `keywords` (str) and `creation_date` (int,
/// Unix seconds). Unknown keys are ignored.
fn emit_options(py: Python<'_>, meta: Option<Bound<'_, PyDict>>) -> PyResult<EmitOptions> {
    let _ = py;
    match meta {
        None => Ok(EmitOptions::default()),
        Some(m) => meta_dict_to_emit(&m),
    }
}

/// The four string-valued info fields, paired with the setter that writes each
/// onto the [`EmitOptions`] being built. Kept as data so [`meta_dict_to_emit`]
/// stays a single loop (one branch) rather than five sequential `?` extractions.
type StrSetter = fn(&mut EmitOptions, Option<String>);
const STR_FIELDS: [(&str, StrSetter); 4] = [
    ("title", |o, v| o.title = v),
    ("author", |o, v| o.author = v),
    ("subject", |o, v| o.subject = v),
    ("keywords", |o, v| o.keywords = v),
];

/// Read the recognized keys off a present `meta` dict into [`EmitOptions`].
fn meta_dict_to_emit(m: &Bound<'_, PyDict>) -> PyResult<EmitOptions> {
    let mut out = EmitOptions::default();
    for (key, set) in STR_FIELDS {
        set(&mut out, opt_str(m, key)?);
    }
    out.creation_date = opt_i64(m, "creation_date")?;
    Ok(out)
}

/// Extract an optional `str` value for `key` from a meta dict.
fn opt_str(m: &Bound<'_, PyDict>, key: &str) -> PyResult<Option<String>> {
    match m.get_item(key)? {
        Some(v) => Ok(Some(v.extract()?)),
        None => Ok(None),
    }
}

/// Extract an optional `int` value for `key` from a meta dict.
fn opt_i64(m: &Bound<'_, PyDict>, key: &str) -> PyResult<Option<i64>> {
    match m.get_item(key)? {
        Some(v) => Ok(Some(v.extract()?)),
        None => Ok(None),
    }
}

/// Extract an optional `bool` value for `key` from a dict.
fn opt_bool_key(m: &Bound<'_, PyDict>, key: &str) -> PyResult<Option<bool>> {
    match m.get_item(key)? {
        Some(v) => Ok(Some(v.extract()?)),
        None => Ok(None),
    }
}

/// Glue each foreign PDF blob after `base`, page by page. A parse failure becomes
/// an [`turbo_html2pdf_core::AppendError`]; with no extras `base` is returned as-is.
fn merge_appended(
    base: Vec<u8>,
    extras: &[Vec<u8>],
) -> Result<Vec<u8>, turbo_html2pdf_core::AppendError> {
    if extras.is_empty() {
        return Ok(base);
    }
    let refs: Vec<&[u8]> = extras.iter().map(Vec::as_slice).collect();
    append_pdfs(&base, &refs)
}

/// Register the binding's classes (`Program`, `Fonts`) on the module.
fn register_classes(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Program>()?;
    m.add_class::<Fonts>()
}

/// Register the module-level functions (`compile`, `render`, `append_pdf`).
fn register_functions(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(compile, m)?)?;
    register_render_functions(m)
}

/// Register the render/IO module functions (`render`, `append_pdf`).
fn register_render_functions(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(render, m)?)?;
    m.add_function(wrap_pyfunction!(append_pdf, m)?)
}

/// The Python extension module `turbo_html2pdf._turbo_html2pdf`. Re-exported by
/// the pure-Python `turbo_html2pdf/__init__.py` shim.
#[pymodule]
fn _turbo_html2pdf(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_classes(m)?;
    register_functions(m)?;
    m.add("TurboPdfError", m.py().get_type::<TurboPdfError>())
}
