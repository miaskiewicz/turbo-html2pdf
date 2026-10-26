// Chrome-156 control for paginated box geometry (miaskiewicz/turbo-html2pdf#31): margins at
// page starts (CSS Fragmentation §5.2 — a block's top margin is kept at the start of its first
// fragment, truncated only after a break) and `<html>`/`<body>` box decorations painted on
// every page fragment (`box-decoration-break: slice`).
//
// Each case is an HTML document with `@page { size: 600px 400px; margin: 40px }` whose boxes
// are solid-colour, text-free, unbreakable blocks. Chrome for Testing 156 prints it with `page.pdf()`
// (`preferCSSPageSize`, `printBackground`); poppler's `pdftoppm -r 96` rasterizes every page
// at 1 CSS px per device px, and for every colour of the case the control records, per page,
// the bounding box `[x0, y0, x1, y1]` (inclusive device px) of the pixels of exactly that
// colour, plus `top`/`bottom`: whether the box's first/last row is that colour across >= 90%
// of its width (a painted horizontal border edge or a filled block). Asserted against
// `render_pages` fragments by crates/turbo-html2pdf-core/tests/pagination_chrome.rs.
//
// Usage: node scripts/pagination-chrome-control.mjs [--chrome PATH] [--out PATH]
//   --chrome  defaults to the Playwright-cached Chrome for Testing 156;
//   --out     defaults to crates/turbo-html2pdf-core/tests/fixtures/pagination_chrome156.json.
// Needs playwright-core (PLAYWRIGHT_CORE, default ../turbo-surf/node_modules/playwright-core)
// and poppler's `pdftoppm` on PATH.
import { execFileSync } from "node:child_process";
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "..");
const { values } = parseArgs({ options: { chrome: { type: "string" }, out: { type: "string" } } });
const chromePath =
  values.chrome ||
  join(
    homedir(),
    "Library/Caches/ms-playwright/chromium-1248/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
  );
const out = resolve(
  values.out || join(repo, "crates/turbo-html2pdf-core/tests/fixtures/pagination_chrome156.json"),
);
const playwrightCore =
  process.env.PLAYWRIGHT_CORE || resolve(repo, "../turbo-surf/node_modules/playwright-core");
const require = createRequire(import.meta.url);
const { chromium } = require(playwrightCore);
const playwrightVersion = require(join(playwrightCore, "package.json")).version;

const PAGE = "@page{size:600px 400px;margin:40px}";
const DOC = (css, body) =>
  `<!DOCTYPE html><html><head><style>${PAGE}${css}</style></head><body>${body}</body></html>`;
// `break-inside: avoid` keeps each block whole (Chrome would otherwise slice a tall empty
// block across pages; the engine keeps childless blocks whole): a block that does not fit
// moves to the next page, which is what the margin cases measure.
const block = (color, height, style = "") =>
  `<div style="height:${height}px;background:${color};break-inside:avoid;${style}"></div>`;
// Distinct pure colours: [css, rgb].
const C = {
  red: ["#ff0000", [255, 0, 0]],
  green: ["#00ff00", [0, 255, 0]],
  blue: ["#0000ff", [0, 0, 255]],
  yellow: ["#ffff00", [255, 255, 0]],
  magenta: ["#ff00ff", [255, 0, 255]],
  cyan: ["#00ffff", [0, 255, 255]],
  orange: ["#ff8000", [255, 128, 0]],
};
const blocks = (names, height, style) => names.map((n) => block(C[n][0], height, style)).join("");

const CASES = [
  {
    // The first block's top margin (collapsed with body's 8px) is kept on page 1.
    id: "first-page-margin",
    html: DOC("", block(C.red[0], 100, "margin-top:30px") + blocks(["green", "blue", "yellow"], 120)),
    colors: ["red", "green", "blue", "yellow"],
  },
  {
    // A wrapper too tall for any page splits; its children start below its margin + padding
    // on page 1, and every later page starts at the content top (unforced breaks).
    id: "split-block-margin",
    html: DOC(
      "body{margin:0}",
      `<div style="margin-top:50px;padding-top:6px">${blocks(["red", "green", "blue", "yellow", "magenta"], 90, "margin-top:10px")}</div>`,
    ),
    colors: ["red", "green", "blue", "yellow", "magenta"],
  },
  {
    // Margins adjoining an unforced break are truncated at the top of the next page.
    id: "unforced-break-margin",
    html: DOC("body{margin:0}", blocks(["red", "green", "blue", "yellow"], 140, "margin-top:25px")),
    colors: ["red", "green", "blue", "yellow"],
  },
  {
    // After a forced break (`break-before: page`).
    id: "forced-break-margin",
    html: DOC("body{margin:0}", block(C.red[0], 60) + block(C.green[0], 60, "break-before:page;margin-top:30px")),
    colors: ["red", "green"],
  },
  {
    // `<body>`'s border on every page fragment (slice: top edge on the first, bottom on the last).
    id: "body-border",
    html: DOC("body{border:4px solid #ff0000;padding:6px}", blocks(["green", "blue", "yellow", "magenta"], 140)),
    colors: ["red", "green", "blue", "yellow", "magenta"],
  },
  {
    // A body that fits on one page: one unsliced box, both edges.
    id: "body-border-single-page",
    html: DOC("body{border:4px solid #ff0000;padding:6px}", blocks(["green"], 100)),
    colors: ["red", "green"],
  },
  {
    // An empty body: its border box (padding only) on the first page.
    id: "body-border-empty",
    html: DOC("body{border:4px solid #ff0000;padding:6px}", ""),
    colors: ["red"],
  },
  {
    // `<html>`'s border (the root box) and `<body>`'s, nested.
    id: "html-and-body-border",
    html: DOC(
      "html{border:5px solid #ff0000} body{border:3px solid #0000ff}",
      blocks(["green", "yellow", "magenta"], 150),
    ),
    colors: ["red", "blue", "green", "yellow", "magenta"],
  },
  {
    // With an `<html>` background, `<body>`'s background is not propagated to the canvas:
    // it paints in the body box on each page fragment.
    id: "body-background-not-propagated",
    html: DOC("html{background:#00ffff} body{background:#ff8000;padding:10px}", blocks(["green", "blue", "magenta"], 150)),
    colors: ["cyan", "orange", "green", "blue", "magenta"],
  },
];

