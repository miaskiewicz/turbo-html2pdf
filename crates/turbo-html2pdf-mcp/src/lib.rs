//! turbo-html2pdf MCP server — the protocol surface.
//!
//! A hand-rolled **JSON-RPC 2.0** server (no SDK) exposing the turbo-html2pdf
//! HTML/CSS + Jinja → PDF engine as MCP tools over stdio. Mirrors the sibling
//! turbo-xlsx-mcp / turbo-parsepdf-mcp layout: this `lib` is the testable
//! protocol core ([`handle`]); `main.rs` is the stdio pump.
//!
//! Tools (all converge on `turbo-html2pdf-core`, the same pipeline the napi / python
//! bindings drive):
//!   - `render`         — a template + data/CSS/fonts/images/… → `.pdf`
//!   - `append_pdf`     — glue foreign PDF documents after a base PDF
//!   - `check_template` — compile a template; report header/footer + faults
//!
//! Binary I/O is path-or-base64: every binary input (fonts, images, appended
//! PDFs, the append base) takes `path` OR `dataBase64`; every PDF output takes an
//! optional `out` path (→ `{ path, bytes }`) and otherwise returns
//! `{ base64, bytes }`. Base64 is hand-rolled to keep the dep set at
//! `serde`/`serde_json` (plus the core) — no transitive surprises.
//!
//! The full binding surface is exposed: PDF/A, PDF/UA, CMYK, AES-256 encryption,
//! watermarks, named images, foreign-PDF append, and the bundled fonts all flow
//! through `render`. Only `svg` is omitted, matching the standard
//! `turbo-html2pdf` npm package.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use serde_json::{json, Value};

use turbo_html2pdf_core::style::{parse_stylesheet, TokenSet};
use turbo_html2pdf_core::{
    append_pdfs, build_cascade, compile as core_compile, emit_pdf_with_images, render_pages,
    CompileOptions, Diagnostics, EmitOptions, Encryption, FontFace, FontRegistry, ImageResolver,
    ImageWatermark, Lint, LintCode, MissingPolicy, NoImages, Page, Permissions, RenderInputs, Rgba,
    TextWatermark, Watermark,
};

/// Per-connection state. Currently stateless — the tools are pure utilities —
/// but kept as a handle so future stateful tools slot in without changing
/// [`handle`]'s signature, exactly like the sibling MCP servers' sessions.
#[derive(Default)]
pub struct Session;

impl Session {
    /// A fresh session.
    pub fn new() -> Self {
        Session
    }
}

/// Dispatch one JSON-RPC request. Returns the response value to write back, or
/// `None` for a notification (a message with no `id`, which must get no reply).
pub fn handle(session: &mut Session, req: &Value) -> Option<Value> {
    let id = req.get("id").cloned()?;
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    match method {
        "initialize" => Some(ok(id, initialize_result(session))),
        "tools/list" => Some(ok(id, json!({ "tools": tools() }))),
        "tools/call" => Some(tools_call(id, req.get("params"))),
        other => Some(err(id, &format!("unknown method: {other}"))),
    }
}

/// The `initialize` result: protocol version, tool capability, server identity.
fn initialize_result(_session: &mut Session) -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "turbo-html2pdf-mcp", "version": env!("CARGO_PKG_VERSION") }
    })
}

// ---- tools/call dispatch ----------------------------------------------------

/// Route a `tools/call`: validate params, run the tool, wrap success or the
/// error string in the MCP content envelope (a tool error is a normal result
/// with `isError: true`, not a JSON-RPC protocol error).
fn tools_call(id: Value, params: Option<&Value>) -> Value {
    let params = match params {
        Some(p) => p,
        None => return err(id, "tools/call: missing params"),
    };
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let empty = json!({});
    let args = params.get("arguments").unwrap_or(&empty);
    match call_tool(name, args) {
        Ok(result) => ok(id, tool_content(result)),
        Err(message) => ok(id, tool_error(&message)),
    }
}

/// Dispatch by tool name.
fn call_tool(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "render" => tool_render(args),
        "append_pdf" => tool_append_pdf(args),
        "check_template" => tool_check_template(args),
        other => Err(format!("unknown tool: {other}")),
    }
}

// ---- render -----------------------------------------------------------------

