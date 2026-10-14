// Reproduce the license files shipped with bindings that embed the core fonts.
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const families = [
  "inter/LICENSE.txt",
  "roboto/OFL.txt",
  "liberation-serif/LICENSE",
  "pt-serif/OFL.txt",
  "fira-code/OFL.txt",
  "ibm-plex-mono/OFL.txt",
];
const fonts = join(root, "crates/turbo-html2pdf-core/assets/fonts");
const notices =
  "# Third-party notices\n\n" +
  "## Blink HTML font metric policy\n\n" +
  readFileSync(
    join(root, "crates/turbo-html2pdf-core/src/text/Blink-font-metrics.LICENSE"),
    "utf8",
  ) +
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
const license = readFileSync(join(root, "LICENSE"), "utf8");
for (const path of [
  "crates/turbo-pdf-wasm/LICENSE",
  "crates/turbo-pdf-py/LICENSE",
  "packages/react/LICENSE",
  "packages/template/LICENSE",
])
  files.set(path, license);
for (const [path, content] of files) {
  if (process.argv.includes("--check")) {
    if (readFileSync(join(root, path), "utf8") !== content)
      throw new Error(`Stale package notice: ${path}`);
  } else writeFileSync(join(root, path), content);
}
