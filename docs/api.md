# JS / React / WASM API

turbo-html2pdf ships four JS-facing surfaces:

- **`turbo-html2pdf`** — the Node native binding (`compile`/`render` → PDF
  `Buffer`).
- **`turbo-html2pdf-wasm`** — the WebAssembly binding (same pipeline in the browser).
- **`turbo-html2pdf-react`** — author templates as React components.
- **`turbo-html2pdf-template`** — author templates with plain functions (no React).

The model is the same everywhere: a **template** (markup + Jinja +
[`t:` directives](paged-media.md)) is compiled once into a `Program`, then
rendered against **data** + **CSS** + **fonts** to a PDF. The two frontends
(`react`, `template`) only *produce the template source string*; the `napi`/`wasm`
bindings *run it*.

> **Warm-start note.** Two reusable handles: `Program` (**compile once, render
> many**) and `Fonts` (**parse fonts once, reuse across renders**). Build both at
> server startup and reuse them per request — fonts are then never re-parsed.
> `Fonts.load(buffers)` returns the handle; pass it as the second argument to
> `program.render(opts, fonts)` (NAPI) or to `program.renderWithFonts(args, fonts)`
> (WASM). Omit it to fall back to per-call `RenderOptions.fonts`.

---

## 1. `turbo-html2pdf` (Node)

```js
const { compile, render, TurboPdfError } = require('turbo-html2pdf')
const fs = require('node:fs')

const { Fonts } = require('turbo-html2pdf')
const font = fs.readFileSync('Go-Regular.ttf')

// Warm both handles ONCE at startup, reuse them per request.
const program = compile('<h1>{{ title }}</h1><p>{{ body }}</p>')
const fonts = Fonts.load([font])   // parse fonts once

const { pdf, diagnostics, pageCount } = program.render({
  data: { title: 'Hello', body: 'World' },
  css: '@page { size: A4; margin: 20px } h1 { font-size: 24px }',
  meta: { title: 'My Doc', author: 'Me' },
  now: 0,                 // pin the now() clock for reproducible output
}, fonts)                 // <- reuse the prebuilt font handle

fs.writeFileSync('out.pdf', pdf)   // pdf starts with %PDF-1.7
```

### Surface (`crates/turbo-pdf-napi/index.d.ts`)

```ts
function compile(templateHtml: string, opts?: unknown): Program  // opts reserved, currently ignored
function render(templateHtml: string, opts?: RenderOptions, fonts?: Fonts): RenderResult  // one-shot

class Fonts {
  static load(fonts: Buffer[]): Fonts   // parse fonts once; reuse across renders
}

interface Program {
  render(opts?: RenderOptions, fonts?: Fonts): RenderResult   // throws TurboPdfError on a fatal fault
  hasHeader(): boolean   // source declared <t:running-header>
  hasFooter(): boolean   // source declared <t:running-footer>
}

interface RenderOptions {
  data?: unknown      // interpolated into the body
  css?: string        // author CSS; also supplies @page geometry
  fonts?: Buffer[]    // raw OpenType/TrueType bytes, one Buffer per face
  images?: Buffer[]   // accepted but NOT yet embedded (Phase 9b no-op)
  meta?: DocMeta
  now?: number        // pins now() (Unix seconds) for determinism
}

interface RenderResult {
  pdf: Buffer
  diagnostics: Diagnostic[]   // non-fatal lints, never thrown
  pageCount: number
}

interface DocMeta {
  title?: string; author?: string; subject?: string; keywords?: string
  creationDate?: number   // Unix seconds; omit for the reproducible sentinel date
}

interface Diagnostic { code: string; message: string; line: number; col: number }

class TurboPdfError extends Error {
  name: 'TurboPdfError'
  code: string    // e.g. "TemplateSyntax", "UndefinedValue"
  span: { line: number; col: number; byteOffset: number }
}
```