/// `render`: compile `templateHtml` and render it to a PDF with the full option
/// surface (data, CSS, fonts, images, metadata, watermark, conformance,
/// encryption, appended PDFs). Returns `{ base64 | path, bytes, pageCount,
/// diagnostics }`.
fn tool_render(args: &Value) -> Result<Value, String> {
    validate_encryption(args)?;
    let (pdf, page_count, diags) = render_pdf(args)?;
    Ok(render_result(emit_blob(args, &pdf)?, page_count, &diags))
}

/// An `encryption` object must carry a `userPassword` string — otherwise the
/// document would be "encrypted" with an empty open password. Mirrors the
/// napi/python bindings, which both require it.
fn validate_encryption(args: &Value) -> Result<(), String> {
    match args.get("encryption") {
        Some(e) if e.get("userPassword").and_then(Value::as_str).is_none() => {
            Err("encryption requires a 'userPassword' string".into())
        }
        _ => Ok(()),
    }
}

/// The render pipeline: fonts/images → layout → emit → append. Returns the PDF
/// bytes, the page count, and the collected lints.
fn render_pdf(args: &Value) -> Result<(Vec<u8>, usize, Diagnostics), String> {
    let registry = build_registry(args)?;
    let resolver = build_resolver(args)?;
    let (pages, diags) = layout_pages(args, &registry, &resolver)?;
    let page_count = pages.len();
    let emit = build_emit(args, &registry);
    let pdf = emit_pdf_with_images(&pages, &emit, image_source(&resolver));
    let pdf = finish_pdf(args, pdf)?;
    Ok((pdf, page_count, diags))
}

/// Compile, cascade, and lay out the template into positioned pages plus lints.
fn layout_pages(
    args: &Value,
    registry: &FontRegistry,
    resolver: &MapResolver,
) -> Result<(Vec<Page>, Diagnostics), String> {
    let template = req_str(args, "templateHtml")?;
    let (program, _diags) = core_compile(&template, &compile_opts(args)).map_err(stringify)?;
    let css = opt_str(args, "css").unwrap_or_default();
    let data = args.get("data").cloned().unwrap_or(Value::Null);
    let cascade = build_cascade(&css, "", TokenSet::default());
    let at_rules = parse_stylesheet(&css).at_rules;
    let mut diags = Diagnostics::default();
    let inputs = RenderInputs {
        program: &program,
        data: &data,
        cascade: &cascade,
        at_rules: &at_rules,
        fonts: registry,
        images: image_source(resolver),
        now: args.get("now").and_then(Value::as_i64),
    };
    let pages = render_pages(&inputs, &mut diags).map_err(stringify)?;
    Ok((pages, diags))
}

/// Glue any `appendPdfs` after the rendered PDF (no-op when there are none).
fn finish_pdf(args: &Value, pdf: Vec<u8>) -> Result<Vec<u8>, String> {
    let append = read_blobs(args, "appendPdfs")?;
    apply_append(pdf, &append)
}

/// Merge the output envelope with the page count and the collected lints.
fn render_result(mut out: Value, page_count: usize, diags: &Diagnostics) -> Value {
    if let Some(map) = out.as_object_mut() {
        map.insert("pageCount".into(), json!(page_count));
        map.insert("diagnostics".into(), diagnostics_json(diags));
    }
    out
}

/// Build the optional compile knobs (`partials`, `missingPolicy`,
/// `includeMaxDepth`) from the render args; an absent/malformed field defaults.
fn compile_opts(args: &Value) -> CompileOptions {
    let defaults = CompileOptions::default();
    CompileOptions {
        partials: partials(args),
        missing_policy: missing_policy(args.get("missingPolicy").and_then(Value::as_str)),
        include_max_depth: args
            .get("includeMaxDepth")
            .and_then(Value::as_u64)
            .map(|d| d as u32)
            .unwrap_or(defaults.include_max_depth),
    }
}

/// The `partials` map (`{ name: source }`) for `{% include %}`, or empty.
fn partials(args: &Value) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(map) = args.get("partials").and_then(Value::as_object) {
        for (k, v) in map {
            if let Some(s) = v.as_str() {
                out.insert(k.clone(), s.to_string());
            }
        }
    }
    out
}

