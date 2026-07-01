// turbo-pdf N-API images + watermark end-to-end test (binding-gap closure).
//
// Proves the two capabilities that the binding previously dropped:
//   1. NAMED IMAGES are embedded. `<img src="logo">` with
//      `images: [{ name: 'logo', data: png }]` must produce a real PDF image
//      XObject (the spike's bug was ZERO image objects). We assert the output
//      contains `/Subtype /Image` / `/XObject` markers, i.e. image objects > 0.
//   2. A DRAFT WATERMARK is emitted: rendering `watermark: { text: 'DRAFT' }`
//      must still produce a structurally valid PDF (qpdf clean when available),
//      with a watermark fade ExtGState (`/GSwm`) present.
//
// The PNG is generated in-process (no external files); fonts come from
// crates/turbo-html2pdf-core/assets/fonts. The suite is SKIPPED (not failed) when the
// native addon is not built.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import { deflateSync } from "node:zlib";

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

function qpdfAvailable() {
  try {
    execFileSync("qpdf", ["--version"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

// --- build a minimal, valid PNG entirely in-process -------------------------
// A solid-red NxN 8-bit RGB PNG: signature + IHDR + IDAT (zlib-deflated raw
// scanlines, each prefixed by a 0 filter byte) + IEND. No external files.
function crc32(buf) {
  let crc = 0xffffffff;
  for (let i = 0; i < buf.length; i++) {
    crc ^= buf[i];
    for (let k = 0; k < 8; k++) {
      crc = crc & 1 ? (crc >>> 1) ^ 0xedb88320 : crc >>> 1;
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const typeBuf = Buffer.from(type, "latin1");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])), 0);
  return Buffer.concat([len, typeBuf, data, crc]);
}

function makePng(size = 4) {
  const sig = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0); // width
  ihdr.writeUInt32BE(size, 4); // height
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // color type: RGB
  ihdr[10] = 0; // compression
  ihdr[11] = 0; // filter
  ihdr[12] = 0; // interlace
  // raw scanlines: each row is [filter=0, R,G,B per pixel]
  const row = Buffer.concat([Buffer.from([0]), Buffer.alloc(size * 3, 0).fill(0)]);
  for (let i = 1; i < row.length; i += 3) row[i] = 0xff; // red
  const raw = Buffer.concat(Array.from({ length: size }, () => row));
  const idat = deflateSync(raw);
  return Buffer.concat([
    sig,
    chunk("IHDR", ihdr),
    chunk("IDAT", idat),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const PNG = makePng(4);
const CSS = "@page { size: 200px 200px; margin: 10px } img { width: 64px; height: 64px }";

test("the generated PNG is a valid PNG (sniffs as PNG)", () => {
  assert.equal(PNG.subarray(0, 4).toString("latin1"), "\x89PNG");
});

test("named image embeds an image XObject (was 0 before)", { skip: !lib }, () => {
  const font = readFileSync(FONT);
  const result = lib.render('<img src="logo">', {
    css: CSS,
    fonts: [{ data: font, family: "Go" }],
    images: [{ name: "logo", data: PNG }],
    now: 0,
  });

  assert.equal(result.pdf.subarray(0, 5).toString("latin1"), "%PDF-", "PDF magic");
  const text = result.pdf.toString("latin1");
  const imageObjects = (text.match(/\/Subtype\s*\/Image/g) || []).length;
  assert.ok(imageObjects > 0, `expected at least one image XObject, got ${imageObjects}`);
  assert.ok(text.includes("/XObject"), "XObject resource present");

  if (qpdfAvailable()) {
    const path = join(tmpdir(), "turbo-pdf-napi-image.pdf");
    writeFileSync(path, result.pdf);
    execFileSync("qpdf", ["--check", path], { stdio: "ignore" });
  }
});

test("rendering WITHOUT images embeds zero image XObjects", { skip: !lib }, () => {
  const font = readFileSync(FONT);
  const result = lib.render('<img src="logo">', {
    css: CSS,
    fonts: [{ data: font, family: "Go" }],
    now: 0,
  });
  const imageObjects = (result.pdf.toString("latin1").match(/\/Subtype\s*\/Image/g) || []).length;
  assert.equal(imageObjects, 0, "no images supplied -> no XObject embedded");
});

test("DRAFT watermark is emitted (fade ExtGState present, qpdf clean)", { skip: !lib }, () => {
  const font = readFileSync(FONT);
  const result = lib.render("<p>Body text under the mark</p>", {
    css: "@page { size: 200px 200px; margin: 10px } p { font-size: 12px }",
    fonts: [{ data: font, family: "Go" }],
    watermark: { text: "DRAFT" },
    now: 0,
  });

  assert.equal(result.pdf.subarray(0, 5).toString("latin1"), "%PDF-", "PDF magic");
  const text = result.pdf.toString("latin1");
  // The watermark fade rides a dedicated `/GSwm` ExtGState (see core watermark.rs).
  assert.ok(text.includes("GSwm"), "watermark fade ExtGState (/GSwm) present");

  if (qpdfAvailable()) {
    const path = join(tmpdir(), "turbo-pdf-napi-watermark.pdf");
    writeFileSync(path, result.pdf);
    execFileSync("qpdf", ["--check", path], { stdio: "ignore" });
  }
});

test("image watermark resolves through the named images", { skip: !lib }, () => {
  const font = readFileSync(FONT);
  const result = lib.render("<p>x</p>", {
    css: "@page { size: 200px 200px; margin: 10px }",
    fonts: [{ data: font, family: "Go" }],
    images: [{ name: "mark", data: PNG }],
    watermark: { image: "mark", opacity: 0.2, tiled: true },
    now: 0,
  });
  const imageObjects = (result.pdf.toString("latin1").match(/\/Subtype\s*\/Image/g) || []).length;
  assert.ok(imageObjects > 0, "image watermark embeds its raster as an XObject");
});
