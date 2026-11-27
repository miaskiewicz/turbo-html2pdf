"use strict";

const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");

const [arch, flavor] = process.argv.slice(2);
assert.equal(process.platform, "linux");
assert.equal(process.arch, arch);
assert.equal(process.report.getReport().header.glibcVersionRuntime, "2.34");
assert.match(readFileSync("/etc/os-release", "utf8"), /^VERSION_ID="?2023"?$/m);

const dir = join("/artifacts", flavor);
const binding = require(join(dir, "index.js"));
assert.ok(require.cache[join(dir, `turbo-pdf-napi.linux-${arch}-gnu.node`)]);
assert.equal(binding.version(), readFileSync("/artifacts/VERSION", "utf8").trim());

const svg = Buffer.from(
  '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><rect width="40" height="40" fill="red"/></svg>',
);
const withSvg = flavor === "svg";
const image = withSvg ? '<img src="smoke.svg" width="40" height="40">' : "";
const result = binding.render(`<p>Lambda PDF smoke ${flavor}</p>${image}`, {
  css: "@page { size: 300px 200px; margin: 12px; } body { font-family: Go; }",
  fonts: [{ family: "Go", data: readFileSync("/artifacts/Go-Regular.ttf") }],
  images: withSvg ? [{ name: "smoke.svg", data: svg }] : [],
  now: 0,
});

assert.equal(result.pageCount, 1);
assert.ok(Buffer.isBuffer(result.pdf));
assert.match(result.pdf.subarray(0, 5).toString(), /^%PDF-/);
process.stdout.write(result.pdf);