/// Map a `missingPolicy` string to the core enum; unknown/absent stays strict.
fn missing_policy(name: Option<&str>) -> MissingPolicy {
    match name {
        Some("empty") | Some("lenient") => MissingPolicy::Empty,
        _ => MissingPolicy::Strict,
    }
}

// ---- append_pdf -------------------------------------------------------------

/// `append_pdf`: glue each PDF in `extras` after `base`, page by page. `base` and
/// each extra are path-or-base64. Returns `{ base64 | path, bytes }`.
fn tool_append_pdf(args: &Value) -> Result<Value, String> {
    let base = read_blob(args.get("base").ok_or("append_pdf: missing 'base'")?)?;
    let extras = read_blobs(args, "extras")?;
    let refs: Vec<&[u8]> = extras.iter().map(Vec::as_slice).collect();
    let merged = append_pdfs(&base, &refs).map_err(stringify)?;
    emit_blob(args, &merged)
}

// ---- check_template ---------------------------------------------------------

/// `check_template`: compile `templateHtml` and report whether it declared a
/// running header / footer. A compile fault becomes a tool error.
fn tool_check_template(args: &Value) -> Result<Value, String> {
    let template = req_str(args, "templateHtml")?;
    let (program, _diags) = core_compile(&template, &compile_opts(args)).map_err(stringify)?;
    Ok(json!({
        "ok": true,
        "hasHeader": program.has_header(),
        "hasFooter": program.has_footer(),
    }))
}

// ---- fonts / images ---------------------------------------------------------

/// Build a [`FontRegistry`] from the optional `fonts` array (each
/// `{ path | dataBase64, family, weight?, italic? }`). Unparseable faces are
/// skipped, exactly like the napi binding.
fn build_registry(args: &Value) -> Result<FontRegistry, String> {
    let mut reg = FontRegistry::new();
    for face in array(args, "fonts") {
        if let Some(f) = font_face(face)? {
            reg.add(f);
        }
    }
    Ok(reg)
}

/// Parse one font face spec into a core [`FontFace`] (`None` if unparseable).
fn font_face(spec: &Value) -> Result<Option<FontFace>, String> {
    let data = read_blob(spec)?;
    let family = req_str(spec, "family")?;
    let weight = spec.get("weight").and_then(Value::as_u64).unwrap_or(400) as u16;
    let italic = spec.get("italic").and_then(Value::as_bool).unwrap_or(false);
    Ok(FontFace::from_web_font_bytes(data, family, weight, italic))
}

/// A name-keyed [`ImageResolver`] built from caller-supplied rasters.
struct MapResolver(HashMap<String, Vec<u8>>);

