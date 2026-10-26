// Chrome-156 control for paginated box geometry (miaskiewicz/turbo-html2pdf#31): margins at
// page starts (CSS Fragmentation §5.2 — a block's top margin is kept at the start of its first
// fragment, truncated only after a break) and `<html>`/`<body>` box decorations painted on
// every page fragment (`box-decoration-break: slice`). miaskiewicz/turbo-html2pdf#32 adds:
// fixed boxes repeated on every page and absolute boxes placed from their containing block
// through the page capacities; the canvas inside the page area; no blank page for a forced
// break before the first content (or a doubled one); a split block's top border/padding at
// a page start; only the next block's own margin after a forced break; a block that exactly
// fills the page moving on with its ancestors' bottom padding/border; plus a split block's
// remaining height after its content and the flow under a relatively offset block.
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
    html: DOC(
      "",
      block(C.red[0], 100, "margin-top:30px") + blocks(["green", "blue", "yellow"], 120),
    ),
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
    html: DOC(
      "body{margin:0}",
      block(C.red[0], 60) + block(C.green[0], 60, "break-before:page;margin-top:30px"),
    ),
    colors: ["red", "green"],
  },
  {
    // `<body>`'s border on every page fragment (slice: top edge on the first, bottom on the last).
    id: "body-border",
    html: DOC(
      "body{border:4px solid #ff0000;padding:6px}",
      blocks(["green", "blue", "yellow", "magenta"], 140),
    ),
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
    html: DOC(
      "html{background:#00ffff} body{background:#ff8000;padding:10px}",
      blocks(["green", "blue", "magenta"], 150),
    ),
    colors: ["cyan", "orange", "green", "blue", "magenta"],
  },
  // --- miaskiewicz/turbo-html2pdf#32 ---
  {
    // Item 1: a `position: fixed` box repeats on every page at its insets from the page
    // area (the print initial containing block), wherever it sits in the source, and
    // takes no space in the flow.
    id: "fixed-repeats",
    html: DOC(
      "body{margin:0}",
      block(C.green[0], 140) +
        `<div style="position:fixed;top:10px;left:20px;width:50px;height:30px;background:${C.red[0]}"></div>` +
        blocks(["blue", "yellow", "magenta", "orange"], 140),
    ),
    colors: ["red", "green", "blue", "yellow", "magenta", "orange"],
  },
  {
    // Item 1: `bottom`/`right` insets of a fixed box resolve against the page area.
    id: "fixed-bottom-right",
    html: DOC(
      "body{margin:0}",
      `<div style="position:fixed;bottom:10px;right:20px;width:50px;height:30px;background:${C.red[0]}"></div>` +
        blocks(["green", "blue", "yellow"], 140),
    ),
    colors: ["red", "green", "blue", "yellow"],
  },
  {
    // Item 1: an absolute box with no positioned ancestor (the initial containing
    // block, the first page area) whose offset runs past the first page.
    id: "absolute-root-offset",
    html: DOC(
      "body{margin:0}",
      `<div style="position:absolute;top:350px;left:20px;width:50px;height:30px;background:${C.red[0]}"></div>` +
        blocks(["green", "blue", "yellow"], 140),
    ),
    colors: ["red", "green", "blue", "yellow"],
  },
  {
    // Item 1: an absolute box inside a relative container that lands on page 2 sits at
    // its insets from the container, and takes no space in the container's flow.
    id: "absolute-in-relative",
    html: DOC(
      "body{margin:0}",
      block(C.green[0], 200) +
        `<div style="position:relative;height:200px;background:${C.blue[0]};break-inside:avoid">` +
        `<div style="position:absolute;top:20px;left:30px;width:50px;height:30px;background:${C.red[0]}"></div>` +
        block(C.yellow[0], 40) +
        `</div>`,
    ),
    colors: ["red", "green", "blue", "yellow"],
  },
  {
    // Item 1: an absolute box with auto insets sits at its static position and the
    // next block follows the previous one directly.
    id: "absolute-static-position",
    html: DOC(
      "body{margin:0}",
      block(C.green[0], 100) +
        `<div style="position:absolute;width:50px;height:30px;background:${C.red[0]}"></div>` +
        blocks(["blue", "yellow"], 140),
    ),
    colors: ["red", "green", "blue", "yellow"],
  },
  {
    // Item 2: the root background (canvas) paints the page area only, on every page.
    id: "canvas-root-background",
    html: DOC("html{background:#00ffff}", blocks(["green", "blue", "yellow"], 140)),
    colors: ["cyan", "green", "blue", "yellow"],
  },
  {
    // Item 2: `<body>`'s background propagated to the canvas.
    id: "canvas-body-background",
    html: DOC("body{background:#ff8000}", blocks(["green", "blue"], 200)),
    colors: ["orange", "green", "blue"],
  },
  {
    // Item 3: a forced break before the first content is ignored (no blank page 1);
    // the block keeps its top margin.
    id: "break-before-first",
    html: DOC(
      "body{margin:0}",
      block(C.red[0], 60, "break-before:page;margin-top:20px") + block(C.green[0], 60),
    ),
    colors: ["red", "green"],
  },
  {
    // Item 3: the same through a wrapper (the break propagates to the start of the flow).
    id: "break-before-first-nested",
    html: DOC(
      "",
      `<div>${block(C.red[0], 60, "break-before:page")}${block(C.green[0], 60)}</div>` +
        block(C.blue[0], 60, "break-before:page"),
    ),
    colors: ["red", "green", "blue"],
  },
  {
    // Item 4: a split block that starts a page after an unforced break keeps its top
    // border and padding on its first fragment (only its margin is truncated).
    id: "split-block-page-start",
    html: DOC(
      "body{margin:0}",
      block(C.red[0], 250) +
        `<div style="margin-top:30px;border-top:5px solid transparent;padding-top:10px">${blocks(["green", "yellow", "magenta"], 140)}</div>`,
    ),
    colors: ["red", "green", "yellow", "magenta"],
  },
  {
    // Item 4: nested split blocks at a page start: both top borders + paddings are kept.
    id: "split-block-page-start-nested",
    html: DOC(
      "body{margin:0}",
      block(C.red[0], 250) +
        `<div style="margin-top:30px;padding-top:7px"><div style="margin-top:12px;border-top:3px solid transparent;padding-top:4px">${blocks(["green", "yellow", "magenta"], 140)}</div></div>`,
    ),
    colors: ["red", "green", "yellow", "magenta"],
  },
  {
    // Item 5: after a forced break only the next block's own top margin is kept, not
    // the larger bottom margin of the block before the break.
    id: "forced-break-prev-margin",
    html: DOC(
      "body{margin:0}",
      block(C.red[0], 60, "margin-bottom:80px") +
        block(C.green[0], 60, "break-before:page;margin-top:20px"),
    ),
    colors: ["red", "green"],
  },
  {
    // Item 5: `break-after` on the previous block, with no top margin on the next one.
    id: "forced-break-after-margin",
    html: DOC(
      "body{margin:0}",
      block(C.red[0], 60, "margin-bottom:80px;break-after:page") + block(C.green[0], 60),
    ),
    colors: ["red", "green"],
  },
  {
    // Item 5: the next block's top margin collapses with its first child's.
    id: "forced-break-child-margin",
    html: DOC(
      "body{margin:0}",
      block(C.red[0], 60, "margin-bottom:80px") +
        `<div style="break-before:page;margin-top:10px">${block(C.green[0], 60, "margin-top:25px")}</div>`,
    ),
    colors: ["red", "green"],
  },
  {
    // Item 6: a block that exactly fills the page, followed by its parent's bottom
    // padding, moves to the next page with the padding.
    id: "exact-fill-parent-padding",
    html: DOC(
      "body{margin:0}",
      `<div style="padding-bottom:20px">${block(C.blue[0], 100)}${block(C.red[0], 220)}</div>` +
        block(C.green[0], 60),
    ),
    colors: ["blue", "red", "green"],
  },
  {
    // Item 6: the same with a bottom border instead of padding.
    id: "exact-fill-parent-border",
    html: DOC(
      "body{margin:0}",
      `<div style="border-bottom:20px solid transparent">${block(C.blue[0], 100)}${block(C.red[0], 220)}</div>` +
        block(C.green[0], 60),
    ),
    colors: ["blue", "red", "green"],
  },
  {
    // Item 6 (control): the exact fill without a parent padding stays on the page.
    id: "exact-fill-no-padding",
    html: DOC(
      "body{margin:0}",
      `<div>${block(C.blue[0], 100)}${block(C.red[0], 220)}</div>` + block(C.green[0], 60),
    ),
    colors: ["blue", "red", "green"],
  },
  {
    // Item 6: the last child's bottom margin inside the padded parent counts too.
    id: "exact-fill-child-margin",
    html: DOC(
      "body{margin:0}",
      `<div style="padding-bottom:20px">${block(C.blue[0], 100)}${block(C.red[0], 200, "margin-bottom:20px")}</div>` +
        block(C.green[0], 60),
    ),
    colors: ["blue", "red", "green"],
  },
  {
    // Item 3: `break-after` on one block and `break-before` on the next are one break
    // point: one page break, no blank page between.
    id: "forced-break-after-and-before",
    html: DOC(
      "body{margin:0}",
      block(C.red[0], 60, "break-after:page") + block(C.green[0], 60, "break-before:page"),
    ),
    colors: ["red", "green"],
  },
  {
    // Item 6: `<body>`'s bottom padding follows the last block the same way.
    id: "exact-fill-body-padding",
    html: DOC("body{margin:0;padding-bottom:20px}", block(C.blue[0], 100) + block(C.red[0], 220)),
    colors: ["blue", "red"],
  },
  {
    // Item 6 (control): a trailing margin is truncated at the page end: the block stays.
    id: "exact-fill-body-margin",
    html: DOC("body{margin:0 0 20px}", block(C.blue[0], 100) + block(C.red[0], 220)),
    colors: ["blue", "red"],
  },
  {
    // Item 6: the padding of a grandparent the block's parent ends with.
    id: "exact-fill-grandparent-padding",
    html: DOC(
      "body{margin:0}",
      `<div style="padding-bottom:20px"><div>${block(C.blue[0], 100)}${block(C.red[0], 220)}</div></div>` +
        block(C.green[0], 60),
    ),
    colors: ["blue", "red", "green"],
  },
  {
    // A split block taller than its content: the block stays (its remaining height can
    // break), and the remaining height + bottom padding continue at the top of the next
    // page before the next block.
    id: "split-block-explicit-height",
    html: DOC(
      "body{margin:0}",
      `<div style="height:380px;padding-bottom:10px">${block(C.blue[0], 100)}${block(C.red[0], 220)}</div>` +
        block(C.green[0], 60),
    ),
    colors: ["blue", "red", "green"],
  },
  {
    // Item 1: an absolute box far past the content creates the pages it lands on.
    id: "absolute-root-past-content",
    html: DOC(
      "body{margin:0}",
      `<div style="position:absolute;top:900px;left:20px;width:50px;height:30px;background:${C.red[0]}"></div>` +
        block(C.green[0], 100),
    ),
    colors: ["red", "green"],
  },
  {
    // Item 1: `bottom` of an absolute box in the initial containing block (the first
    // page area).
    id: "absolute-root-bottom",
    html: DOC(
      "body{margin:0}",
      `<div style="position:absolute;bottom:10px;left:20px;width:50px;height:30px;background:${C.red[0]}"></div>` +
        blocks(["green", "blue", "yellow"], 140),
    ),
    colors: ["red", "green", "blue", "yellow"],
  },
  {
    // Item 1: a fixed box with auto insets repeats at its static position.
    id: "fixed-static-position",
    html: DOC(
      "body{margin:0}",
      block(C.green[0], 100) +
        `<div style="position:fixed;width:50px;height:30px;background:${C.red[0]}"></div>` +
        blocks(["blue", "yellow"], 140),
    ),
    colors: ["red", "green", "blue", "yellow"],
  },
  {
    // Item 1: an absolute box in a relative container split across pages: offsets run
    // from the container's start (here page 2, its first child moved there) through the
    // page capacities.
    id: "absolute-in-split-relative",
    html: DOC(
      "body{margin:0}",
      block(C.green[0], 200) +
        `<div style="position:relative">${blocks(["yellow", "magenta", "orange"], 140)}` +
        `<div style="position:absolute;top:400px;left:20px;width:50px;height:30px;background:${C.red[0]}"></div></div>`,
    ),
    colors: ["red", "green", "yellow", "magenta", "orange"],
  },
  {
    // Item 1: the same container starting on page 1.
    id: "absolute-in-split-relative-page-1",
    html: DOC(
      "body{margin:0}",
      block(C.green[0], 100) +
        `<div style="position:relative">${blocks(["yellow", "magenta", "orange"], 140)}` +
        `<div style="position:absolute;top:400px;left:20px;width:50px;height:30px;background:${C.red[0]}"></div></div>`,
    ),
    colors: ["red", "green", "yellow", "magenta", "orange"],
  },
  {
    // Item 1: an absolute box in a static block that moves to page 2 stays relative to
    // the initial containing block (page 1).
    id: "absolute-in-moved-static",
    html: DOC(
      "body{margin:0}",
      block(C.green[0], 200) +
        `<div style="height:200px;background:${C.blue[0]};break-inside:avoid">` +
        `<div style="position:absolute;top:10px;left:20px;width:50px;height:30px;background:${C.red[0]}"></div></div>`,
    ),
    colors: ["red", "green", "blue"],
  },
  {
    // Item 1: a fixed box with a negative `z-index` paints under the flow.
    id: "fixed-negative-z-index",
    html: DOC(
      "body{margin:0}",
      `<div style="position:fixed;top:10px;left:20px;width:50px;height:30px;background:${C.red[0]};z-index:-1"></div>` +
        blocks(["green"], 140),
    ),
    colors: ["red", "green"],
  },
  {
    // A relatively positioned block is painted shifted, but the flow continues from its
    // unshifted position (and it paints over the next block).
    id: "relative-offset-flow",
    html: DOC(
      "body{margin:0}",
      block(C.red[0], 60, "position:relative;top:30px") + block(C.green[0], 60),
    ),
    colors: ["red", "green"],
  },
  {
    // The same for a relatively offset block split across pages: its content is painted
    // shifted on every page, the page fill and the next block follow the flow.
    id: "relative-offset-split",
    html: DOC(
      "body{margin:0}",
      `<div style="position:relative;top:20px">${blocks(["yellow", "magenta", "orange"], 140)}</div>` +
        block(C.green[0], 60),
    ),
    colors: ["yellow", "magenta", "orange", "green"],
  },
  {
    // A table split across pages repeats its header row; the next row follows it.
    id: "table-header-repeat",
    html: DOC(
      "body{margin:0} table{border-collapse:collapse;border-spacing:0} td{padding:0;width:520px}",
      `<table><thead><tr><td style="height:30px;background:${C.red[0]}"></td></tr></thead><tbody>` +
        ["green", "blue", "yellow", "magenta"]
          .map(
            (n) =>
              `<tr style="break-inside:avoid"><td style="height:100px;background:${C[n][0]}"></td></tr>`,
          )
          .join("") +
        "</tbody></table>",
    ),
    colors: ["red", "green", "blue", "yellow", "magenta"],
  },
  // --- miaskiewicz/turbo-html2pdf#38 ---
  {
    // A forced break between the children of a block that fits on the page.
    id: "forced-break-nested",
    html: DOC(
      "body{margin:0}",
      `<div>${block(C.red[0], 60)}${block(C.green[0], 60, "break-before:page")}</div>`,
    ),
    colors: ["red", "green"],
  },
  {
    // `break-after` two levels down.
    id: "forced-break-after-nested",
    html: DOC(
      "body{margin:0}",
      `<div><div>${block(C.red[0], 60, "break-after:page")}${block(C.green[0], 60)}</div></div>` +
        block(C.blue[0], 60),
    ),
    colors: ["red", "green", "blue"],
  },
  // --- miaskiewicz/turbo-html2pdf#37 ---
  {
    // Item 1: a block with a background and border split across pages paints a slice of
    // its box on every page (box-decoration-break: slice): top edge on the first, bottom
    // edge on the last, the box running to the page area's end in between.
    id: "split-decorated",
    html: DOC(
      "body{margin:0}",
      `<div style="border:4px solid ${C.blue[0]};background:${C.orange[0]};padding:6px">${blocks(["green", "yellow", "magenta"], 140)}</div>`,
    ),
    colors: ["blue", "orange", "green", "yellow", "magenta"],
  },
  {
    // Item 1: nested decorated blocks split together; the inner box paints over the outer.
    id: "split-decorated-nested",
    html: DOC(
      "body{margin:0}",
      `<div style="border:4px solid ${C.blue[0]};background:${C.orange[0]};padding:6px">` +
        `<div style="border:3px solid ${C.red[0]};background:${C.cyan[0]};padding:4px">${blocks(["green", "yellow", "magenta"], 120)}</div></div>`,
    ),
    colors: ["blue", "orange", "red", "cyan", "green", "yellow", "magenta"],
  },
  {
    // Item 1: a decorated block that starts on page 1 below other content and splits.
    id: "split-decorated-after-content",
    html: DOC(
      "body{margin:0}",
      block(C.red[0], 100) +
        `<div style="border-top:5px solid ${C.blue[0]};border-bottom:5px solid ${C.blue[0]};background:${C.orange[0]}">${blocks(["green", "yellow"], 150)}</div>`,
    ),
    colors: ["red", "blue", "orange", "green", "yellow"],
  },
  {
    // Item 2: a percentage top margin resolves against the containing block's width
    // (10% of 200px = 20px), here kept after a forced break.
    id: "percent-margin-nested",
    html: DOC(
      "body{margin:0}",
      block(C.red[0], 60, "margin-bottom:80px") +
        `<div style="width:200px">${block(C.green[0], 60, "break-before:page;margin-top:10%")}</div>`,
    ),
    colors: ["red", "green"],
  },
  {
    // Item 2: a percentage bottom padding (10% of its 200px containing block = 20px)
    // follows the last block, which exactly fills the page and so moves on with it.
    id: "percent-padding-nested",
    html: DOC(
      "body{margin:0}",
      `<div style="width:200px"><div style="padding-bottom:10%">${block(C.blue[0], 100)}${block(C.red[0], 220)}</div></div>` +
        block(C.green[0], 60),
    ),
    colors: ["blue", "red", "green"],
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
  return {
    width,
    height,
    px: (x, y) => [...data.subarray((y * width + x) * 3, (y * width + x) * 3 + 3)],
  };
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
  cases.push({
    id: testCase.id,
    html: testCase.html,
    colors: Object.fromEntries(testCase.colors.map((n) => [n, C[n][0]])),
    pages,
  });
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
