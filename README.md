# turbo-html2pdf

**Turn HTML + CSS (with a Jinja templating layer) into a PDF — natively, in Rust,
with no headless browser.** A from-scratch document engine: templating → HTML/CSS
layout → automatic pagination → PDF.

The incumbent way to make a PDF from HTML is to drive headless Chrome
(Puppeteer/Playwright/Gotenberg) — ship ~200 MB of Chromium, spin up a process per
render, host a service. **turbo-html2pdf is just a binary.** It *is* the layout +
PDF engine, so generating a PDF is a single library call — no Chromium to download,
no service to run, no 200 ms spin-up, deterministic output, and **tens to hundreds
of times faster** in the last measured run ([benchmark history](docs/BENCHMARKS-history.md)). It renders with **zero caller
fonts** too, because the default sans/serif/mono fonts ship in the build.

**Then the kicker: the same engine also runs *inside* a web browser.** Because it
compiles to WebAssembly (~3 MB), you can generate PDFs 100% client-side — no
server, no backend ([details](#-bonus-the-same-engine-runs-inside-a-web-browser)).

> ⚠️ "Browser" means two different things here, and that's the whole trick:
> turbo-html2pdf needs **no headless *Chromium*** to *generate* a PDF (it does the
> layout itself), *and* it can run **inside a *web browser*** as a WASM module.
> Different senses of the word — the first is what we replace, the second is where
> we also run.

Ships as a tiny native addon for **Node**, a **WASM** build for the **browser**,
and a **Python** wheel — one engine, every runtime.

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
| **`turbo-html2pdf-core`** | crates.io | The **pure-Rust** HTML/CSS + Jinja → PDF engine, for in-process Rust consumers. Re-exports the Jinja-free `layout_html` drive (raw HTML → positioned `Fragment` galley) of its layout engine, [`turbo-dom-layout`](https://github.com/miaskiewicz/turbo-dom/tree/main/crates/turbo-dom-layout), for consumers that only want the layout + font engine. | `cargo add turbo-html2pdf-core` |
| **`turbo-html2pdf-mcp`** | crates.io | Native **MCP server** (stdio JSON-RPC 2.0) exposing `render` / `append_pdf` / `check_template` to agents. Installable binary, or prebuilt archives on each GitHub Release. | `cargo install turbo-html2pdf-mcp` |

The two authoring packages (`-react`, `-template`) only *produce the template
string* — they pair with a render package (`turbo-html2pdf` on Node, a
`turbo-html2pdf-wasm*` build in the browser, or the PyPI wheel) to actually emit a
PDF. The Rust engine lives in
[`crates/turbo-html2pdf-core`](https://github.com/miaskiewicz/turbo-html2pdf/tree/main/crates/turbo-html2pdf-core).

> Release version: **`v0.6.0`**. See [CHANGELOG.md](./CHANGELOG.md) and
> [publishing and registry verification](./PUBLISHING.md).

## 🌐 Bonus: the same engine runs *inside* a web browser

Everything above is the server/CLI story — a binary that makes PDFs with no
Chromium. Here's the kicker: because that engine is pure Rust → WebAssembly
(~3 MB), the **identical** code also runs **100% client-side in a web-browser tab**
— no server, no backend, no Chromium. A user pastes HTML/CSS and gets PDF bytes
without anything leaving the page.

> Note the two senses of "browser": the server path needs **no headless Chromium**
> to *render*; this WASM path *runs in* a **web browser**. Same engine, both worlds.

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

### Bun single-file executables (`bun build --compile`)

Use a **WASM** package here, **not** the Node/N-API `turbo-html2pdf`. A Bun
standalone executable can only embed assets it can see statically, and it cannot
carry the N-API loader's dynamically-resolved native `.node` addon — the compiled
binary bakes in the build machine's absolute paths and fails on any other host
with `native addon not found`. The `.wasm`, by contrast, embeds cleanly with an
`import ... with { type: 'file' }`:

```ts
import initPdf, { compile } from 'turbo-html2pdf-wasm-fonts'
import wasm from 'turbo-html2pdf-wasm-fonts/turbo_pdf_wasm_bg.wasm' with { type: 'file' }

// Load the bytes yourself — a URL/path won't resolve inside the compiled binary.
const bytes = await Bun.file(wasm).bytes()
await initPdf({ module_or_path: bytes })   // single-object form; positional initPdf(bytes) is deprecated

const program = compile('<h1>{{ t }}</h1>')
const { pdf } = program.render({ data: { t: 'Hi' },
                                 css: 'h1{font-family:sans-serif;font-size:24pt}' })
```

Pass the wasm bytes to `initPdf` as `{ module_or_path: bytes }` — the bare
positional `initPdf(bytes)` still works but logs a wasm-bindgen deprecation
warning.

---

## Why it's fast

No headless Chromium means no ~200 MB browser to ship, no per-process spin-up, and
a small memory footprint. turbo-html2pdf is a native layout + PDF engine — a library
call, not an IPC round-trip to Chrome — so the common "render one document" path is
**tens to hundreds of times faster** (last measured before 0.6.0; see below).

### Benchmarks — pending re-run

0.6.0 has not been re-benchmarked. The layout engine moved into the shared turbo-dom
crates in this release, so the earlier numbers no longer describe the shipped code; a
re-run of all four turbo-* repos together is planned after the release. The last
measured (pre-consolidation, 0.1–0.2) numbers, with their machine and date, are kept in
[`docs/BENCHMARKS-history.md`](docs/BENCHMARKS-history.md). Harness:
[`benches/competitive/`](benches/competitive/) (`pnpm bench`).

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
the font assets is in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) (generated by
`scripts/package-notices.mjs` from turbo-dom-layout's `assets/fonts/NOTICE.md`, where the
faces live); keep it alongside the binary when you redistribute.

You can still pass your own fonts (via `Fonts.load(...)` on Node or the per-render
`fonts` argument) — caller fonts win, the bundled ones are the fallback. The browser
split is the exception: **`turbo-html2pdf-wasm` ships lean (no fonts)** and the
caller supplies font bytes, while **`turbo-html2pdf-wasm-fonts`** embeds the same
default set in the `.wasm`.

---

## Architecture

A template is **compiled once** into a reusable `Program`, then rendered against
data many times. Since 0.6.0 the engine is built on the shared
[turbo-dom](https://github.com/miaskiewicz/turbo-dom) crates — the same DOM, selector
engine and layout code turbo-test and turbo-surf use — instead of carrying its own copies:

- **`turbo-dom`** — the document tree the rendered markup is parsed into, and the one
  selector engine (`parse_for_cascade` / `match_for_cascade`) the cascade matches with.
- **`turbo-dom-layout`** — style cascade, block / inline / flex / table layout, text
  shaping and fonts (the bundled faces live there), images, SVG and layout snapshots.
  `turbo-html2pdf-core` re-exports it at the old paths (`turbo_html2pdf_core::style::…`,
  `::layout::…`, `::Fragment`).
- **`turbo-html2pdf-core`** (this repo) — templating, `t:` directives, pagination,
  PDF/UA semantics and the PDF emitter.

```
template (HTML + Jinja + t: directives)
   │  compile()                      ── parse + cache (MiniJinja)
   ▼
render_pages(data, css, fonts):
   │  1. render markup               ── Jinja → HTML string (data substituted)
   │  2. parse                       ── turbo-dom document, t: directives as markers
   │  3. cascade + inherit           ── turbo-dom-layout → ComputedStyle per node
   │  4. layout                      ── turbo-dom-layout: block / inline / flex (taffy) /
   │                                    table → a "galley" of positioned, shaped fragments
   │  5. paginate                    ── break the galley into pages (orphans/widows,
   │                                    break rules, repeated <thead>); resolve
   │                                    running headers/footers + footnotes per page
   ▼  emit_pdf()                     ── fragments → PDF: subset + embed fonts
                                        (TrueType / Type0-CFF), text, vector boxes/
   PDF bytes                            borders, raster images, links, watermark
```

Font shaping (`rustybuzz`), PDF writing (`pdf-writer` + `subsetter`) and layout are
all native; `taffy` powers flexbox. The engine does no network or filesystem I/O —
images are supplied by the caller, and fonts are either the bundled defaults or the
ones you pass. Pagination is checked against Chrome 156 `page.pdf()` controls
(`scripts/pagination-chrome-control.mjs`).

**Frontends & bindings** (`packages/`, `crates/`):
- `turbo-html2pdf` — Node native (N-API) addon (`compile`/`render` → `Buffer`).
- `turbo-html2pdf-svg` — the same Node engine built with the `svg` feature (resvg).
- `turbo-html2pdf-wasm` / `-wasm-fonts` — the engine in the browser (WebAssembly),
  lean or with the default fonts embedded.
- `turbo-html2pdf-react` — author templates as React components (compiled to the
  template string at authoring time, never on the render path).
- `turbo-html2pdf-template` — author templates with plain functions (no React).
- `turbo-html2pdf-mcp` — native MCP server (stdio JSON-RPC) exposing `render` /
  `append_pdf` / `check_template` to agents (`cargo install turbo-html2pdf-mcp`).

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
  [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)).
- Running headers & footers with per-page values; auto-numbered footnotes.
- Font subsetting + embedding (TrueType & CFF/OpenType); per-glyph fallback.
- Raster images (PNG/JPEG, alpha → SMask) with a sane max-size clamp; optional SVG
  via the `turbo-html2pdf-svg` build (resvg).
- **Internal links & cross-references**, **watermarks**, **append/merge**,
  **stamp** (post-emit watermark on an existing PDF), and the
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

As in Chrome, an element `id` is a destination too (`<h2 id="ch2">` works without a
`<t:anchor>`), and an absolute `href` (`https://…`, `mailto:…`) becomes a `URI` link.
A relative `href` has no base URL and carries no link. Known gaps (turbo-dom#210): a
`display:block` internal link's rectangle covers only its text, and an *inline*
external link gets no annotation (block-level external links do).

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

### Stamp: watermark an existing PDF

`watermark` (above) only paints while pages are freshly emitted. `stamp` is the
post-emit counterpart: it overlays a faded, rotated text mark onto **every page
of a PDF you already have** — e.g. a document you `append`ed a certified PDF
onto, or any foreign PDF — without touching the original content streams. It can
also open a password-protected input and hand back a still-protected result.

```js
const { stamp } = require('turbo-html2pdf')

const stamped = stamp(existingPdfBytes, {
  watermark: { text: 'CANCELLED' },   // gray, 15% opacity, 45°, bundled sans-serif (embedded)
})

// fully custom mark, plus a decrypt/re-encrypt round trip:
stamp(existingPdfBytes, {
  watermark: { text: 'VOID', color: '#cc0000', opacity: 0.2, angle: 30, fontSize: 72 },
  password: 'open-me',                                       // decrypts the input first
  encryption: { userPassword: 'new-pw', ownerPassword: 'owner-secret' }, // re-seals the output
})
```

| Field | What it does |
|---|---|
| **`watermark.text`** | The word to stamp. Required. |
| **`watermark.color`** | Fill color `#rrggbb`. Defaults to gray. |
| **`watermark.opacity`** | Fill opacity `0.0..=1.0`. Defaults to `0.15`. |
| **`watermark.angle`** | Rotation in degrees. Defaults to `45`. |
| **`watermark.fontSize`** | Font size in CSS px. Defaults to `64`. |
| **`watermark.font`** | Font (TrueType/OTF bytes, a `Buffer`) to shape and embed the text with. Omit for the bundled sans-serif. Either way the font is embedded, so the output is self-contained and never relies on a base-14 font the reader may lack. |
| **`password`** | Opens an encrypted `pdf` input first. Omit for a plaintext input. |
| **`encryption`** | Re-encrypts the stamped output — same shape as `render`'s `encrypt` above. Omit for a plaintext output. |

Throws `TurboPdfError` if `pdf` doesn't parse or has no pages, or if `password`/
`encryption` fail to open/re-seal it.

Unlike the render-time mark (which may be text *or* image), `stamp` is text-only —
no image form. Like the render mark, its text is shaped and embedded from a real
font (the bundled sans-serif by default, or a caller-supplied `watermark.font`), so
the output is self-contained and never relies on a base-14 font the reader may lack.
In Rust it's `stamp(pdf, &StampWatermark, password, encryption)` behind the `stamp`
feature (`crates/turbo-html2pdf-core/src/stamp.rs`).

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

(Rust users: enable the feature directly — `turbo-html2pdf-core = { features = ["svg"] }`.)

## Build, test and gates

```bash
cargo test --workspace            # engine + bindings' Rust tests
pnpm install && pnpm lint         # oxlint + biome for the JS/TS packages
```

PR CI (`.github/workflows/ci.yml`) runs on ubuntu: rustfmt, clippy `-D warnings`, the
workspace tests, the core under `--features svg`, `--all-features` (perf budgets) and
`--no-default-features` (the font-less wasm build), a cyclomatic-complexity gate
(`tools/cc-check`, max 5, blocking), and **100% line coverage** of the covered crates
(`cargo tarpaulin`; excludes and `fail-under = 100.0` in `tarpaulin.toml` — the napi, py,
wasm and MCP shims are excluded). Binding conformance (napi + wasm + Python) and the JS
lint run as separate jobs. macOS and Windows builds run only in the release workflows
(`release.yml`, `release-py.yml`).

## Status

**`v0.6.0`** is the release version for npm, PyPI and crates.io; the
[release procedure](./PUBLISHING.md) records the tag triggers and registry checks.
Until turbo-dom 0.6.0 is on crates.io the workspace pins the turbo-dom crates by git
revision (`[patch.crates-io]` in `Cargo.toml`); the release guard refuses a tag while
that patch remains. Bindings: Node (N-API), WebAssembly (lean + fonts), Python (PyO3),
MCP. See [`docs/`](docs/) for the full guide and [`CHANGELOG.md`](CHANGELOG.md) for
what changed in 0.6.0.

## License

MIT (engine + bindings). Bundled fonts are SIL OFL 1.1 — see
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).

### Native live-tree layout snapshots

The core's additive `layout_snapshot` API preserves caller-owned source IDs across
all native layout fragments and composes nested transforms. Its continuous-media
path retains nonpainting boxes and sizes replaced images without PDF page-fit
caps. PDF filtering, page-fit and legacy layout policies are preserved; shared CSS
cascade fixes also improve valid PDF output. see [the API contract, Chrome controls and
remaining geometry boundaries](docs/live-layout-snapshot.md).
