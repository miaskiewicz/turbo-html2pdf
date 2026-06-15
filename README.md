# turbo-html2pdf

**Turn HTML + CSS (with a Jinja templating layer) into a PDF — natively, in Rust,
with no headless browser.** A from-scratch document engine: templating → HTML/CSS
layout → automatic pagination → PDF, shipped as a tiny native addon for Node, a
WebAssembly build for the browser, and a Python wheel. No Chromium download, no
200 ms browser spin-up, deterministic output — and it renders with **zero caller
fonts**, because the default sans/serif/mono fonts ship in the build.

> The name says exactly what it does — HTML → PDF — and avoids clashing with the
> unrelated "TurboPDF".

## Packages

One engine, shipped to every ecosystem. Pick the one for your runtime:

| Package | Registry | What it is | Install |
|---|---|---|---|
| **`turbo-html2pdf`** | npm | **Node** native (N-API) addon — `compile`/`render` → PDF `Buffer`. The default; bundles the default fonts. | `npm i turbo-html2pdf` |
| **`turbo-html2pdf-svg`** | npm | Same Node engine **+ SVG images** (built with the `svg` feature → bundles `resvg`). Drop-in for `turbo-html2pdf` when you embed SVG `<img>` content. | `npm i turbo-html2pdf-svg` |
| **`turbo-html2pdf-wasm`** | npm | **Browser / WASM** — the full engine client-side, no server. **Lean: ships no fonts** (the caller supplies font bytes via the `fonts` API). | `npm i turbo-html2pdf-wasm` |
| **`turbo-html2pdf-wasm-fonts`** | npm | Same browser engine **with the default OFL fonts embedded in the `.wasm`** — zero-config rendering, heavier (~6 MB) download. | `npm i turbo-html2pdf-wasm-fonts` |
| **`turbo-html2pdf-react`** | npm | Author templates as **React** components → template string (at authoring time). | `npm i turbo-html2pdf-react` |
| **`turbo-html2pdf-template`** | npm | Author templates with **plain functions** (no React). | `npm i turbo-html2pdf-template` |
| **`turbo-html2pdf`** | PyPI | **Python** binding (PyO3 / maturin) — `compile`/`render` → `bytes`. Bundles the default fonts. | `pip install turbo-html2pdf` |

