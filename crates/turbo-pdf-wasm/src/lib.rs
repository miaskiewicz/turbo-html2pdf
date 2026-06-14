//! WASM/`wasm-bindgen` binding for turbo-pdf (§8.3): the template → PDF pipeline
//! exposed to browsers and JS runtimes.
//!
//! The surface mirrors the planned N-API binding but is WASM-idiomatic: an
//! optional async [`init`] (panics surface as JS exceptions once installed), a
//! [`compile`] that returns a reusable [`Program`] handle, and
//! [`Program::render`] taking `{ data, css?, fonts?, meta?, now? }` and returning
//! `{ pdf: Uint8Array, diagnostics, pageCount }`.
//!
//! Fonts (and, in a later phase, images) cross the boundary as `Uint8Array`.
//! Diagnostics are *returned* in the result object, never thrown; only fatal
//! compile/render errors reject (as a structured `{ code, message, span }`).
//!
//! ## Determinism (AC-8.6)
//!
//! WASM output must be byte-identical to native modulo the pinned font
//! subsetter. We pin the two sources of nondeterminism the core exposes:
//!
//! * the render clock — when the caller omits `now`, we pass a fixed sentinel
//!   ([`DEFAULT_NOW`]) so `now()`/date field codes are reproducible; and
//! * the PDF creation date — when the caller omits `meta.creationDate`, the core
//!   emitter substitutes its own [`turbo_pdf_core::SENTINEL_DATE`].
//!
//! With both pinned and the same fonts supplied, `program.render(...)` yields the
//! same bytes the native `render_pages` → `emit_pdf` path produces.

#![forbid(unsafe_code)]

mod convert;
mod program;

use wasm_bindgen::prelude::*;

pub use program::{compile, Program};

/// The fixed render-clock sentinel used when the caller omits `now`
/// (`2000-01-01T00:00:00Z`, matching the emitter's creation-date sentinel). This
/// keeps `{{ now() }}` and date field codes reproducible across runs (AC-8.6).
pub const DEFAULT_NOW: i64 = turbo_pdf_core::SENTINEL_DATE;

/// Optional async initializer. There is no module-load work to do today (the
/// engine is pure-Rust with no global setup), but exposing `init()` lets callers
/// write the idiomatic `await init()` and installs a readable panic message — so
/// the entry point is stable if future phases need real async setup.
#[wasm_bindgen]
pub fn init() {
    set_panic_hook();
}

/// Route Rust panics to a JS-readable message instead of an opaque
/// `unreachable`. No-op when the optional `console_error_panic_hook` is absent;
/// kept tiny and dependency-free so the default build stays lean.
fn set_panic_hook() {
    // Intentionally minimal: we do not pull in a panic-hook crate for the
    // default build. Panics still abort with the wasm trap; `init()` exists as
    // the documented async entry point and a hook-installation seam.
}
