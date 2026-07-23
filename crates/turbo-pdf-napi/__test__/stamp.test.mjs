// turbo-pdf N-API `stamp` end-to-end test (post-emit watermark overlay,
// stamp-existing-pdf feature).
//
// The Rust core's `stamp()` is already covered by
// crates/turbo-html2pdf-core/tests/stamp.rs. This suite proves the napi
// wrapper on top of it: option mapping (`StampOptions`/`StampWatermark`/
// `Encryption`/the optional `font` override), the decrypt/re-encrypt round
// trip, and that native faults surface as a typed `TurboPdfError` through
// `index.js`'s `guard()` (the same sentinel-decoding path `render`/`compile`
// use), not a bare Error.
//
// FINDING (documented, not asserted as a requirement): the watermark's Form
// XObject stream is written via `lopdf::Stream::new` with no `/Filter` set,
// and nothing in this codebase ever calls `lopdf::Document::compress()` before
// `save_to`. So a plaintext-in/plaintext-out `stamp()` call emits the overlay
// content stream UNCOMPRESSED — the fill/rotation operators and the
// `TurboWmStamp` XObject name all appear as literal bytes. The watermark's
// SHOWN TEXT itself, however, is NOT literal ASCII: the overlay embeds a real,
// subsetted font (`/Identity-H` CID encoding, `/FontFile2`/`/FontFile3`
// program), so the `Tj` operand is 2-byte big-endian subset-local glyph ids,
// not the word's UTF-8 bytes — hence the structural (not literal-text)
// assertions below. Verified by hand against this build; if a future change
// starts compressing per-object streams, the still-literal greps (XObject
// name, color/opacity/rotation operators) will need the qpdf/Rust-test
// fallback the brief anticipated.
//
// qpdf gating: mirrors e2e.test.mjs / conformance.test.mjs exactly — every
// `qpdf`-dependent assertion is wrapped in `qpdfAvailable()` and skipped (not
// failed) when qpdf isn't on PATH. Tests 4, 5, 7 and the `/Encrypt` byte checks
// in tests 2/3 do NOT depend on qpdf and always run.
//
// The suite is SKIPPED (not failed) when the native addon is not built.

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
const CUSTOM_FONT = join(
  repoRoot,
  "crates",
  "turbo-html2pdf-core",
  "assets",
  "fonts",
  "roboto",
  "Roboto-Regular.ttf",
);

function tryLoad() {
  try {
    return require(join(root, "index.js"));
  } catch {
    return null;
  }
}

const lib = tryLoad();

// Two paragraphs, the second forced onto its own page via `break-before:
// page`, so `render` alone (no appendPdf) yields a real multi-page plaintext
// fixture. No custom font is supplied: the `bundled-fonts` core feature is on
// by default, so the default sans face is enough to lay out plain ASCII text.
const CSS =
  "@page { size: 300px 200px; margin: 16px } p { font-size: 14px } .p2 { break-before: page; }";
const TEMPLATE =
  '<h1>Title</h1><p>Body text on page one.</p><p class="p2">Body text on page two.</p>';

const WATERMARK_TEXT = "CANCELLED";
const ORIGINAL_PASSWORD = "orig-stamp-pw";
const NEW_PASSWORD = "new-stamp-pw";

