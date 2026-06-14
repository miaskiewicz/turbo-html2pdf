// turbo-pdf N-API end-to-end test (Phase 10).
//
// Compiles a real template + data, renders a PDF through the native addon, and
// asserts the page count and that the bytes are a real PDF. When `qpdf` is on
// PATH it additionally asserts `qpdf --check` is clean.
//
// HOW TO RUN
//   1. Build the addon (one of):
//        napi build --release            # if @napi-rs/cli is installed
//        cargo build -p turbo-pdf-napi --release
//      then, for the plain `cargo build` path, drop the cdylib next to index.js:
//        cp ../../target/release/libturbo_pdf_napi.dylib ./turbo-pdf-napi.node   (macOS)
//        cp ../../target/release/libturbo_pdf_napi.so    ./turbo-pdf-napi.node   (linux)
//      (index.js also falls back to target/{release,debug} automatically.)
//   2. node --test crates/turbo-pdf-napi/__test__/e2e.test.mjs
//
// The test is skipped (not failed) when the native addon is not built, so a
// missing Rust toolchain or un-built addon never red-bars an unrelated CI lane.

import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..')

function tryLoad() {
  try {
    return require(join(root, 'index.js'))
  } catch {
    return null
  }
}

const lib = tryLoad()
const FONT = join(root, 'assets', 'fonts', 'Go-Regular.ttf')

const CSS = '@page { size: 320px 240px; margin: 24px } p { font-size: 13px }'
const TEMPLATE =
  '<h1>{{ title }}</h1>' +
  '{% for row in rows %}<p>{{ row.label }}: {{ row.value }}</p>{% endfor %}'
const DATA = {
  title: 'Quarterly Report',
  rows: [
    { label: 'Revenue', value: '120000' },
    { label: 'Costs', value: '80000' },
    { label: 'Profit', value: '40000' },
  ],
}

function qpdfAvailable() {
  try {
    execFileSync('qpdf', ['--version'], { stdio: 'ignore' })
    return true
  } catch {
    return false
  }
}

test('addon is built (otherwise the suite is skipped)', (t) => {
  if (!lib) {
    t.skip('native addon not built — see HOW TO RUN at the top of this file')
    return
  }
  assert.equal(typeof lib.compile, 'function')
  assert.equal(typeof lib.render, 'function')
})

test('compile + render produces a valid multi-line PDF', { skip: !lib }, () => {
  const font = readFileSync(FONT)
  const program = lib.compile(TEMPLATE)
  const result = program.render({ data: DATA, css: CSS, fonts: [font], now: 0 })

  assert.ok(Buffer.isBuffer(result.pdf), 'pdf is a Buffer')
  assert.equal(result.pdf.subarray(0, 5).toString('latin1'), '%PDF-', 'PDF magic')
  assert.ok(result.pageCount >= 1, 'at least one page')
  assert.ok(Array.isArray(result.diagnostics), 'diagnostics is an array')

  if (qpdfAvailable()) {
    const path = join(tmpdir(), 'turbo-pdf-napi-e2e.pdf')
    writeFileSync(path, result.pdf)
    // Throws on a non-zero exit (i.e. qpdf found structural problems).
    execFileSync('qpdf', ['--check', path], { stdio: 'ignore' })
  }
})

test('one-shot render matches and is byte-deterministic', { skip: !lib }, () => {
  const font = readFileSync(FONT)
  const a = lib.render(TEMPLATE, { data: DATA, css: CSS, fonts: [font], now: 0 })
  const b = lib.render(TEMPLATE, { data: DATA, css: CSS, fonts: [font], now: 0 })
  assert.ok(a.pdf.equals(b.pdf), 'identical inputs -> identical bytes')
  assert.equal(a.pdf.subarray(0, 5).toString('latin1'), '%PDF-')
})

test('fatal faults throw a typed TurboPdfError with code + span', { skip: !lib }, () => {
  assert.throws(
    () => lib.compile('{{ broken '),
    (err) => {
      assert.equal(err.name, 'TurboPdfError')
      assert.equal(err.code, 'TemplateSyntax')
      assert.equal(typeof err.span.line, 'number')
      return true
    },
  )
})

test('lints are returned, not thrown', { skip: !lib }, () => {
  const font = readFileSync(FONT)
  // Glyphs absent from every supplied face emit a NotdefGlyph lint; the render
  // must still succeed and return the diagnostic rather than throw.
  const result = lib.render('<p>你好世界</p>', {
    data: {},
    css: CSS,
    fonts: [font],
    now: 0,
  })
  assert.ok(result.pageCount >= 1)
  assert.ok(result.diagnostics.some((d) => d.code === 'NotdefGlyph'))
})