/** Parse a binary PPM (P6, maxval 255) into { width, height, px(x, y) -> [r,g,b] }. */
function readPpm(buf) {
  const fields = [];
  let i = 0;
  while (fields.length < 4) {
    while (buf[i] === 0x20 || buf[i] === 0x0a || buf[i] === 0x0d || buf[i] === 0x09) i++;
    if (buf[i] === 0x23) {
      while (buf[i] !== 0x0a) i++;
      continue;
    }
    let s = "";
    while (![0x20, 0x0a, 0x0d, 0x09].includes(buf[i])) s += String.fromCharCode(buf[i++]);
    fields.push(s);
  }
  i++;
  const [magic, w, h, max] = fields;
  if (magic !== "P6" || max !== "255") throw new Error(`unexpected PPM ${magic} ${max}`);
  const width = Number(w);
  const height = Number(h);
  const data = buf.subarray(i);
  return { width, height, px: (x, y) => [...data.subarray((y * width + x) * 3, (y * width + x) * 3 + 3)] };
}

function measure(img, rgb) {
  const is = (x, y) => {
    const p = img.px(x, y);
    return p[0] === rgb[0] && p[1] === rgb[1] && p[2] === rgb[2];
  };
  let [x0, y0, x1, y1] = [Infinity, Infinity, -1, -1];
  for (let y = 0; y < img.height; y++)
    for (let x = 0; x < img.width; x++)
      if (is(x, y)) {
        x0 = Math.min(x0, x);
        y0 = Math.min(y0, y);
        x1 = Math.max(x1, x);
        y1 = Math.max(y1, y);
      }
  if (x1 < 0) return null;
  const row = (y) => {
    let n = 0;
    for (let x = x0; x <= x1; x++) if (is(x, y)) n++;
    return n >= 0.9 * (x1 - x0 + 1);
  };
  return { bbox: [x0, y0, x1, y1], top: row(y0), bottom: row(y1) };
}

const work = mkdtempSync(join(tmpdir(), "h2p-pagination-"));
const browser = await chromium.launch({ executablePath: chromePath, headless: true });
const version = browser.version();
const page = await browser.newPage();
const cases = [];
for (const testCase of CASES) {
  await page.setContent(testCase.html, { waitUntil: "load" });
  const pdf = join(work, `${testCase.id}.pdf`);
  await page.pdf({ path: pdf, preferCSSPageSize: true, printBackground: true });
  const prefix = join(work, testCase.id);
  execFileSync("pdftoppm", ["-r", "96", pdf, prefix]);
  const files = readdirSync(work)
    .filter((f) => f.startsWith(`${testCase.id}-`) && f.endsWith(".ppm"))
    .sort();
  const pages = files.map((f) => {
    const img = readPpm(readFileSync(join(work, f)));
    const boxes = {};
    for (const name of testCase.colors) boxes[name] = measure(img, C[name][1]);
    return { size: [img.width, img.height], boxes };
  });
  cases.push({ id: testCase.id, html: testCase.html, colors: Object.fromEntries(testCase.colors.map((n) => [n, C[n][0]])), pages });
}
await browser.close();
rmSync(work, { recursive: true, force: true });

const fixture = {
  generator: "scripts/pagination-chrome-control.mjs",
  chrome: version,
  playwright_core: playwrightVersion,
  raster: "pdftoppm -r 96",
  cases,
};
writeFileSync(out, `${JSON.stringify(fixture, null, 1)}\n`);
console.log(`Chrome ${version}: ${cases.length} cases -> ${out}`);
for (const c of cases) {
  console.log(c.id);
  c.pages.forEach((p, i) =>
    console.log(
      `  page ${i + 1} ${p.size.join("x")}`,
      Object.entries(p.boxes)
        .filter(([, b]) => b)
        .map(([n, b]) => `${n}:${b.bbox.join(",")}${b.top ? "T" : ""}${b.bottom ? "B" : ""}`)
        .join(" "),
    ),
  );
}
