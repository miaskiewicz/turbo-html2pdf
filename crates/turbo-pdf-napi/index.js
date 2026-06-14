/* turbo-pdf N-API binding — JS entry point (Phase 10).
 *
 * Loads the platform-native addon and wraps the native `compile` / `render`
 * surface so that fatal faults surface as a typed `TurboPdfError` (carrying
 * `.code` and `.span`) instead of a bare Error. Non-fatal lints are returned in
 * `result.diagnostics` by the native layer and are passed through untouched.
 */

'use strict'

const { existsSync } = require('node:fs')
const { join } = require('node:path')

// --- locate the native addon -------------------------------------------------
// In a published package the prebuilt `.node` sits next to this file. In local
// dev (no `napi build`) the cdylib lands in target/{release,debug}; we accept a
// copied/symlinked `turbo-pdf-napi.node` here, or fall back to the cargo output.
const CANDIDATES = [
  join(__dirname, 'turbo-pdf-napi.node'),
  join(__dirname, '..', '..', 'target', 'release', addonName()),
  join(__dirname, '..', '..', 'target', 'debug', addonName()),
]

function addonName() {
  // napi-rs emits a platform cdylib; Node loads it regardless of extension.
  if (process.platform === 'darwin') return 'libturbo_pdf_napi.dylib'
  if (process.platform === 'win32') return 'turbo_pdf_napi.dll'
  return 'libturbo_pdf_napi.so'
}

function loadNative() {
  for (const p of CANDIDATES) {
    if (existsSync(p)) return require(p)
  }
  throw new Error(
    'turbo-pdf-napi: native addon not found. Run `napi build --release` (or ' +
      '`cargo build -p turbo-pdf-napi --release`) first. Looked in:\n  ' +
      CANDIDATES.join('\n  '),
  )
}

const native = loadNative()

// --- typed error -------------------------------------------------------------
const SENTINEL = 'TURBO_PDF_ERR:'

/** A fatal compile/render fault, carrying a machine-readable code and a source
 *  span ({line, col, byteOffset}). Thrown by `compile` / `render`. */
class TurboPdfError extends Error {
  constructor(payload) {
    super(payload.message)
    this.name = 'TurboPdfError'
    this.code = payload.code
    this.span = payload.span
  }
}

/** If `err` is a sentinel-encoded native fault, rethrow it as a TurboPdfError;
 *  otherwise rethrow it unchanged. */
function rethrow(err) {
  const msg = err && typeof err.message === 'string' ? err.message : ''
  const at = msg.indexOf(SENTINEL)
  if (at === -1) throw err
  const json = msg.slice(at + SENTINEL.length)
  let payload
  try {
    payload = JSON.parse(json)
  } catch {
    throw err
  }
  throw new TurboPdfError(payload)
}

function guard(fn) {
  return (...args) => {
    try {
      return fn(...args)
    } catch (err) {
      rethrow(err)
    }
  }
}

// --- public surface ----------------------------------------------------------
const compile = guard((templateHtml, opts) => {
  const program = native.compile(templateHtml, opts)
  // Wrap `program.render` so render-time faults are typed too.
  const render = program.render.bind(program)
  program.render = guard(render)
  return program
})

const render = guard((templateHtml, opts) => native.render(templateHtml, opts))

module.exports = { compile, render, TurboPdfError }
module.exports.default = module.exports
