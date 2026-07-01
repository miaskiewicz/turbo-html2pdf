// turbo-pdf WASM conformance / encryption / append exposure test (Node target).
//
// Mirrors the N-API conformance suite against the WASM build: it proves the
// per-render toggles the browser binding now surfaces are actually wired through
// to the emitter.
//
// HOW TO BUILD THE TESTED ARTIFACT (from crates/turbo-pdf-wasm):
//   RUSTFLAGS='--cfg getrandom_backend="wasm_js"' \
//     wasm-pack build --target nodejs --out-dir pkg-node --dev
// The `getrandom_backend` cfg + the wasm-target getrandom features in Cargo.toml
// give the `encrypt`/`append` features a Web-Crypto randomness backend.
//
// Then run:  node --test __test__/conformance.test.mjs
// The suite is SKIPPED (not failed) when ./pkg-node is not built.

import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const repoRoot = join(root, "..", "..");
const PKG = join(root, "pkg-node", "turbo_pdf_wasm.js");

const wasm = existsSync(PKG) ? require(PKG) : null;
const FONT = readFileSync(
  join(repoRoot, "crates", "turbo-html2pdf-core", "assets", "fonts", "Go-Regular.ttf"),
);
const CSS = "@page { size: 200px 200px; margin: 10px } p { font-size: 12px }";

function render(extra) {
  const program = wasm.compile("<p>Conformance body text</p>", undefined);
  const result = program.render({
    css: CSS,
    fonts: [{ data: new Uint8Array(FONT), family: "Go" }],
    now: 0,
    ...extra,
  });
  return result;
}

function asText(u8) {
  return Buffer.from(u8).toString("latin1");
}

test("pdfA:true emits OutputIntents + GTS_PDFA", { skip: !wasm }, () => {
  const text = asText(render({ pdfA: true }).pdf);
  assert.ok(text.startsWith("%PDF-"), "PDF magic");
  assert.ok(text.includes("/OutputIntents"), "/OutputIntents present");
  assert.ok(text.includes("GTS_PDFA"), "GTS_PDFA subtype present");
});

test("pdfUa:true emits StructTreeRoot + MarkInfo + Lang", { skip: !wasm }, () => {
  const text = asText(render({ pdfUa: true, lang: "en-US" }).pdf);
  assert.ok(text.includes("/StructTreeRoot"), "/StructTreeRoot present");
  assert.ok(text.includes("/MarkInfo"), "/MarkInfo present");
  assert.ok(text.includes("/Lang"), "/Lang present");
});

test("cmyk:true emits a DeviceCMYK k operator", { skip: !wasm }, () => {
  const text = asText(render({ cmyk: true }).pdf);
  assert.ok(/\b\d?\.?\d+ \d?\.?\d+ \d?\.?\d+ \d?\.?\d+ k\b/.test(text), "DeviceCMYK k op present");
});

test("encryption:{userPassword} writes an /Encrypt dict", { skip: !wasm }, () => {
  const text = asText(render({ encryption: { userPassword: "open-sesame" } }).pdf);
  assert.ok(text.includes("/Encrypt"), "/Encrypt dictionary present");
});

test("no conformance flags -> plain RGB, no Encrypt/OutputIntents", { skip: !wasm }, () => {
  const text = asText(render({}).pdf);
  assert.ok(!text.includes("/Encrypt"), "no /Encrypt by default");
  assert.ok(!text.includes("/OutputIntents"), "no /OutputIntents by default");
});

test("appendPdfs grows the page count", { skip: !wasm }, () => {
  const extra = Buffer.from(render({}).pdf);
  const merged = render({ appendPdfs: [new Uint8Array(extra)] });
  const pages = (asText(merged.pdf).match(/\/Type\s*\/Page\b/g) || []).length;
  assert.ok(pages >= 2, `expected >= 2 page nodes, got ${pages}`);
});

test("standalone appendPdf merges two emitted PDFs", { skip: !wasm }, () => {
  const a = render({}).pdf;
  const b = render({}).pdf;
  const merged = wasm.appendPdf(a, [b]);
  const text = asText(merged);
  assert.ok(text.startsWith("%PDF-"), "merged PDF magic");
  const pages = (text.match(/\/Type\s*\/Page\b/g) || []).length;
  assert.ok(pages >= 2, `expected >= 2 page nodes, got ${pages}`);
});

test("appendPdf rejects malformed input with a structured error", { skip: !wasm }, () => {
  const a = render({}).pdf;
  assert.throws(
    () => wasm.appendPdf(a, [new Uint8Array(Buffer.from("not a pdf"))]),
    (err) => err && err.code === "Append",
  );
});
