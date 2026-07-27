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
summary: 186/186 boxes within tolerance across 64 gated fixtures (64 fixtures fully clean)
deferred (tracked, not gated): 4 fixture(s)
  - 36-grid-min-content-row.html [0/3] — …
```

## Fixtures

**Foundational-first** (`01`–`30`), then a **modern-CSS corpus expansion** (`31`–`68`)
covering the constructs real pages (nike / google / wikipedia) exercise. Each fixture
is minimal, self-contained, deterministic, and tags the element(s) under test with
`data-cid`. Current status: **186/186 boxes within 2px across 64 gated fixtures (all 64
fully clean)**, plus **4 documented deferrals** (below) that are reported but not gated.

### Corpus expansion (`31`–`68`)

Adds the six families the original 30 had **zero** coverage for, plus more inline/table/
image/positioning guards. Each fixture cites its real-site origin + CHANGELOG entry in a
`<body>` comment.

| range | family | fixtures |
| ----- | ------ | -------- |
| 31–36 | **CSS Grid** | column-span, template-areas, single-auto-column, justify-items, minmax/fr/gap, min-content row† |
| 37–39 | **calc()** | `calc(% − px)` length, `calc()` inset (vs CB height), `calc()` in a media feature |
| 40    | **aspect-ratio** | auto-height = width ÷ ratio |
| 41–42 | **2D transforms** | translate, scale + `transform-origin` (getBoundingClientRect parity) |
| 43–44 | **`@media` queries** | height query (google blank-page guard), width breakpoint (no space after `@media`) |
| 45    | **logical props** | `margin-inline-start` / `padding-block-start` → physical sides |
| 46–49 | **sizing** | `box-sizing:inherit`, border-box `px` height, `vw`/`vh`, `top:%` vs CB height |
| 50–52 | **inline / white-space** | `nowrap` full-text sizing, inline-block min-content, `::after` no-leak |
| 53–55 | **tables** | auto min-content†, `border-spacing`, empty spacer-row height |
| 56–58 | **replaced images** | `%`-width + intrinsic aspect (PNG), intrinsic-in-flex (GIF), `object-fit` box |
| 59–68 | **UA / overflow / sticky / positioning** | section-scoped `h1`, h4/h6/p em-margins, sticky, overflow-clip, auto-inset abs flex child†, sr-only drop, visibility-hidden†, auto-inset static position, float scoping across sections, `flex-basis` from width |

† = a documented deferral (see below).

### Documented deferrals

Known-hard or intentional-divergence fixtures carry a `<!-- conformance:defer <reason> -->`
marker; the runner reports their deltas as `XFAIL` and lists them separately without
reding the suite (so an open gap is tracked, never hidden or force-passed):

- **`36-grid-min-content-row`** — explicit `min-content` grid-row height doesn't yet match
  Chromium across both cells (diagnosed-but-unfixed; grid geometry is otherwise clean).
- **`53-table-auto-min-content`** — auto-layout collapsed-table column min-content sizing
  (the `table.rs` follow-up; separate-border case is close, collapsed still diverges).
- **`63-abs-auto-inset-flex-child`** — **intentional divergence**: the engine anchors a
  fully auto-inset absolute flex child to its static start (0.2.14 fix for google's AI-Mode
  icon), while Chromium spec-centers it via `justify-content`. Not reverted for a minimal
  repro.
- **`65-visibility-hidden-reserves-space`** — `visibility:hidden` is dropped from layout
  (should reserve its space); paint-drop is correct, layout-space reservation is open.

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
- **`border-collapse` border merging (07/29):** a fixed-layout collapsed table now
  merges shared cell edges onto a collapsed grid — each border counted once, half on
  each side (CSS 2.1 §17.6.2) — so the table/row/cell boxes size correctly across a
  `colspan` (fixture 29 went from ~5px to ≤0.6px; 07 from 1px to exact).

The `31`–`68` expansion then drove six more fixes: grid `justify-self` mapping,
`transform-origin` parsing (was pinned to the box centre), viewport-height threading for
`vh`/height-`@media`, shrink-to-fit flooring at min-content (`white-space:nowrap`
overflow), the browser-default `td`/`th` `padding:1px`, and GIF/WebP intrinsic probing +
`data:` URI decode so replaced-image fixtures size for real.

### Known remaining gaps

Beyond the 4 gated deferrals above: **`border-collapse` on AUTO-layout tables** — the
merged-grid model applies to `table-layout: fixed` collapsed tables; an auto-layout
collapsed table still lays out with each cell's full borders (a `table.rs` follow-up,
exercised by `53`). The **inline replaced-image baseline strut** (an inline `<img>`'s line
box reserves the font descent below the baseline) is not modelled — image fixtures use
`display:block` to isolate the sizing under test. GIF/WebP are **probed for size but not
pixel-decoded** for paint.

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
