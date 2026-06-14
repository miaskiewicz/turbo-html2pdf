# @turbo-pdf/napi

N-API binding that exposes the turbo-pdf template → PDF pipeline to Node/JS. This
is the "usable in a React project" entry point: compile a template once, render
it against data to a PDF `Buffer`.

## API

```js
const { compile, render, TurboPdfError } = require('@turbo-pdf/napi')
const fs = require('node:fs')

const font = fs.readFileSync('Go-Regular.ttf')

// Compile once, render many times. `Program` is a thread-safe native handle.
const program = compile('<h1>{{ title }}</h1><p>{{ body }}</p>')

const { pdf, diagnostics, pageCount } = program.render({
  data: { title: 'Hello', body: 'World' },
  css: '@page { size: A4; margin: 20px } h1 { font-size: 24px }',
  fonts: [font],          // raw OpenType/TrueType bytes, one Buffer per face
  meta: { title: 'My Doc', author: 'Me' },
  now: 0,                 // pin the now() clock for reproducible output
})
// pdf: Buffer (starts with %PDF-1.7), diagnostics: lint[], pageCount: number

fs.writeFileSync('out.pdf', pdf)

// One-shot convenience (compile + render in one call):
const r = render('<p>{{ x }}</p>', { data: { x: 1 }, fonts: [font] })
```

### `compile(templateHtml, opts?) -> Program`
Parses the template into a reusable `Program`. Throws `TurboPdfError` on a fatal
template-syntax fault. `opts` is reserved for future compile knobs and currently
ignored.

### `program.render(opts?) -> { pdf, diagnostics, pageCount }`
Renders the program against `opts`:

| field    | type       | notes |
|----------|------------|-------|
| `data`   | any        | interpolated into the body (`{{ field }}`, not `{{ data.field }}`) |
| `css`    | string     | author CSS; also supplies `@page` geometry (size/margins) |
| `fonts`  | Buffer[]   | font programs, registered via `FontFace::from_bytes` |
| `images` | Buffer[]   | accepted but **not yet embedded** (see Deferred) |
| `meta`   | DocMeta    | title/author/subject/keywords/creationDate for the PDF info dict |
| `now`    | number     | Unix seconds; pins the clock for byte-deterministic output |

### `render(templateHtml, opts?) -> { pdf, diagnostics, pageCount }`
One-shot convenience that compiles and renders in one call.

### Errors vs. diagnostics
* **Fatal** compile/render faults are thrown as `TurboPdfError`, carrying
  `.code` (e.g. `"TemplateSyntax"`, `"UndefinedValue"`) and `.span`
  (`{ line, col, byteOffset }`).
* **Non-fatal** lints (e.g. `"NotdefGlyph"`, `"RegionOverflow"`,
  `"UnsupportedCss"`) are **returned** in `result.diagnostics`, never thrown.

## Building the addon

With the napi CLI (preferred, produces a platform `.node` next to `index.js`):

```sh
npm install            # installs @napi-rs/cli
npm run build          # napi build --platform --release
```

Without the napi CLI (plain cargo + copy step):

```sh
npm run build:cargo    # cargo build -p turbo-pdf-napi --release && copy-addon
```

`index.js` also falls back to `target/{release,debug}` automatically, so a bare
`cargo build -p turbo-pdf-napi` is enough for local use.

## Tests

```sh
npm test               # node --test __test__/*.test.mjs
```

The e2e test compiles a real template + data, renders a PDF, asserts the page
count and `%PDF` magic, and — when `qpdf` is on `PATH` — asserts `qpdf --check`
is clean. If the addon is not built the suite **skips** (it never red-bars on a
missing toolchain).

## Deferred

* **`toBytes()` / `fromBytes()` for `Program`** — the core `Program` is not
  serializable today (it holds a live MiniJinja `Environment`). Deferred to
  `phase10b`; omitted from the surface rather than shipped as a throwing stub.
* **Raster images** — the `images` param is accepted and wired through but is a
  no-op; raster image embedding is Phase 9b.
