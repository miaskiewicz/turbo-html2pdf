// Reproduce the license files shipped with bindings that embed the core fonts, and the
// npm package's pack-time font.
//
//   node scripts/package-notices.mjs           write every file below
//   node scripts/package-notices.mjs --check   fail if a tracked file is stale (a generated
//                                              file is checked only when present)
//   node scripts/package-notices.mjs --pack    check the tracked files, then write the
//                                              generated ones (release.yml runs this before
//                                              `npm publish`)
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { layoutCrateDir, layoutFontsDir } from "./layout-crate-dir.mjs";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
// The layout half (its Blink-derived policy notices and the embedded fonts with their
// licenses) lives in the turbo-dom-layout crate; it is located through cargo so a
// path, git or registry dependency all resolve.
const layout = layoutCrateDir();
const families = [
  "inter/LICENSE.txt",
  "roboto/OFL.txt",
  "liberation-serif/LICENSE",
  "pt-serif/OFL.txt",
  "fira-code/OFL.txt",
  "ibm-plex-mono/OFL.txt",
];
const fonts = layoutFontsDir();
const notices =
  "# Third-party notices\n\n" +
  "## Blink HTML font metric policy\n\n" +
  readFileSync(join(layout, "src/text/Blink-font-metrics.LICENSE"), "utf8") +
  "\n## Blink SVG viewport layout-unit policy\n\n" +
  readFileSync(join(layout, "src/layout/Blink-layout-unit.LICENSE"), "utf8") +
  "\n## Embedded fonts\n\n" +
  readFileSync(join(fonts, "NOTICE.md"), "utf8") +
  "\n" +
  families.map((path) => `\n## ${path}\n\n${readFileSync(join(fonts, path), "utf8")}\n`).join("");
const files = new Map([
  ["THIRD_PARTY_NOTICES.md", notices],
  ["crates/turbo-pdf-napi/THIRD_PARTY_NOTICES.md", notices],
  ["crates/turbo-pdf-wasm/THIRD_PARTY_NOTICES.md", notices],
  ["crates/turbo-pdf-py/python/turbo_html2pdf/THIRD_PARTY_NOTICES.md", notices],
  [
    "crates/turbo-pdf-napi/assets/fonts/Go-Font-LICENSE",
    readFileSync(join(fonts, "Go-Font-LICENSE"), "utf8"),
  ],
]);
// Generated at pack time and gitignored (no vendored font copy in this repo): the npm
// package ships the Go face (`files: assets/fonts/*.ttf`) from turbo-dom-layout's font.
const generated = new Map([
  [
    "crates/turbo-pdf-napi/assets/fonts/Go-Regular.ttf",
    readFileSync(join(fonts, "Go-Regular.ttf")),
  ],
]);
const license = readFileSync(join(root, "LICENSE"), "utf8");
for (const path of [
  "crates/turbo-pdf-wasm/LICENSE",
  "crates/turbo-pdf-py/LICENSE",
  "packages/react/LICENSE",
  "packages/template/LICENSE",
])
  files.set(path, license);
const check = process.argv.includes("--check");
const pack = process.argv.includes("--pack");
function verify(path, content) {
  const current = readFileSync(join(root, path));
  if (!current.equals(Buffer.from(content))) throw new Error(`Stale package notice: ${path}`);
}
for (const [path, content] of files) {
  if (check || pack) verify(path, content);
  else writeFileSync(join(root, path), content);
}
for (const [path, content] of generated) {
  if (!check) writeFileSync(join(root, path), content);
  else if (existsSync(join(root, path))) verify(path, content);
}
