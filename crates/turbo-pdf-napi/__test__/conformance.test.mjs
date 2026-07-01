// turbo-pdf N-API conformance / encryption / append exposure test.
//
// Proves the merged-engine capabilities the binding now surfaces through
// `RenderOptions` (and the standalone `appendPdf`):
//   * pdfA: true   -> output carries `/OutputIntents` + the `GTS_PDFA` subtype.
//   * pdfUa: true  -> output carries `/StructTreeRoot` + `/MarkInfo`.
//   * cmyk: true   -> a DeviceCMYK fill operator (`k`/`K`) appears in content.
//   * encryption   -> an `/Encrypt` dictionary is written (and qpdf opens it
//                     with the password when qpdf is present).
//   * appendPdfs / appendPdf -> page count grows by the appended document.
//
// Fonts come from crates/turbo-html2pdf-core/assets/fonts. The suite is SKIPPED (not
// failed) when the native addon is not built.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const repoRoot = join(root, "..", "..");

function tryLoad() {
  try {
    return require(join(root, "index.js"));
  } catch {
    return null;
  }
}

const lib = tryLoad();
const FONT = join(repoRoot, "crates", "turbo-html2pdf-core", "assets", "fonts", "Go-Regular.ttf");
const CSS = "@page { size: 200px 200px; margin: 10px } p { font-size: 12px }";

function qpdfAvailable() {
  try {
    execFileSync("qpdf", ["--version"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

function renderWith(opts) {
  const font = readFileSync(FONT);
  return lib.render("<p>Conformance body text</p>", {
    css: CSS,
    fonts: [{ data: font, family: "Go" }],
    now: 0,
    ...opts,
  });
}

test("pdfA:true emits an OutputIntent with the GTS_PDFA subtype", { skip: !lib }, () => {
  const { pdf } = renderWith({ pdfA: true });
  assert.equal(pdf.subarray(0, 5).toString("latin1"), "%PDF-", "PDF magic");
  const text = pdf.toString("latin1");
  assert.ok(text.includes("/OutputIntents"), "/OutputIntents present");
  assert.ok(text.includes("GTS_PDFA"), "GTS_PDFA output-intent subtype present");
});

test("pdfA defaults off (no OutputIntents without the flag)", { skip: !lib }, () => {
  const { pdf } = renderWith({});
  assert.ok(!pdf.toString("latin1").includes("/OutputIntents"), "no OutputIntents by default");
});

test("pdfUa:true emits StructTreeRoot + MarkInfo", { skip: !lib }, () => {
  const { pdf } = renderWith({ pdfUa: true, lang: "en-US" });
  const text = pdf.toString("latin1");
  assert.ok(text.includes("/StructTreeRoot"), "/StructTreeRoot present");
  assert.ok(text.includes("/MarkInfo"), "/MarkInfo present");
  assert.ok(text.includes("/Lang"), "/Lang present");
});

test("cmyk:true emits a DeviceCMYK fill operator", { skip: !lib }, () => {
  const { pdf } = renderWith({ cmyk: true });
  const text = pdf.toString("latin1");
  // A CMYK fill uses the `k`/`K` operator (4 components) vs RGB's `rg`/`RG`.
  assert.ok(
    /\b\d?\.?\d+ \d?\.?\d+ \d?\.?\d+ \d?\.?\d+ k\b/.test(text),
    "DeviceCMYK k operator present",
  );
});

test("encryption:{userPassword} writes an /Encrypt dict (qpdf opens it)", { skip: !lib }, () => {
  const { pdf } = renderWith({ encryption: { userPassword: "open-sesame" } });
  assert.equal(pdf.subarray(0, 5).toString("latin1"), "%PDF-", "PDF magic");
  const text = pdf.toString("latin1");
  assert.ok(text.includes("/Encrypt"), "/Encrypt dictionary present");

  if (qpdfAvailable()) {
    const path = join(tmpdir(), "turbo-pdf-napi-encrypted.pdf");
    writeFileSync(path, pdf);
    // qpdf opens it with the right password and rejects a wrong one.
    execFileSync("qpdf", ["--password=open-sesame", "--check", path], { stdio: "ignore" });
    let rejected = false;
    try {
      execFileSync("qpdf", ["--password=wrong", "--check", path], { stdio: "ignore" });
    } catch {
      rejected = true;
    }
    assert.ok(rejected, "qpdf rejects the wrong password");
  }
});

test("encryption is omitted by default (no /Encrypt)", { skip: !lib }, () => {
  const { pdf } = renderWith({});
  assert.ok(!pdf.toString("latin1").includes("/Encrypt"), "no /Encrypt by default");
});

test("appendPdfs grows the page count by the appended document", { skip: !lib }, () => {
  const base = renderWith({});
  assert.equal(base.pageCount, 1, "base is a single page");

  // Render a second single-page doc and append it via the render option.
  const extra = renderWith({}).pdf;
  const merged = renderWith({ appendPdfs: [extra] });
  // The merged page tree must carry 2 pages: count /Type /Page (not /Pages).
  const pageNodes = (merged.pdf.toString("latin1").match(/\/Type\s*\/Page\b/g) || []).length;
  assert.ok(pageNodes >= 2, `expected >= 2 page nodes after append, got ${pageNodes}`);

  if (qpdfAvailable()) {
    const path = join(tmpdir(), "turbo-pdf-napi-appended.pdf");
    writeFileSync(path, merged.pdf);
    const out = execFileSync("qpdf", ["--show-npages", path]).toString().trim();
    assert.equal(out, "2", "qpdf reports 2 pages after append");
  }
});

test("standalone appendPdf merges two emitted PDFs", { skip: !lib }, () => {
  const a = renderWith({}).pdf;
  const b = renderWith({}).pdf;
  const merged = lib.appendPdf(a, [b]);
  assert.equal(merged.subarray(0, 5).toString("latin1"), "%PDF-", "merged PDF magic");
  const pageNodes = (merged.toString("latin1").match(/\/Type\s*\/Page\b/g) || []).length;
  assert.ok(pageNodes >= 2, `expected >= 2 page nodes, got ${pageNodes}`);
});

test("appendPdf throws TurboPdfError on a malformed extra", { skip: !lib }, () => {
  const a = renderWith({}).pdf;
  assert.throws(
    () => lib.appendPdf(a, [Buffer.from("not a pdf")]),
    (err) => err && err.name === "TurboPdfError",
  );
});