impl MapResolver {
    /// Whether any images were supplied (drives the `&NoImages` fast path).
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl ImageResolver for MapResolver {
    fn resolve(&self, name: &str) -> Option<&[u8]> {
        self.0.get(name).map(Vec::as_slice)
    }
}

/// Build the image resolver from the optional `images` array (each
/// `{ name, path | dataBase64 }`).
fn build_resolver(args: &Value) -> Result<MapResolver, String> {
    let mut map = HashMap::new();
    for img in array(args, "images") {
        let name = req_str(img, "name")?;
        map.insert(name, read_blob(img)?);
    }
    Ok(MapResolver(map))
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

// ---- emit options (metadata / watermark / conformance / encryption) ---------

/// Translate the `meta`, `watermark`, conformance and `encryption` args into
/// core [`EmitOptions`]. Mirrors the napi binding's `emit_options` + conformance.
fn build_emit(args: &Value, registry: &FontRegistry) -> EmitOptions {
    let mut opts = EmitOptions::default();
    apply_meta(&mut opts, args.get("meta"));
    opts.watermark = args
        .get("watermark")
        .and_then(|w| build_watermark(w, registry));
    opts.cmyk = flag(args, "cmyk");
    opts.pdf_a = flag(args, "pdfA");
    opts.pdf_ua = flag(args, "pdfUa");
    opts.lang = opt_str(args, "lang");
    opts.encryption = args.get("encryption").map(encryption);
    opts
}

/// Write the recognized `meta` keys (`title`/`author`/`subject`/`keywords`/
/// `creationDate`) onto the emit options. A missing `meta` leaves the defaults.
fn apply_meta(opts: &mut EmitOptions, meta: Option<&Value>) {
    let Some(m) = meta else { return };
    opts.title = opt_str(m, "title");
    opts.author = opt_str(m, "author");
    opts.subject = opt_str(m, "subject");
    opts.keywords = opt_str(m, "keywords");
    opts.creation_date = m.get("creationDate").and_then(Value::as_i64);
}

/// Build a core [`Watermark`] from the JS shape. `image` (if set) makes a raster
/// mark; otherwise a text mark seeded from the `DRAFT` preset of the registry's
/// first face. Returns `None` for a text mark with no face available.
fn build_watermark(w: &Value, registry: &FontRegistry) -> Option<Watermark> {
    if let Some(name) = opt_str(w, "image") {
        return Some(Watermark::Image(ImageWatermark {
            name,
            opacity: w.get("opacity").and_then(Value::as_f64).unwrap_or(1.0) as f32,
            tiled: w.get("tiled").and_then(Value::as_bool).unwrap_or(false),
        }));
    }
    let face = registry.select(&[], 400, false)?.clone();
    let mut mark = TextWatermark::draft(face);
    apply_text_overrides(&mut mark, w);
    Some(Watermark::Text(Box::new(mark)))
}

/// Apply the optional overrides onto a preset text watermark, leaving any
/// omitted field at its `DRAFT`-preset default.
fn apply_text_overrides(mark: &mut TextWatermark, w: &Value) {
    if let Some(text) = opt_str(w, "text") {
        mark.text = text;
    }
    if let Some(color) = opt_str(w, "color").as_deref().and_then(parse_hex_color) {
        mark.color = color;
    }
    if let Some(opacity) = w.get("opacity").and_then(Value::as_f64) {
        mark.opacity = opacity as f32;
    }
    if let Some(angle) = w.get("angle").and_then(Value::as_f64) {
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

/// Lower an `encryption` object into the core [`Encryption`], applying the
/// permission overrides onto an all-granted default. A missing `userPassword`
/// falls back to empty (the open password is still required to read).
fn encryption(e: &Value) -> Encryption {
    let mut perms = Permissions::all();
    let fields: [(&str, &mut bool); 8] = [
        ("print", &mut perms.print),
        ("modify", &mut perms.modify),
        ("copy", &mut perms.copy),
        ("annotate", &mut perms.annotate),
        ("fillForms", &mut perms.fill_forms),
        ("accessibility", &mut perms.accessibility),
        ("assemble", &mut perms.assemble),
        ("highQualityPrint", &mut perms.high_quality_print),
    ];
    for (key, slot) in fields {
        if let Some(v) = e.get(key).and_then(Value::as_bool) {
            *slot = v;
        }
    }
    Encryption {
        user_password: opt_str(e, "userPassword").unwrap_or_default(),
        owner_password: opt_str(e, "ownerPassword"),
        permissions: perms,
    }
}

// ---- diagnostics ------------------------------------------------------------

/// The collected lints as a JSON array of `{ code, message, line, col }`.
fn diagnostics_json(diags: &Diagnostics) -> Value {
    Value::Array(diags.lints.iter().map(lint_json).collect())
}

/// One lint → its JSON wire shape.
fn lint_json(lint: &Lint) -> Value {
    json!({
        "code": lint_code_str(lint.code),
        "message": lint.message,
        "line": lint.span.line,
        "col": lint.span.col,
    })
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

// ---- binary I/O (path-or-base64) --------------------------------------------

/// Read a binary blob from an object carrying `path` (a file) or `dataBase64`
/// (inline bytes).
fn read_blob(obj: &Value) -> Result<Vec<u8>, String> {
    if let Some(path) = obj.get("path").and_then(Value::as_str) {
        return std::fs::read(path).map_err(|e| format!("read {path}: {e}"));
    }
    if let Some(b64) = obj.get("dataBase64").and_then(Value::as_str) {
        return Ok(b64_decode(b64));
    }
    Err("provide 'path' or 'dataBase64'".into())
}

/// Read every blob in the optional array field `key` (each path-or-base64).
fn read_blobs(args: &Value, key: &str) -> Result<Vec<Vec<u8>>, String> {
    array(args, key).iter().map(read_blob).collect()
}

/// Glue each foreign PDF blob after `pdf`, page by page. With no extras `pdf`
/// passes through unchanged.
fn apply_append(pdf: Vec<u8>, extras: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    if extras.is_empty() {
        return Ok(pdf);
    }
    let refs: Vec<&[u8]> = extras.iter().map(Vec::as_slice).collect();
    append_pdfs(&pdf, &refs).map_err(stringify)
}

/// Either write the PDF bytes to `out` (→ `{ path, bytes }`) or return base64.
fn emit_blob(args: &Value, bytes: &[u8]) -> Result<Value, String> {
    match args.get("out").and_then(Value::as_str) {
        Some(path) => {
            std::fs::write(path, bytes).map_err(|e| format!("write {path}: {e}"))?;
            Ok(json!({ "path": path, "bytes": bytes.len() }))
        }
        None => Ok(json!({ "base64": b64_encode(bytes), "bytes": bytes.len() })),
    }
}

// ---- small arg helpers ------------------------------------------------------

/// The array at `key`, or an empty slice when absent / not an array.
fn array<'a>(args: &'a Value, key: &str) -> &'a [Value] {
    args.get(key).and_then(Value::as_array).map_or(&[], |v| v)
}

/// A required string field; an absent/non-string value is a tool error.
fn req_str(args: &Value, key: &str) -> Result<String, String> {
    opt_str(args, key).ok_or_else(|| format!("missing '{key}'"))
}

/// An optional string field.
fn opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(String::from)
}

/// An optional bool flag, defaulting to `false`.
fn flag(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

// ---- envelopes --------------------------------------------------------------

/// The MCP success envelope: the result as pretty JSON in a single text block.
fn tool_content(result: Value) -> Value {
    let text = serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string());
    json!({ "content": [ { "type": "text", "text": text } ], "isError": false })
}

/// The MCP tool-error envelope.
fn tool_error(message: &str) -> Value {
    json!({ "content": [ { "type": "text", "text": message } ], "isError": true })
}

/// A successful JSON-RPC response.
fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// A JSON-RPC method error (used only for protocol-level faults).
fn err(id: Value, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": message } })
}

/// Render any `Display` error as a `String` (the tools' uniform error channel).
fn stringify(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ---- tool catalog -----------------------------------------------------------

/// The advertised tool list (name + description + JSON-Schema input shape).
fn tools() -> Vec<Value> {
    vec![render_tool(), append_tool(), check_tool()]
}

/// The `render` tool descriptor — the full option surface.
fn render_tool() -> Value {
    tool_schema(
        "render",
        "Render an HTML/CSS + Jinja template to a PDF. 'templateHtml' is required; \
         'data' is the interpolation object, 'css' the author stylesheet (also \
         supplies @page geometry). 'fonts' / 'images' / 'appendPdfs' take \
         path-or-base64 blobs; 'meta', 'watermark', 'encryption', and the \
         'pdfA'/'pdfUa'/'cmyk'/'lang' flags drive conformance. Set 'out' to write \
         a file (→ { path, bytes }); otherwise returns { base64, bytes }. Always \
         includes 'pageCount' and 'diagnostics'.",
        json!({
            "type": "object",
            "properties": {
                "templateHtml": { "type": "string", "description": "The template source (HTML/CSS + Jinja)." },
                "data": { "description": "Data interpolated into the template; its keys are the root context ({{ key }}, {% for x in list %})." },
                "css": { "type": "string", "description": "Author CSS; also supplies @page size/margins." },
                "fonts": { "type": "array", "description": "Font faces: [{ path|dataBase64, family, weight?, italic? }]." },
                "images": { "type": "array", "description": "Named rasters: [{ name, path|dataBase64 }]." },
                "meta": { "type": "object", "description": "{ title?, author?, subject?, keywords?, creationDate? }." },
                "watermark": { "type": "object", "description": "{ text?, color?, image?, opacity?, angle?, tiled? }." },
                "now": { "type": "integer", "description": "Pin now() (Unix seconds) for deterministic output." },
                "pdfA": { "type": "boolean", "description": "Emit PDF/A-2b archival conformance." },
                "pdfUa": { "type": "boolean", "description": "Emit tagged / accessible PDF/UA-1." },
                "lang": { "type": "string", "description": "Natural-language tag (e.g. en-US); only with pdfUa." },
                "cmyk": { "type": "boolean", "description": "Emit fills in DeviceCMYK instead of DeviceRGB." },
                "encryption": { "type": "object", "description": "AES-256: { userPassword, ownerPassword?, print?, modify?, copy?, annotate?, fillForms?, accessibility?, assemble?, highQualityPrint? }." },
                "appendPdfs": { "type": "array", "description": "Foreign PDFs glued after the rendered pages: [{ path|dataBase64 }]." },
                "partials": { "type": "object", "description": "Partial templates for {% include %}: { name: source }." },
                "missingPolicy": { "type": "string", "enum": ["strict", "empty", "lenient"] },
                "includeMaxDepth": { "type": "integer" },
                "out": { "type": "string", "description": "Optional output file path." }
            },
            "required": ["templateHtml"]
        }),
    )
}

/// The `append_pdf` tool descriptor.
fn append_tool() -> Value {
    tool_schema(
        "append_pdf",
        "Glue one or more foreign PDF documents after a base PDF, page by page. \
         'base' and each entry of 'extras' are path-or-base64. Set 'out' to write \
         a file (→ { path, bytes }); otherwise returns { base64, bytes }.",
        json!({
            "type": "object",
            "properties": {
                "base": { "type": "object", "description": "The base PDF: { path | dataBase64 }." },
                "extras": { "type": "array", "description": "PDFs to append: [{ path | dataBase64 }]." },
                "out": { "type": "string", "description": "Optional output file path." }
            },
            "required": ["base"]
        }),
    )
}

/// The `check_template` tool descriptor.
fn check_tool() -> Value {
    tool_schema(
        "check_template",
        "Compile a template without rendering: validates the syntax and reports \
         whether it declared a running header / footer. Returns { ok, hasHeader, \
         hasFooter }; a syntax fault comes back as a tool error.",
        json!({
            "type": "object",
            "properties": {
                "templateHtml": { "type": "string" },
                "partials": { "type": "object" },
                "missingPolicy": { "type": "string", "enum": ["strict", "empty", "lenient"] },
                "includeMaxDepth": { "type": "integer" }
            },
            "required": ["templateHtml"]
        }),
    )
}

/// One tool descriptor.
fn tool_schema(name: &str, description: &str, input_schema: Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": input_schema })
}

// ---- base64 (hand-rolled, std-only) -----------------------------------------

const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with `=` padding.
fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(B64_ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(B64_ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(b64_tail(chunk.len() > 1, (n >> 6) as usize & 63));
        out.push(b64_tail(chunk.len() > 2, n as usize & 63));
    }
    out
}

/// A tail base64 char, or `=` padding when the source byte is absent.
fn b64_tail(present: bool, idx: usize) -> char {
    if present {
        B64_ALPHABET[idx] as char
    } else {
        '='
    }
}

/// Lenient base64 decode: skips padding/whitespace and any non-alphabet byte.
fn b64_decode(s: &str) -> Vec<u8> {
    let mut bits = 0u32;
    let mut nbits = 0u32;
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for &c in s.as_bytes() {
        let Some(v) = b64_val(c) else { continue };
        bits = (bits << 6) | v;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((bits >> nbits) as u8);
        }
    }
    out
}

/// A base64 alphabet byte → its 6-bit value, or `None` for non-alphabet bytes.
fn b64_val(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some(u32::from(c - b'A')),
        b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, args: Value) -> Value {
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": name, "arguments": args } });
        let mut s = Session::new();
        handle(&mut s, &req).expect("a response")
    }

    fn tool_text(resp: &Value) -> String {
        resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn tool_json(resp: &Value) -> Value {
        serde_json::from_str(&tool_text(resp)).unwrap()
    }

    /// Render a minimal document and return its base64 PDF (bundled fonts mean no
    /// caller faces are needed).
    fn render_pdf_b64() -> String {
        let resp = call(
            "render",
            json!({ "templateHtml": "<p>hello {{ who }}</p>",
            "data": { "who": "world" } }),
        );
        let v = tool_json(&resp);
        assert_eq!(resp["result"]["isError"], false);
        assert!(v["pageCount"].as_u64().unwrap() >= 1);
        v["base64"].as_str().unwrap().to_string()
    }

    #[test]
    fn initialize_advertises_tools() {
        let mut s = Session::new();
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" });
        let resp = handle(&mut s, &req).unwrap();
        assert_eq!(resp["result"]["serverInfo"]["name"], "turbo-html2pdf-mcp");
        let list = handle(
            &mut s,
            &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        )
        .unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["render", "append_pdf", "check_template"]);
    }

    #[test]
    fn notifications_get_no_reply() {
        let mut s = Session::new();
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle(&mut s, &note).is_none());
    }

