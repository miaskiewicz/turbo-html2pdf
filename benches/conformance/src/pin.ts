// Font pinning + fixture prep.
//
// The whole harness compares laid-out geometry, and text-metric-driven boxes
// (line-box heights, wrap points, intrinsic widths) only match if BOTH sides
// shape with the SAME font. The engine renders with its bundled default face —
// `font-family: sans-serif` maps to Inter (see turbo-html2pdf-core's bundled set)
// — so we pin Chromium to the exact same Inter OTF via an `@font-face` whose
// `src` is a base64 `data:` URI of the engine's own font file, and force every
// element onto family `Inter` on both sides.
//
// The engine resolves family `Inter` straight to its bundled face (its font
// registry registers the bundled faces under their real family name), and it
// ignores the `@font-face` data URL (its registry is fixed, not fed from author
// CSS) — so the same injected `<style>` pins Chromium and is a harmless no-op for
// the engine. The style is injected at the start of `<body>` (head and body styles
// both apply since #29).

import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { layoutFontsDir } from "../../../scripts/layout-crate-dir.mjs";

// The engine's bundled Inter faces (turbo-dom-layout's assets/fonts, via cargo).
const FONT_DIR = join(layoutFontsDir(), "inter");

function dataUri(file: string): string {
  const bytes = readFileSync(resolve(FONT_DIR, file));
  return `data:font/otf;base64,${bytes.toString("base64")}`;
}

/** The pinned-font + reset `<style>`, injected into every fixture on both sides.
 *  Regular (400) and Bold (700) Inter faces cover the fixtures' default and
 *  heading/`<b>` weights so weight selection matches the engine's bundled set. */
export function pinStyle(): string {
  const regular = dataUri("Inter-Regular.otf");
  const bold = dataUri("Inter-Bold.otf");
  return [
    "<style>",
    `@font-face{font-family:'Inter';font-weight:400;font-style:normal;src:url(${regular}) format('opentype')}`,
    `@font-face{font-family:'Inter';font-weight:700;font-style:normal;src:url(${bold}) format('opentype')}`,
    // Zero the UA margins the two engines disagree on at the document edges, so
    // the page origin lines up. Per-element UA defaults (headings, lists) are
    // left intact — those are deliberately under test.
    "html,body{margin:0;padding:0}",
    // Pin the family everywhere; kill smoothing so nothing nudges metrics.
    "*{font-family:'Inter';-webkit-font-smoothing:none}",
    "</style>",
  ].join("");
}

/** Insert `style` at the very start of the fixture's `<body>` (both engines see
 *  the identical document). Fixtures always carry a `<body>`. */
export function injectPin(html: string, style: string): string {
  const m = html.match(/<body[^>]*>/i);
  if (!m) return style + html;
  const at = (m.index ?? 0) + m[0].length;
  return html.slice(0, at) + style + html.slice(at);
}
