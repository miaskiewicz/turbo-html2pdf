# turbo-html2pdf

**Turn HTML + CSS (with a Jinja templating layer) into a PDF — natively, in Rust,
with no headless browser.** A from-scratch document engine: templating → HTML/CSS
layout → automatic pagination → PDF, shipped as a tiny native addon for Node and
WebAssembly. No Chromium download, no 200 ms browser spin-up, deterministic output.

> The name says exactly what it does — HTML → PDF — and avoids clashing with the
> unrelated "TurboPDF".

## Packages

One engine, shipped to every ecosystem. Pick the one for your runtime:

| Package | Registry | What it is | Install |
|---|---|---|---|
| **`turbo-html2pdf`** | npm | **Node** native addon — `compile`/`render` → PDF `Buffer`. The default. | `npm i turbo-html2pdf` |
| **`turbo-html2pdf-svg`** | npm | Same as above **+ SVG images** (bundles `resvg`). Use only if you embed SVG. | `npm i turbo-html2pdf-svg` |
| **`turbo-html2pdf-wasm`** | npm | **Browser / WASM** — the full engine client-side, no server. | `npm i turbo-html2pdf-wasm` |
| **`turbo-html2pdf-react`** | npm | Author templates as **React** components → template string. | `npm i turbo-html2pdf-react` |
| **`turbo-html2pdf-template`** | npm | Author templates with **plain functions** (no React). | `npm i turbo-html2pdf-template` |
| **`turbo-html2pdf`** | PyPI | **Python** binding (PyO3) — `compile`/`render` → `bytes`. | `pip install turbo-html2pdf` |