function qpdfAvailable() {
  try {
    execFileSync("qpdf", ["--version"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

function twoPagePlaintextPdf() {
  return lib.render(TEMPLATE, { css: CSS, now: 0 });
}

function twoPageEncryptedPdf(userPassword) {
  return lib.render(TEMPLATE, { css: CSS, now: 0, encryption: { userPassword } });
}

function writeTemp(name, pdf) {
  const path = join(tmpdir(), name);
  writeFileSync(path, pdf);
  return path;
}

/** true if `qpdf --check` (optionally with `--password=`) exits 0. */
function qpdfChecks(path, password) {
  const args = password ? [`--password=${password}`, "--check", path] : ["--check", path];
  try {
    execFileSync("qpdf", args, { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

/**
 * Assert `pdfBytes` embeds a real, subsetted font for the watermark overlay
 * (Identity-H CID encoding + an embedded FontFile2/FontFile3 program) rather
 * than depending on the reader's own base-14 Helvetica. The shown text is
 * therefore 2-byte glyph ids, not literal ASCII — see the file-level FINDING
 * comment — so this checks structure, not the literal watermark word.
 */
function assertEmbeddedWatermarkFont(pdfBytes) {
  const text = pdfBytes.toString("latin1");
  assert.ok(text.includes("/Identity-H"), "overlay font uses /Identity-H CID encoding");
  assert.ok(
    text.includes("/FontFile2") || text.includes("/FontFile3"),
    "overlay font attaches an embedded /FontFile2 or /FontFile3 program",
  );
  assert.ok(
    !text.includes("/BaseFont /Helvetica"),
    "overlay must not fall back to base-14 /BaseFont /Helvetica",
  );
}

function qpdfPageCount(path, password) {
  const args = password
    ? [`--password=${password}`, "--show-npages", path]
    : ["--show-npages", path];
  return parseInt(
    execFileSync("qpdf", args, { stdio: ["ignore", "pipe", "ignore"] })
      .toString("utf8")
      .trim(),
    10,
  );
}

test("addon is built (otherwise the suite is skipped)", (t) => {
  if (!lib) {
    t.skip("native addon not built — see e2e.test.mjs for HOW TO RUN");
    return;
  }
  assert.equal(typeof lib.stamp, "function");
  assert.equal(typeof lib.TurboPdfError, "function");
});

test("stamps a plaintext PDF: valid multi-page PDF, page count preserved", { skip: !lib }, () => {
  const base = twoPagePlaintextPdf();
  assert.equal(base.pageCount, 2, "fixture must start with two pages");

  const stamped = lib.stamp(base.pdf, { watermark: { text: WATERMARK_TEXT } });
  assert.ok(Buffer.isBuffer(stamped), "stamp returns a Buffer");
  assert.equal(stamped.subarray(0, 5).toString("latin1"), "%PDF-", "PDF magic");

  // Uncompressed overlay stream (see the file-level FINDING comment) — the
  // watermark's XObject name is a literal byte string; its shown text is not
  // (see assertEmbeddedWatermarkFont).
  assert.ok(stamped.includes(Buffer.from("TurboWmStamp")), "watermark XObject name present");
  assertEmbeddedWatermarkFont(stamped);

  if (qpdfAvailable()) {
    const path = writeTemp("turbo-pdf-napi-stamp-plain.pdf", stamped);
    execFileSync("qpdf", ["--check", path], { stdio: "ignore" }); // throws on any structural fault
    assert.equal(
      qpdfPageCount(path),
      base.pageCount,
      "qpdf page count matches the input's pageCount",
    );
  }
});

test("plaintext in -> encrypted out requires the password", { skip: !lib }, () => {
  const base = twoPagePlaintextPdf();
  const stamped = lib.stamp(base.pdf, {
    watermark: { text: WATERMARK_TEXT },
    encryption: { userPassword: "pw" },
  });

  assert.equal(stamped.subarray(0, 5).toString("latin1"), "%PDF-", "PDF magic");
  // The /Encrypt trailer reference itself is plaintext even though strings/
  // streams under it are AES-encrypted, so this grep is meaningful.
  assert.ok(stamped.includes(Buffer.from("/Encrypt")), "/Encrypt dictionary present");

  if (qpdfAvailable()) {
    const path = writeTemp("turbo-pdf-napi-stamp-encrypt.pdf", stamped);
    assert.equal(
      qpdfChecks(path),
      false,
      "qpdf --check with no password fails on encrypted output",
    );
    assert.equal(qpdfChecks(path, "pw"), true, "qpdf --check with the right password succeeds");
  }
});

test("encrypted in (correct password) -> encrypted out round-trips", { skip: !lib }, () => {
  const encIn = twoPageEncryptedPdf(ORIGINAL_PASSWORD);
  assert.ok(encIn.pdf.includes(Buffer.from("/Encrypt")), "fixture is actually encrypted");

  const stamped = lib.stamp(encIn.pdf, {
    watermark: { text: WATERMARK_TEXT },
    password: ORIGINAL_PASSWORD,
    encryption: { userPassword: NEW_PASSWORD },
  });

  assert.equal(stamped.subarray(0, 5).toString("latin1"), "%PDF-", "PDF magic");
  assert.ok(
    stamped.includes(Buffer.from("/Encrypt")),
    "/Encrypt dictionary present on the re-sealed output",
  );

  if (qpdfAvailable()) {
    const path = writeTemp("turbo-pdf-napi-stamp-roundtrip.pdf", stamped);
    assert.equal(qpdfChecks(path, NEW_PASSWORD), true, "opens under the NEW password");
    assert.equal(qpdfChecks(path, ORIGINAL_PASSWORD), false, "the OLD password no longer opens it");
  }
});

test("wrong password throws a typed TurboPdfError", { skip: !lib }, () => {
  const encIn = twoPageEncryptedPdf(ORIGINAL_PASSWORD);

  assert.throws(
    () => lib.stamp(encIn.pdf, { watermark: { text: WATERMARK_TEXT }, password: "WRONG" }),
    (err) => {
      assert.ok(err instanceof lib.TurboPdfError, "err is a TurboPdfError instance");
      assert.equal(err.name, "TurboPdfError");
      assert.equal(typeof err.code, "string");
      assert.notEqual(err.constructor, Error, "not a bare Error");
      return true;
    },
  );
});

test("malformed input throws a typed TurboPdfError", { skip: !lib }, () => {
  assert.throws(
    () => lib.stamp(Buffer.from("not a pdf"), { watermark: { text: WATERMARK_TEXT } }),
    (err) => {
      assert.ok(err instanceof lib.TurboPdfError, "err is a TurboPdfError instance");
      assert.equal(err.name, "TurboPdfError");
      assert.equal(typeof err.code, "string");
      return true;
    },
  );
});

test("watermark options (color/opacity/angle/fontSize) pass through", { skip: !lib }, () => {
  const base = twoPagePlaintextPdf();
  const stamped = lib.stamp(base.pdf, {
    watermark: { text: "X", color: "#ff0000", opacity: 0.3, angle: 30, fontSize: 48 },
  });

  assert.equal(stamped.subarray(0, 5).toString("latin1"), "%PDF-", "PDF magic");

  // Uncompressed overlay stream (see the file-level FINDING comment) lets us
  // check the actual option values landed as real PDF operands, not just that
  // rendering didn't throw.
  const text = stamped.toString("latin1");
  assert.ok(text.includes("1 0 0 rg"), "color #ff0000 -> DeviceRGB fill operand '1 0 0 rg'");
  assert.ok(
    text.includes("/ca 0.3") && text.includes("/CA 0.3"),
    "opacity 0.3 on the fade ExtGState",
  );
  assert.ok(
    text.includes("0.8660254 0.5 -0.5 0.8660254"),
    "angle 30deg -> rotation matrix built from cos(30)=0.8660254, sin(30)=0.5",
  );
  assert.ok(
    text.includes("/F1 36 Tf"),
    "fontSize 48 CSS px -> 36pt (px_to_pt: 72/96 ratio) Tf operand",
  );

  if (qpdfAvailable()) {
    const path = writeTemp("turbo-pdf-napi-stamp-options.pdf", stamped);
    execFileSync("qpdf", ["--check", path], { stdio: "ignore" });
  }
});

test("stamp is byte-deterministic for a plaintext watermark", { skip: !lib }, () => {
  const base = twoPagePlaintextPdf();
  const a = lib.stamp(base.pdf, { watermark: { text: WATERMARK_TEXT } });
  const b = lib.stamp(base.pdf, { watermark: { text: WATERMARK_TEXT } });
  assert.equal(Buffer.compare(a, b), 0, "same input + no encryption -> byte-identical output");
});

test("a caller-supplied font embeds and stamps cleanly", { skip: !lib }, () => {
  const base = twoPagePlaintextPdf();
  const font = readFileSync(CUSTOM_FONT);

  const stamped = lib.stamp(base.pdf, {
    watermark: { text: WATERMARK_TEXT, font },
  });

  assert.equal(stamped.subarray(0, 5).toString("latin1"), "%PDF-", "PDF magic");
  assert.ok(stamped.includes(Buffer.from("TurboWmStamp")), "watermark XObject name present");
  assertEmbeddedWatermarkFont(stamped);

  if (qpdfAvailable()) {
    const path = writeTemp("turbo-pdf-napi-stamp-custom-font.pdf", stamped);
    execFileSync("qpdf", ["--check", path], { stdio: "ignore" }); // throws on any structural fault
  }
});