The two authoring packages (`-react`, `-template`) only *produce the template
string* — they pair with a render package (`turbo-html2pdf` on Node, a
`turbo-html2pdf-wasm*` build in the browser, or the PyPI wheel) to actually emit a
PDF. The Rust engine lives in
[`crates/turbo-pdf-core`](https://github.com/miaskiewicz/turbo-html2pdf/tree/main/crates/turbo-pdf-core).

> Status: the npm and PyPI packages ship at **`v0.1.3`**.

## 🌐 It generates PDFs entirely in the browser

Because the engine is pure Rust → WebAssembly, it runs **100% client-side** — no
server, no backend, no Chromium. A user pastes HTML/CSS and gets PDF bytes, all in
the browser tab.

As far as we know, **no other library does HTML/CSS → PDF in the browser**: the
HTML→PDF tools that match its fidelity (Puppeteer, Playwright, Gotenberg, WeasyPrint,
wkhtmltopdf) are all **server-side** — they drive a headless browser or a native
binary you have to host. The libraries that *do* run in the browser (jsPDF, PDFKit,
react-pdf) are **draw APIs**, not HTML/CSS layout engines — you place every box by
hand. turbo-html2pdf is the only one that is *both* a real HTML/CSS engine *and*
runs with zero server.

### Two browser builds: lean vs. fonts-included

The WASM engine comes in two flavours, so you only pay for what you need:

- **`turbo-html2pdf-wasm` (lean, ~3 MB)** — ships **no bundled fonts**. The browser
  caller supplies font bytes at runtime via the `fonts` API. Smallest download;
  ideal when you already have the fonts you want to embed.
- **`turbo-html2pdf-wasm-fonts` (~6 MB)** — the **default OFL font set is embedded
  in the `.wasm`** itself, so `font-family: sans-serif | serif | monospace` resolve
  with zero caller setup. Heavier download, but renders out of the box.

```js
// lean build — you supply the font bytes:
import init, { compile } from 'turbo-html2pdf-wasm'
await init()                                  // load the wasm once
const program = compile('<h1>{{ t }}</h1>')
const { pdf } = program.render({ data: { t: 'Hello' }, css: 'h1{font-size:24pt}',
                                 fonts: [{ data: fontBytes, family: 'Inter' }] })
// `pdf` is a Uint8Array — download it, no round-trip to a server

// fonts-included build — no fonts argument needed:
import init, { compile } from 'turbo-html2pdf-wasm-fonts'
await init()
const program = compile('<h1>{{ t }}</h1>')
const { pdf } = program.render({ data: { t: 'Hi' },
                                 css: 'h1{font-family:sans-serif;font-size:24pt}' })
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
  inputs produce byte-identical PDFs (unless you turn on encryption, which is
  intentionally non-deterministic; see below).

---

## Fonts — renders with zero caller setup

The Node and Python builds **bundle a default font set** (default-on), so
`font-family: sans-serif | serif | monospace` resolve out of the box and a document
renders without you supplying a single font byte:

| Generic family | Primary | Fallback |
|---|---|---|
| `sans-serif` | **Inter** | Roboto |
| `serif` | **Liberation Serif** | PT Serif |
| `monospace` | **Fira Code** | IBM Plex Mono |

All bundled faces are **SIL Open Font License 1.1**. The attribution that ships with
the font assets is in [`assets/fonts/NOTICE.md`](crates/turbo-pdf-core/assets/fonts/NOTICE.md);
keep it alongside the binary when you redistribute.

You can still pass your own fonts (via `Fonts.load(...)` on Node or the per-render
`fonts` argument) — caller fonts win, the bundled ones are the fallback. The browser
split is the exception: **`turbo-html2pdf-wasm` ships lean (no fonts)** and the
caller supplies font bytes, while **`turbo-html2pdf-wasm-fonts`** embeds the same
default set in the `.wasm`.

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
   ▼  emit_pdf()                     ── fragments → PDF: subset + embed fonts
                                        (TrueType / Type0-CFF), text, vector boxes/
   PDF bytes                            borders, raster images, links, watermark
```

Layout, font shaping (`rustybuzz`), and PDF writing (`pdf-writer` + `subsetter`) are
all native; `taffy` powers flexbox. The engine does no network or filesystem I/O —
images are supplied by the caller, and fonts are either the bundled defaults or the
ones you pass.

**Frontends & bindings** (`packages/`, `crates/`):
- `turbo-html2pdf` — Node native (N-API) addon (`compile`/`render` → `Buffer`).
- `turbo-html2pdf-svg` — the same Node engine built with the `svg` feature (resvg).
- `turbo-html2pdf-wasm` / `-wasm-fonts` — the engine in the browser (WebAssembly),
  lean or with the default fonts embedded.
- `turbo-html2pdf-react` — author templates as React components (compiled to the
  template string at authoring time, never on the render path).
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
  - `<t:anchor name="…">` — a named cross-reference target (see Internal links).
  - Pagination is automatic: **page count is an output, never an input.** `@page`
    sets size/margins; break rules + orphans/widows are honored; `<thead>` repeats
    when a table spans pages.

```js
const { compile, Fonts } = require('turbo-html2pdf')

// Warm at startup, reuse per request — the fast path.
// No fonts needed: sans-serif/serif/monospace resolve to the bundled defaults.
const program = compile('<h1>{{ title }}</h1><p>{{ body }}</p>')

const { pdf, pageCount } = program.render({
  data: { title: 'Hello', body: 'World' },
  css: '@page { size: A4; margin: 20mm } h1 { font-size: 24pt }',
  meta: { title: 'My Doc' },
})

require('node:fs').writeFileSync('out.pdf', pdf)

// Bring your own fonts (parsed ONCE, reused per request) when you need them:
const fonts = Fonts.load([require('node:fs').readFileSync('Inter.ttf')])
program.render({ data, css, meta }, fonts)   // caller fonts win over the defaults
```

---

## Conformance & security

PDF/A, PDF/UA, CMYK and encryption are **per-render options** on the `render` call.
They are **off by default**, so the default output is byte-deterministic plain RGB,
untagged, screen-targeted. Turn one on only when you need it — each changes the
output:

| Option | What it does |
|---|---|
| **`pdfA: true`** | Emit **PDF/A-2b** (archival): embedded sRGB ICC `OutputIntent` + XMP metadata, no transparency. Validates green under veraPDF `--flavour 2b`. |
| **`pdfUa: true`** | Emit **PDF/UA-1** (tagged / accessible): `StructTreeRoot` + marked content (headings/lists/tables, `<img alt>`, reading order) + `ToUnicode` maps for screen readers. Validates green under veraPDF `--flavour ua1`. |
| **`cmyk: true`** | Emit **DeviceCMYK** colour for a real press. Default is DeviceRGB (right for screen). |
| **`encrypt: { … }`** | **AES-256 password protection** — PDF 2.0 Standard Security Handler V5/R6 (AESV3). Takes user/owner passwords; output is intentionally **non-deterministic** when a password is set. |

```js
// archival + accessible + print colour:
program.render({ data, css, pdfA: true, pdfUa: true, cmyk: true })

// password-protected (AES-256):
program.render({
  data, css,
  encrypt: { userPassword: 'open-me', ownerPassword: 'owner-secret' },
})
```

> Off by default = the plain-RGB, untagged output is byte-identical across runs.
> Each toggle changes the bytes; encryption deliberately randomizes them.

---

## Features

- HTML + CSS subset (block / inline / flexbox / tables), automatic pagination.
- **Bundled default fonts** (Node + Python) — `sans-serif` / `serif` / `monospace`
  resolve with zero caller fonts (SIL OFL 1.1; see
  [`assets/fonts/NOTICE.md`](crates/turbo-pdf-core/assets/fonts/NOTICE.md)).
- Running headers & footers with per-page values; auto-numbered footnotes.
- Font subsetting + embedding (TrueType & CFF/OpenType); per-glyph fallback.
- Raster images (PNG/JPEG, alpha → SMask) with a sane max-size clamp; optional SVG
  via the `turbo-html2pdf-svg` build (resvg).
- **Internal links & cross-references**, **watermarks**, **append/merge**, and the
  **PDF/A · PDF/UA · CMYK · AES-256** per-render toggles (above).
- Deterministic output; `Send + Sync`; no network / no system fonts.

### Internal links & cross-references

Mark a spot in the document and link to it — clickable in the PDF (a `GoTo`
destination + a `Link` annotation):

```html
<t:anchor name="ch2"/>
...
<a href="#ch2">see Chapter 2</a>
```

### Append / merge

Glue an **existing external PDF after the rendered pages** — e.g. attach a
government-certified document (a CFDI/DSNE) so the rendered cover/body and the
certified PDF arrive as one file:

```js
program.render({
  data, css,
  append: existingPdfBytes,   // appended after the rendered pages
})
```

### Watermarks

A watermark is a **render option** — it paints behind the page body on every page.
Two kinds:

- **Text** — a faded, rotated word. Out-of-the-box `DRAFT` (gray, 25% opacity, 45°),
  or any word/colour/opacity/angle. The word is shaped + embedded from a bundled or
  caller-supplied font.
- **Image** — a raster image behind the body, centered or tiled, at a chosen opacity.

```js
// DRAFT stamp, via the render options:
program.render({ data, css, watermark: { text: 'DRAFT' } })  // gray, 25%, 45°

// fully custom text:
watermark: { text: 'CONFIDENTIAL', color: '#cc0000', opacity: 0.15, angle: 30 }

// image watermark (name resolved from the provided images), tiled & faint:
watermark: { image: 'logo', tiled: true, opacity: 0.08 }
```

In Rust the watermark lives on `EmitOptions.watermark`
(`Watermark::Text(TextWatermark)` / `Watermark::Image(ImageWatermark)`;
`TextWatermark::draft(face)` is the preset).

### Opt-in: SVG images

SVG vector images (`<img>` / `background-image`) are **off in the default build to
keep the download small** — SVG pulls in the [`resvg`](https://crates.io/crates/resvg)
rasterizer (a few MB). Everything else (PNG/JPEG, all layout) works without it.
**Need SVG? Install the SVG-enabled package** — same API, prebuilt with resvg, no
build step:

```bash
npm i turbo-html2pdf        # default — small, no SVG
npm i turbo-html2pdf-svg    # identical API, SVG support baked in (resvg)
```

(Rust users: enable the feature directly — `turbo-pdf-core = { features = ["svg"] }`.)

## Status

**`v0.1.3`** on npm and PyPI. The core engine is complete and heavily tested (the
`turbo-pdf-core` crate holds 100% line coverage with a cyclomatic-complexity ≤ 5
gate). Bindings: Node (N-API), WebAssembly (lean + fonts), and Python (PyO3). See
[`docs/`](docs/) for the full guide and
[`benches/competitive/`](benches/competitive/) for the benchmark harness.

## License

MIT (engine + bindings). Bundled fonts are SIL OFL 1.1 — see
[`assets/fonts/NOTICE.md`](crates/turbo-pdf-core/assets/fonts/NOTICE.md).