**Data access:** body data is referenced at the top level — `{{ title }}`, not
`{{ data.title }}`. (Inside a running header/footer region it is nested under
`data`; see [paged-media.md](paged-media.md#the-per-page-context).)

**Errors vs diagnostics:** fatal compile/render faults **throw** `TurboPdfError`
(with `.code` and `.span`); non-fatal lints (`"NotdefGlyph"`, `"RegionOverflow"`,
`"UnsupportedCss"`, …) come back in `result.diagnostics` and are never thrown.

> `Program.toBytes()` / `fromBytes()` (serialize a compiled program) are deferred
> — not on the current surface.

---

## 2. `turbo-html2pdf-wasm` (browser)

Same pipeline, async-initialized. The JS API is `wasm-bindgen`-generated from the
Rust `#[wasm_bindgen]` exports (`crates/turbo-pdf-wasm/src/`):

```ts
function init(): void          // optional async initializer (installs the panic hook seam)
function compile(templateHtml: string, opts?: JsCompileOptions): Program

class Fonts {
  static load(fonts: JsFont[]): Fonts   // parse fonts once; reuse across renders
}

interface Program {
  render(args?: JsRenderArgs): RenderOutcome                    // inline fonts from args
  renderWithFonts(args: JsRenderArgs, fonts: Fonts): RenderOutcome  // reuse a prebuilt handle
  hasHeader(): boolean
  hasFooter(): boolean
}
```

```ts
interface JsCompileOptions {
  partials?: Record<string, string>
  missingPolicy?: 'empty' | 'lenient' | 'strict'   // unknown → strict
  includeMaxDepth?: number
}

interface JsRenderArgs {
  data?: unknown
  css?: string
  fonts?: Array<{                 // richer than NAPI: objects, not bare bytes
    data: Uint8Array              // .ttf / .otf bytes
    family: string
    weight?: number               // default 400
    italic?: boolean              // default false
  }>
  images?: Uint8Array[]           // accepted but NOT embedded (Phase 9b no-op)
  meta?: DocMeta                  // creationDate = Unix seconds
  now?: number                    // Unix seconds; omit → sentinel date 2000-01-01T00:00:00Z
}

interface RenderOutcome {
  pdf: Uint8Array
  diagnostics: Array<{ code: string; message: string; span: { line: number; col: number; byteOffset: number } }>
  pageCount: number
}
```

### Differences from NAPI

- **Fonts:** WASM fonts are objects `{ data, family, weight?, italic? }`; NAPI
  fonts are bare `Buffer[]` with no metadata.
- **Warm-start handle:** both expose a `Fonts` handle (`Fonts.load(...)`), but
  NAPI threads it as the 2nd arg to `render(opts, fonts)` while WASM uses a
  dedicated `program.renderWithFonts(args, fonts)`.
- **Bytes:** WASM uses `Uint8Array`; NAPI uses `Buffer`.
- **Compile options:** WASM `compile` honors `partials` / `missingPolicy` /
  `includeMaxDepth`; NAPI `compile` opts are reserved/ignored.
- **Errors:** WASM **rejects** with a structured `{ code, message, span }`; NAPI
  **throws** a typed `TurboPdfError`.
- WASM has `init()`; NAPI has no initializer.
- WASM has **no** one-shot `render(templateHtml, opts)` — use
  `compile(...).render(...)`. NAPI has both.
- WASM defaults the clock to the sentinel date `2000-01-01T00:00:00Z` when `now`
  is omitted; NAPI passes `now` straight through.

The warm-start pattern is the same: `const p = compile(tpl); p.render({...})`
repeatedly.

---

## 3. `turbo-html2pdf-react`

Author the template as React components. The components render **once at
authoring time** (via `renderToStaticMarkup`) into a turbo-html2pdf template *source
string* you then hand to `compile`. React is never on the render hot path; the
attribute values are **expression strings** resolved later in Rust, not evaluated
JS.

```ts
import { compileTemplate } from 'turbo-html2pdf-react'

const source = compileTemplate(<InvoiceDoc />)   // string: HTML + Jinja + t: directives
// → hand `source` to turbo-html2pdf or /wasm `compile(...)`
```

`compileTemplate(element, { trim?: boolean })` renders the element and trims by
default (`trim: false` to keep surrounding whitespace).

### Control-flow components → Jinja

| Component | Props | Emits |
|---|---|---|
| `If` | `cond: string` | `{% if COND %}…{% endif %}` |
| `ElseIf` | `cond: string` | `{% elif COND %}…` |
| `Else` | — | `{% else %}…` |
| `Each` | `of: string; as: string; index?: string` | `{% for AS in OF %}…{% endfor %}` (`index` adds `{% set INDEX = loop.index0 %}`) |
| `Switch` | `on: string` | `{% switch ON %}…{% endswitch %}` |
| `Case` | `value: string` | `{% case VALUE %}…` (comma in `value` = membership) |
| `Default` | — | `{% default %}…` |
| `Include` | `src: string; with?: string` | `{% include SRC [with CTX] %}` |
| `Expr` | `value: string` | `{{ VALUE }}` |
| `Raw` | `html: string` | the raw string verbatim (escape hatch for `class`/`style`/`t:style`) |

### Paged-media components → `t:` directives

| Component | Props | Emits |
|---|---|---|
| `RunningHeader` | `extent?` | `<t:running-header>…</t:running-header>` |
| `RunningFooter` | `extent?` | `<t:running-footer>…</t:running-footer>` |
| `Footnote` | `mark?; reset?` | `<t:footnote mark=… t:footnote-reset=…>…</t:footnote>` |
| `FootnoteSeparator` | — | `<t:footnote-separator>…</t:footnote-separator>` |
| `Page` | — | `<t:page>` (current page number) |
| `Pages` | — | `<t:pages>` (total page count) |
| `Leader` | — | `<t:leader>…</t:leader>` |
| `PageMaster` | `name; size?; orientation?; margin?` | `<t:page-master …>…</t:page-master>` |
| `Region` | `slot; extent?` | `<t:region …>…</t:region>` |
| `Variant` | `kind` | `<t:variant kind=…>…</t:variant>` |
| `UseMaster` | `name` | `<t:use-master name=…>` |
| `Counter` | `name; action?; step?; start?` | `<t:counter …>` |
| `Anchor` | `id` | `<t:anchor id=…>` |
| `Endnote` | — | `<t:endnote>…</t:endnote>` |
| `Endnotes` | — | `<t:endnotes>` |
| `Running` | `name; as?; policy?` | a host element (default `<span>`) with a `t:running="NAME"` **attribute** (this one emits an attribute, not a `t:` element) |

These emit the markup 1:1 — but remember that only the directives marked
**implemented** in [paged-media.md](paged-media.md#the-t-directives) actually do
anything at render time. `Footnote`, `RunningHeader`/`RunningFooter`, and
`Page`/`Pages` are the implemented set; `PageMaster`, `Region`, `Variant`,
`UseMaster`, `Counter`, `Leader`, `Anchor`, `Endnote`, `Endnotes` are deferred.

---

## 4. `turbo-html2pdf-template`

A framework-free authoring frontend that emits the **same** template source
string as the React frontend (byte-identical to `renderToStaticMarkup` output),
with no React dependency. Everything is a plain function taking expression
strings (`packages/template/src/index.ts`):

```ts
// control flow → Jinja
ifBlock(cond, ...children)        // {% if %}…{% endif %}
elseIf(cond, ...children)         // {% elif %}…
elseBlock(...children)            // {% else %}…
each(of, as, ...children)         // {% for as in of %}…{% endfor %}
switchBlock(on, ...children)      // {% switch %}…{% endswitch %}
caseBlock(value, ...children)     // {% case %}…
defaultBlock(...children)         // {% default %}…
include(src, withCtx?)            // {% include … [with …] %}
expr(value)                       // {{ value }}

// paged media → t: directives
runningHeader({ extent? }, ...children)
runningFooter({ extent? }, ...children)
footnote({ mark?, reset? }, ...children)   // reset → t:footnote-reset attr
page()                            // <t:page></t:page>
pages()                           // <t:pages></t:pages>
leader()
pageMaster({ name, size?, orientation?, margin? }, ...children)
region({ slot, extent? }, ...children)
variant(kind, ...children)
useMaster(name)
counter({ name, action?, step?, start? })
running({ name, tag?, policy? }, ...children)   // t:running attr on host (default span)

// assembly
compileTemplate(...children)      // join + trim → the source string
```

Note: `running` here takes `tag?` (the React component takes `as?`). Author plain
text is *not* auto-escaped in this frontend, so you escape it yourself.
