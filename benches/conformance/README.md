# Layout-conformance harness (slice 1)

Catches **foundational layout bugs** by comparing the engine's laid-out element
geometry against Chromium's, one standard HTML/CSS feature at a time.

For each fixture the harness lays the page out two ways and compares the
**border-box rectangle** of every element tagged `data-cid="..."`:

- **Candidate** — `turbo-html2pdf-core`, via the napi `layoutBoxes` debug export,
  which walks the laid-out `Fragment` galley and dumps `{cid, x, y, width, height}`.
- **Oracle** — Chromium, via Playwright: `page.setContent(html)` then
  `getBoundingClientRect()` for each `[data-cid]`.

Because it tests the shared `turbo-html2pdf-core` layout engine, it also governs
**turbo-surf's raster** (screenshots) and every PDF the engine emits — the same
box tree feeds all of them.

## Box geometry, not pixels

This compares **laid-out box coordinates** (x/y/width/height in px), not rendered
pixels. That is deliberate for a *layout* conformance gate:

- It isolates layout math (placement, sizing, flow) from paint concerns
  (anti-aliasing, subpixel hinting, gamma) that a pixel diff conflates.
- Failures are **legible**: you get the exact px delta on the exact axis for the
  exact element, not a heatmap. `Δ = max(|dx|, |dy|, |dw|, |dh|)`.
- It is deterministic and fast — no image decode, no per-pixel threshold tuning.

A box that matches Chromium's rect to within tolerance is, by construction, in
the right place at the right size; pixel fidelity on top of that is a separate
(raster) concern.

## Font pinning (why, and how)

Text-metric-driven boxes — line-box heights, wrap points, intrinsic widths — only
match if **both sides shape with the same font**. The engine renders with its
bundled default face (`font-family: sans-serif` → **Inter**, from
`turbo-html2pdf-core/assets/fonts/inter`).

So the harness injects, into every fixture on **both** sides, a `<style>` that:

1. Declares `@font-face { font-family: 'Inter'; src: url(data:font/otf;base64,…) }`
   for Regular (400) and Bold (700), base64-embedding **the engine's own Inter
   OTF files**. Chromium then shapes with the identical face.
2. Forces `* { font-family: 'Inter' }`. The engine resolves family `Inter`
   straight to its bundled face (its registry registers bundled faces under their
   real family name) and **ignores** the `@font-face` data URL (its font set is
   fixed, not fed from author CSS) — so the same injected style pins Chromium and
   is a harmless no-op for the engine.
3. Zeroes `html,body` margin so the two coordinate origins line up. Per-element UA
   defaults (heading/list margins) are left intact — those are under test.

Styles live in the fixture `<body>`, because the engine collects `<body>` styles
but drops `<head>` ones.

## Running

```sh
# 1. Build the napi addon so `layoutBoxes` is current (needs a Rust toolchain).
pnpm build:addon        # cargo build -p turbo-pdf-napi --release + refresh .node

# 2. Run the harness (needs Chromium via Playwright).
pnpm conformance
```

Flags: `--width 800 --height 600 --tolerance 2 --fixture <substr>`.

**Chromium is the oracle and may be absent.** If Playwright or a launchable
Chromium isn't available, the harness prints `SKIPPED: no chromium` and **exits
0** — a missing browser never reds the build. A missing/stale napi addon is a
hard error (exit 1); build it with `pnpm build:addon`. When Chromium is present
the harness exits nonzero if any box is out of tolerance.

Install the oracle if needed:

```sh
pnpm -w add -D playwright        # or rely on the competitive bench's install
npx playwright install chromium
```

## Output

A per-fixture table plus a summary:

```
14-nested-flex.html  [4/4 within tolerance]
  cid       engine (x,y,w,h)        chromium (x,y,w,h)      Δ       result
  body      (0, 60, 200, 140)       (0, 60, 200, 140)       0px     PASS
  inner     (75, 105, 50, 50)       (75, 105, 50, 50)       0px     PASS
  ...
summary: 104/108 boxes within tolerance across 30 fixtures (29 fixtures fully clean)
```

## Fixtures