The two authoring packages (`-react`, `-template`) only *produce the template
string*; they pair with `turbo-html2pdf` (Node), `turbo-html2pdf-wasm` (browser),
or the Python package to actually render. The Rust engine lives in
[`crates/turbo-pdf-core`](https://github.com/miaskiewicz/turbo-html2pdf/tree/main/crates/turbo-pdf-core).

> Status: `turbo-html2pdf`, `-react`, `-template` are live on npm at `0.1.0`;
> `-svg`, `-wasm`, and the PyPI package ship with `v0.1.1`.

## 🌐 It generates PDFs entirely in the browser

Because the engine is pure Rust → WebAssembly (~3 MB), it runs **100% client-side**
— no server, no backend, no Chromium. A user pastes HTML/CSS, supplies a font as a
`Uint8Array`, and gets PDF bytes, all in the browser tab.

As far as we know, **no other library does HTML/CSS → PDF in the browser**: the
HTML→PDF tools that match its fidelity (Puppeteer, Playwright, Gotenberg, WeasyPrint,
wkhtmltopdf) are all **server-side** — they drive a headless browser or a native
binary you have to host. The libraries that *do* run in the browser (jsPDF, PDFKit,
react-pdf) are **draw APIs**, not HTML/CSS layout engines — you place every box by
hand. turbo-html2pdf is the only one that is *both* a real HTML/CSS engine *and*
runs with zero server.

```js
import init, { compile } from 'turbo-html2pdf-wasm'
await init()                                  // load the ~3 MB wasm once
const program = compile('<h1>{{ t }}</h1>')
const { pdf } = program.render({ data: { t: 'Hello' }, css: 'h1{font-size:24pt}',
                                 fonts: [{ data: fontBytes, family: 'Inter' }] })
// `pdf` is a Uint8Array — download it, no round-trip to a server
```

---

## Why it's fast

It does **HTML/CSS → PDF without a browser**. The incumbent way to turn HTML into a
PDF is to drive headless Chrome (Puppeteer/Playwright/Gotenberg) — which means
shipping ~200 MB of Chromium, paying a per-process spin-up, and a big memory
footprint. turbo-html2pdf is a native layout + PDF engine instead, so the common
"render one document" path is **tens to hundreds of times faster**.

### Benchmarks (this machine: Apple M3, 8 cores; `benches/competitive`)

Two scenarios, both measured:

- **Warm (server):** the realistic server path — the template's compiled `Program`
  is reused and **fonts are parsed once and cached** (the `Fonts` handle). This is
  the number to lead with.
- **Cold (one-shot):** a single render from a cold start (compile + parse fonts +
  render), e.g. a CLI invocation.

All engines render the **same** content (same fonts, A4 geometry, same text); a
PNG-equivalence diff (`sim` ≈ 0.98 vs the reference) confirms the outputs are
comparable, so this isn't "winning by doing less".

**`invoice` — a one-page document (the bread-and-butter case), median ms:**

| Engine | Warm (cached) | Cold | vs turbo (warm) | Memory | Ships a browser? |
|---|--:|--:|--:|--:|:--:|
| **turbo-html2pdf** | **1.27** | **1.48** | **1×** | **232 MB** | **no** |
| Playwright (Chromium) | 46.3 | 49.4 | **36× slower** | 393 MB | yes (~200 MB) |
| Gotenberg (Chromium) | 66.9 | 70.6 | **53× slower** | 424 MB | yes (~1 GB) |
| Puppeteer (Chromium) | 86.1 | 132.2 | **68× slower** | 550 MB | yes (~200 MB) |
| Typst (native, own DSL) | 63.8 | 62.7 | **50× slower** | 1.5 GB | no |
| WeasyPrint (Python) | 375.8 | 379.3 | **296× slower** | 379 MB | no |

**It's also the fastest on every other workload — not just the small one:**

| Workload | turbo-html2pdf (warm/cold) | Playwright | Puppeteer | Gotenberg | WeasyPrint | Typst |
|---|--:|--:|--:|--:|--:|--:|
| `invoice` (1 page) | **1.27 / 1.48** | 46 / 49 | 86 / 132 | 67 / 71 | 376 / 379 | 64 / 63 |
| `report-1k` (1 000-row table) | **119 / 130** | 187 / 170 | 384 / 429 | 204 / 204 | 2 550 / 2 547 | 220 / 221 |
| `legal` (prose + footnotes) | **25 / 26** | 71 / 98 | 124 / 147 | 80 / 80 | 444 / 453 | 72 / 72 |
| `mixed` (flex + table) | **8.6 / 9.2** | 57 / 58 | 94 / 229 | 73 / 73 | 432 / 437 | 69 / 71 |

Against every **HTML/CSS-to-PDF** engine, turbo-html2pdf wins on **all** workloads —
by 1.6× (big tables) up to ~300× (small docs) — at **lower memory** and with **no
browser** to install.

> **Honest footnote.** `pdfkit` (~6 ms) and `jspdf` (~2 ms) are faster on raw
> output, but they are **imperative draw APIs** — *you* compute and place every box;
> there is no HTML/CSS, no layout engine, no pagination. `@react-pdf` is a React
> component DSL (no HTML/CSS) and its flexbox layout melts on big tables (`report-1k`
> ≈ **1 300 ms**, ~10× slower than turbo). Different category from "give me HTML, get
> a PDF". `wkhtmltopdf` (legacy/unmaintained) was not installed.

Numbers are "on this machine", never absolutes — rerun with `cd benches/competitive
&& pnpm bench`. Full table + methodology in
[`benches/competitive/RESULTS.md`](benches/competitive/RESULTS.md).

### Why it's *that* fast

- **No browser, no process spin-up.** It's a library call, not an IPC round-trip to
  Chrome.
- **The font program is parsed once.** `ttf-parser` + `rustybuzz` faces are cached
  in the `FontFace` (via `self_cell`) instead of re-parsed per text run; with the
  `Fonts` warm-start handle a server parses fonts once at startup and reuses them.
- **Box styles resolve once per box.** Context-independent boxes cache their
  resolved `BoxStyle` instead of re-parsing ~25 CSS properties on every layout pass.
- **Shaping is memoized** by run text (the measure + place passes, and repeated
  cells, share one shape).
- **Deterministic.** No wall clock, no randomness, no system-font lookup — identical
  inputs produce byte-identical PDFs.

---

## Architecture

A template is **compiled once** into a reusable `Program`, then rendered against
data many times. The pipeline (`crates/turbo-pdf-core`):

```
template (HTML + Jinja + t: directives)
   │  compile()                      ── parse + cache (MiniJinja)
   ▼
render_pages(data, css, fonts):
   │  1. render markup               ── Jinja → HTML string (data substituted)
   │  2. parse                       ── html5ever → node tree, t: directives typed
   │  3. cascade + inherit           ── CSS subset → ComputedStyle per node
   │  4. layout                      ── box tree → block / inline / flex (taffy) / table
   │                                    → a "galley" of positioned, shaped fragments
   │  5. paginate                    ── break the galley into pages (orphans/widows,
   │                                    break rules, repeated <thead>); resolve
   │                                    running headers/footers + footnotes per page
   ▼  emit_pdf()                     ── fragments → PDF 1.7: subset + embed fonts
                                        (TrueType / Type0-CFF), text, vector boxes/
   PDF bytes                            borders, raster images, watermark
```

Layout, font shaping (`rustybuzz`), and PDF writing (`pdf-writer` + `subsetter`) are
all native; `taffy` powers flexbox. The engine embeds no fonts and does no network
or filesystem I/O — fonts and images are supplied by the caller.

**Frontends & bindings** (`packages/`, `crates/`):
- `turbo-html2pdf` — Node native addon (`compile`/`render` → `Buffer`).
- `turbo-html2pdf-wasm` — the same engine in the browser (WebAssembly).
- `turbo-html2pdf-react` — author templates as React components (compiled to the
  template string at build time, never on the render path).
- `turbo-html2pdf-template` — author templates with plain functions (no React).

The engine is `Send + Sync`: one compiled `Program` renders concurrently across
threads.

---

## The DSL — HTML for documents

You write **ordinary HTML + a CSS subset**, plus a Jinja templating layer for data,
plus a handful of `t:` directives for paged-media features browsers can't do.
Full reference in [`docs/`](docs/): [`dsl.md`](docs/dsl.md),
[`paged-media.md`](docs/paged-media.md), [`css-support.md`](docs/css-support.md),
[`api.md`](docs/api.md).

- **Templating (Jinja / MiniJinja):** `{{ value }}`, `{% for %}`, `{% if %}`,
  includes/macros, plus document filters (`currency`, `number`, `percent`, `ordinal`,
  `date`, `datetime`, …) and a `{% switch %}` / `{% case %}` extension.
- **`t:` directives** (the paged-media layer):
  - `<t:running-header>` / `<t:running-footer>` — headers/footers repeated on every
    page, re-evaluated per page so `{{ page.number }}` / `{{ page.total }}` (and
    `<t:page/>` / `<t:pages/>`) are correct.
  - `<t:footnote>` — auto-numbered footnotes; the body lands on the page where the
    reference falls (content-driven, with a body/footnote fix-point).
  - Pagination is automatic: **page count is an output, never an input.** `@page`
    sets size/margins; break rules + orphans/widows are honored; `<thead>` repeats
    when a table spans pages.

```js
const { compile, Fonts } = require('turbo-html2pdf')
const fs = require('node:fs')

// Warm at startup, reuse per request — the fast path.
const program = compile('<h1>{{ title }}</h1><p>{{ body }}</p>')
const fonts = Fonts.load([fs.readFileSync('Inter.ttf')])   // parse fonts ONCE

const { pdf, pageCount } = program.render({
  data: { title: 'Hello', body: 'World' },
  css: '@page { size: A4; margin: 20mm } h1 { font-size: 24pt }',
  meta: { title: 'My Doc' },
}, fonts)                                                   // reuse the handle

fs.writeFileSync('out.pdf', pdf)   // %PDF-1.7
```

---

## Features

- HTML + CSS subset (block / inline / flexbox / tables), automatic pagination.
- Running headers & footers with per-page values; auto-numbered footnotes.
- Font subsetting + embedding (TrueType & CFF/OpenType); per-glyph fallback.
- Raster images (PNG/JPEG, alpha → SMask) with a sane max-size clamp.
- Watermarks: out-of-the-box faded diagonal **DRAFT** text (word/colour/angle
  configurable) and background-image watermarks.
- Deterministic output; `Send + Sync`; no network / no system fonts.

### Built-in capabilities (on by default)

These are compiled into the **published packages** — no special build, no rebuild.
They cost nothing until used (a doc that has no `<t:endnote>` pays nothing for
endnotes, etc.).

| Capability | How to use it |
|---|---|
| **Internal links & cross-references** | Mark a spot with `<t:anchor name="ch2"/>`; link to it with `<a href="#ch2">see Chapter 2</a>` → clickable, jumps there in the PDF. |
| **Endnotes** | `<t:endnote>…note body…</t:endnote>` drops a numbered marker and collects the note; `<t:endnotes/>` renders the collected list at its position. |
| **PDF/A-2b (archival)** | Request it on the render options (`pdfA: true`) → the document is emitted PDF/A-2b conformant (embedded sRGB ICC, XMP, no transparency). Validates green under veraPDF `--flavour 2b`. |
| **PDF/UA (accessible / tagged)** | Request it on the render options (`pdfUa: true`) → adds the tagged structure tree (headings/lists/tables, `<img alt>`, reading order) for screen readers. |
| **CMYK print color** | Request it on the render options (`cmyk: true`) → output is emitted in DeviceCMYK for a real press. **Default is RGB** (right for screen). |

> PDF/A, PDF/UA and CMYK are **modes you opt into per render** (they change the
> output); they're *available* by default but only *active* when you ask — the
> default stays screen-friendly RGB, no transparency restrictions, untagged.
> Endnotes and links just work from the markup, no option needed.

### Opt-in: SVG images

SVG vector images (`<img>`/`background-image`) are **off by default to keep the
download small** — SVG pulls in the [`resvg`](https://crates.io/crates/resvg)
rasterizer (a few MB), which matters most for the bundle. Everything else (PNG/JPEG,
all layout) works without it. **Need SVG? Just install the SVG-enabled package** —
same API, prebuilt with resvg, no build step:

```bash
npm i turbo-html2pdf        # default — small, no SVG
npm i turbo-html2pdf-svg    # identical API, SVG support baked in (resvg)
```

(Rust users: enable the feature directly — `turbo-pdf-core = { features = ["svg"] }`.)

### Watermarks

A watermark is a **render option** — it paints behind the page body on every page.
Two kinds:

- **Text** — a faded, rotated word. Out-of-the-box `DRAFT` (gray, 25% opacity, 45°),
  or any word/colour/opacity/angle. The word is shaped + embedded from one of the
  fonts you pass.
- **Image** — a raster image behind the body, centered or tiled, at a chosen opacity.

```js
// DRAFT stamp (uses the first provided font), via the render options:
program.render({
  data, css, fonts: [draftFont],
  watermark: { text: 'DRAFT' },                         // defaults: gray, 25%, 45°
})

// fully custom text:
watermark: { text: 'CONFIDENTIAL', color: '#cc0000', opacity: 0.15, angle: 30 }

// image watermark (name resolved from the provided images), tiled & faint:
watermark: { image: 'logo', tiled: true, opacity: 0.08 }
```

In Rust the same lives on `EmitOptions.watermark` (`Watermark::Text(TextWatermark)` /
`Watermark::Image(ImageWatermark)`; `TextWatermark::draft(face)` is the preset).

> The watermark **render option** in the JS/WASM/Python bindings ships in `v0.1.1`
> (the engine has had it since the PDF emitter; the bindings just expose it).

## Status

`v0.1.0`. The core engine is complete and heavily tested (the `turbo-pdf-core`
crate holds 100% line coverage with a cyclomatic-complexity ≤ 5 gate). Bindings:
Node (napi) and WebAssembly. See [`docs/`](docs/) for the full guide and
[`benches/competitive/`](benches/competitive/) for the benchmark harness.

## License

MIT.
