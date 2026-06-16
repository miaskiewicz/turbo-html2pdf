/* turbo-pdf N-API binding — JS entry point (Phase 10).
 *
 * Loads the platform-native addon and wraps the native `compile` / `render`
 * surface so that fatal faults surface as a typed `TurboPdfError` (carrying
 * `.code` and `.span`) instead of a bare Error. Non-fatal lints are returned in
 * `result.diagnostics` by the native layer and are passed through untouched.
 *
 * Do NOT let `napi build` overwrite this file — its plain loader has no
 * `TurboPdfError` wrapper. The build scripts redirect codegen to
 * `index.generated.js` (git-ignored) via `--js`; pass that flag if running
 * `napi build` by hand.
 */

"use strict";

const { existsSync } = require("node:fs");
const { join } = require("node:path");

// --- locate the native addon -------------------------------------------------
// In a published package, EVERY platform's prebuilt `.node` sits next to this
// file, each named `turbo-pdf-napi.<platform>.node` (the suffix that
// `napi build --platform` emits — e.g. `turbo-pdf-napi.darwin-arm64.node`,
// `turbo-pdf-napi.linux-x64-gnu.node`). We pick the one matching this host, the
// same way turbo-dom's bundled package does. Local-dev fallbacks: an unsuffixed
// `turbo-pdf-napi.node` (single-target `napi build`), or the raw cargo cdylib in
// target/{release,debug} (plain `cargo build -p turbo-pdf-napi`).
const CANDIDATES = [
  join(__dirname, `turbo-pdf-napi.${napiPlatform()}.node`),
  join(__dirname, "turbo-pdf-napi.node"),
  join(__dirname, "..", "..", "target", "release", addonName()),
  join(__dirname, "..", "..", "target", "debug", addonName()),
];

function isMusl() {
  // glibc builds report a glibc runtime version; musl builds do not.
  if (!process.report || typeof process.report.getReport !== "function") {
    try {
      const ldd = require("node:child_process").execSync("which ldd").toString().trim();
      return require("node:fs").readFileSync(ldd, "utf8").includes("musl");
    } catch {
      return true;
    }
  }
  const { glibcVersionRuntime } = process.report.getReport().header;
  return !glibcVersionRuntime;
}

// The platform suffix used by @napi-rs/cli for the bundled `.node` filenames.
// Mirrors the naming in NAPI-RS's generated loader, for the matrix this repo
// ships: linux x64 gnu/musl, linux arm64, darwin arm64, win32 x64 msvc.
function napiPlatform() {
  const { platform, arch } = process;
  if (platform === "darwin") return `darwin-${arch}`; // arm64 -> darwin-arm64
  if (platform === "win32") return `win32-${arch}-msvc`; // x64 -> win32-x64-msvc
  if (platform === "linux") {
    const abi = isMusl() ? "musl" : "gnu";
    return `linux-${arch}-${abi}`; // x64 -> linux-x64-gnu | linux-x64-musl; arm64 -> linux-arm64-gnu
  }
  return `${platform}-${arch}`;
}

function addonName() {
  // napi-rs emits a platform cdylib; Node loads it regardless of extension.
  if (process.platform === "darwin") return "libturbo_pdf_napi.dylib";
  if (process.platform === "win32") return "turbo_pdf_napi.dll";
  return "libturbo_pdf_napi.so";
}

function loadNative() {
  for (const p of CANDIDATES) {
    if (existsSync(p)) return require(p);
  }
  throw new Error(
    "turbo-pdf-napi: native addon not found. Run `napi build --release` (or " +
      "`cargo build -p turbo-pdf-napi --release`) first. Looked in:\n  " +
      CANDIDATES.join("\n  "),
  );
}

const native = loadNative();

// --- typed error -------------------------------------------------------------
const SENTINEL = "TURBO_PDF_ERR:";

/** A fatal compile/render fault, carrying a machine-readable code and a source
 *  span ({line, col, byteOffset}). Thrown by `compile` / `render`. */
class TurboPdfError extends Error {
  constructor(payload) {
    super(payload.message);
    this.name = "TurboPdfError";
    this.code = payload.code;
    this.span = payload.span;
  }
}

/** If `err` is a sentinel-encoded native fault, rethrow it as a TurboPdfError;
 *  otherwise rethrow it unchanged. */
function rethrow(err) {
  const msg = err && typeof err.message === "string" ? err.message : "";
  const at = msg.indexOf(SENTINEL);
  if (at === -1) throw err;
  const json = msg.slice(at + SENTINEL.length);
  let payload;
  try {
    payload = JSON.parse(json);
  } catch {
    throw err;
  }
  throw new TurboPdfError(payload);
}

function guard(fn) {
  return (...args) => {
    try {
      return fn(...args);
    } catch (err) {
      rethrow(err);
    }
  };
}

// --- public surface ----------------------------------------------------------
const compile = guard((templateHtml, opts) => {
  const program = native.compile(templateHtml, opts);
  // Wrap `program.render` so render-time faults are typed too.
  const render = program.render.bind(program);
  program.render = guard(render);
  return program;
});

const render = guard((templateHtml, opts, fonts) => native.render(templateHtml, opts, fonts));

// Glue foreign PDFs after `base`; parse faults surface as a typed TurboPdfError.
const appendPdf = guard((base, extras) => native.appendPdf(base, extras));

module.exports = { compile, render, appendPdf, Fonts: native.Fonts, TurboPdfError };
module.exports.default = module.exports;