**Foundational-first**, then progressively adversarial combinations. Each fixture
is minimal, self-contained, deterministic, and tags the element(s) under test
with `data-cid`. Current status: **104/108 boxes within 2px across 30 fixtures
(29/30 fully clean)** — the lone exception is border-collapse merging (#29 below).

| #  | fixture                       | feature under test                                    |
| -- | ----------------------------- | ----------------------------------------------------- |
| 01 | `01-block-flow`               | block flow + vertical margins                         |
| 02 | `02-margin-collapse`          | adjacent-sibling margin collapsing                    |
| 03 | `03-box-sizing`               | box model: `content-box` vs `border-box`              |
| 04 | `04-inline-line-height`       | inline flow, `line-height`, inline-block sizing       |
| 05 | `05-heading-margins`          | UA default h1–h3 margins / sizes                      |
| 06 | `06-list-indent`              | UA default `ul`/`ol` indent + margins                 |
| 07 | `07-table`                    | 2×2 table, `border-collapse`, padded cells            |
| 08 | `08-flex-space-between`       | flex row, `justify-content: space-between`            |
| 09 | `09-abs-relative`             | `position:absolute` offset from a relative parent     |
| 10 | `10-width-auto-center`        | `width:50%` + `margin:auto` centering                 |
| 11 | `11-float-text-wrap`          | text wrapping around a `float:left` block             |
| 12 | `12-float-left-right-clear`   | opposing floats + `clear:both`                        |
| 13 | `13-inline-block-wrap`        | inline-block wrap + `vertical-align` + whitespace     |
| 14 | `14-nested-flex`              | nested flex, `flex-basis`/`flex:1` main-axis sizing   |
| 15 | `15-abs-in-flex-relative`     | absolute (`top`/`right`) badge on a relative flex item |
| 16 | `16-position-fixed`           | `position:fixed` vs the viewport                      |
| 17 | `17-combo-stage`              | flex + float + inline-block + absolute in conjunction |
| 18 | `18-flex-align-items`         | flex `align-items`/`align-self` (stretch/center/end)  |
| 19 | `19-negative-margin`          | negative vertical + horizontal margins                |
| 20 | `20-min-max-width`            | `min-width`/`max-width` clamping                      |
| 21 | `21-parent-child-margin-collapse` | parent/child top-margin collapse-through          |
| 22 | `22-nested-percent-width`     | nested `%` widths + content-box padding               |
| 23 | `23-flex-wrap`                | `flex-wrap` onto multiple lines                       |
| 24 | `24-inline-block-baseline`    | text + inline-block baseline alignment                |
| 25 | `25-abs-percentage`           | absolute `%` insets + `%` size vs the CB              |
| 26 | `26-margin-collapse-chain`    | margin collapse through nested wrappers               |
| 27 | `27-flex-column-grow`         | flex column, multiple `flex-grow` ratios              |
| 28 | `28-overflow-bfc-float`       | `overflow:hidden` BFC contains a float                |
| 29 | `29-table-valign-colspan`     | table `colspan` + collapsed-border widths             |
| 30 | `30-relative-offset`          | `position:relative` paint shift (flow preserved)      |

### Bugs this harness found and drove fixes for

Building these fixtures surfaced — and the engine was then fixed for — several
foundational layout bugs (all now within 2px of Chromium):

- **UA defaults (05/06):** added browser-standard `h1`–`h6`/`p` margins and
  `ul`/`ol` margins + `padding-left:40px` to the UA stylesheet.
- **Negative margins (19):** block flow now collapses margins per CSS 2.1
  (largest positive + most-negative), so a `margin-top:-20px` pulls up.
- **Inline-block placement (13/17/24):** atoms honour `vertical-align`
  (top/bottom/middle/baseline), reserve inter-atom whitespace, and apply their own
  margins in the line box.
- **Flex sizing (14/18/27):** taffy's resolved item height is applied on read-back
  and the `flex` shorthand's `<basis>` is parsed, so column `flex-basis`/`grow` and
  cross-axis `stretch` size correctly; a nested flex container inherits its
  parent-assigned definite height so `align-items:center` centres properly.
- **Floats + absolute (17):** a float's own margins offset its position and are
  registered so later content clears them; an absolute `bottom` inset resolves
  against a definite-height positioned ancestor.
- **Parent/child margin collapse (21/26):** a first child's top margin collapses
  through a borderless/paddingless parent (recursively), while the document root's
  margins never collapse (matching browsers).

### Known remaining gap

- **29 — full `border-collapse` border merging.** Adjacent collapsed cell borders
  are not merged (each cell keeps its full borders), so a colspanned collapsed
  table is ~4–5px wider than Chromium and cells are ~2px off. This is a documented
  v1 engine deferral (`table.rs`): merging requires the full collapsed-border grid
  model and matching Chromium's subpixel collapsed-border box reporting. Non-colspan
  collapsed tables (07) already land within tolerance.

## Adding a fixture

1. Drop `fixtures/NN-name.html` — a full, self-contained document. Put the feature
   CSS in a `<body>` `<style>` (the engine ignores `<head>` styles). Tag each
   element under test with `data-cid="..."`.
2. Tag only **box-generating** elements — block, `inline-block`, flex/grid/table
   items, replaced `<img>`. A `display:inline` element is flattened during box
   generation and drops its `data-cid` (its metrics have no single box). To probe
   inline text, tag its block container or wrap it in an `inline-block`.
3. Do **not** set `font-family` in the fixture — the harness pins Inter for you.
4. `pnpm conformance --fixture NN` to iterate on just yours.

## Roadmap (later slices)

Slice 1 is foundational + first combinations. Natural next fixtures: multi-line
float shapes and `shape-outside`, nested/collapsed table borders, `grid`
track sizing, `overflow`/scroll containers, `transform` + `transform-origin`,
`z-index` stacking contexts, writing-mode/`direction`, `min`/`max-width` clamping
under flex, sticky positioning, and `object-fit` on replaced content.