    #[test]
    fn unknown_method_and_tool_are_reported() {
        let mut s = Session::new();
        let bad = handle(
            &mut s,
            &json!({ "jsonrpc": "2.0", "id": 9, "method": "frobnicate" }),
        )
        .unwrap();
        assert!(bad["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown method"));
        let resp = call("frobnicate", json!({}));
        assert_eq!(resp["result"]["isError"], true);
    }

    #[test]
    fn render_emits_a_pdf_with_base64_and_page_count() {
        let b64 = render_pdf_b64();
        let bytes = b64_decode(&b64);
        assert!(bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn render_honors_full_option_surface() {
        // Metadata + watermark + PDF/A + a named image all flow through without
        // error and still produce a valid PDF.
        let png = b64_encode(&one_px_png());
        let resp = call(
            "render",
            json!({
                "templateHtml": "<h1>{{ t }}</h1><img src=\"logo\">",
                "data": { "t": "Title" },
                "css": "@page { size: A4; margin: 1in; }",
                "images": [ { "name": "logo", "dataBase64": png } ],
                "meta": { "title": "Doc", "author": "me", "creationDate": 0 },
                "watermark": { "text": "DRAFT", "color": "#ff0000", "opacity": 0.2, "angle": 30 },
                "pdfA": true,
                "now": 0
            }),
        );
        assert_eq!(resp["result"]["isError"], false, "{}", tool_text(&resp));
        let v = tool_json(&resp);
        assert!(b64_decode(v["base64"].as_str().unwrap()).starts_with(b"%PDF-"));
    }

    #[test]
    fn append_pdf_glues_documents() {
        let b64 = render_pdf_b64();
        let resp = call(
            "append_pdf",
            json!({ "base": { "dataBase64": &b64 },
                    "extras": [ { "dataBase64": &b64 } ] }),
        );
        assert_eq!(resp["result"]["isError"], false, "{}", tool_text(&resp));
        let v = tool_json(&resp);
        assert!(b64_decode(v["base64"].as_str().unwrap()).starts_with(b"%PDF-"));
    }

    #[test]
    fn check_template_reports_header_footer() {
        let plain = call("check_template", json!({ "templateHtml": "<p>x</p>" }));
        let v = tool_json(&plain);
        assert_eq!(v["ok"], true);
        assert_eq!(v["hasHeader"], false);
        assert_eq!(v["hasFooter"], false);
    }

    #[test]
    fn errors_are_clean() {
        // Missing required template.
        assert_eq!(call("render", json!({}))["result"]["isError"], true);
        assert_eq!(call("check_template", json!({}))["result"]["isError"], true);
        // A blob with neither path nor base64.
        let bad = call("append_pdf", json!({ "base": {} }));
        assert_eq!(bad["result"]["isError"], true);
        // A syntax fault surfaces as a tool error.
        let broken = call("check_template", json!({ "templateHtml": "{{ unclosed" }));
        assert_eq!(broken["result"]["isError"], true);
        // Encryption without a userPassword is rejected (no empty-password PDFs).
        let no_pw = call(
            "render",
            json!({ "templateHtml": "<p>x</p>", "encryption": { "print": false } }),
        );
        assert_eq!(no_pw["result"]["isError"], true);
    }

    #[test]
    fn base64_round_trips_arbitrary_bytes() {
        let data: Vec<u8> = (0u8..=255).collect();
        assert_eq!(b64_decode(&b64_encode(&data)), data);
        assert_eq!(b64_decode(&b64_encode(b"M")), b"M"); // 1-byte (double pad)
        assert_eq!(b64_decode(&b64_encode(b"Ma")), b"Ma"); // 2-byte (single pad)
    }

    /// A 1×1 transparent PNG (smallest valid raster) for the image-embed test.
    fn one_px_png() -> Vec<u8> {
        b64_decode(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
        )
    }
}
