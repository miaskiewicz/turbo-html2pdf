# Paged media: directives, headers/footers, footnotes, pagination

turbo-html2pdf paginates automatically: **page count is an output, never an input**.
You write a continuous body, declare optional running headers/footers and
footnotes with `t:` directives, set the page geometry with `@page`, and the
fragmenter fills one page at a time.

This document covers the `t:` directives, running headers/footers and the
per-page context, footnotes, and the pagination rules. See
[dsl.md](dsl.md) for templating and [css-support.md](css-support.md) for CSS.

> **Implemented vs deferred.** The current slice implements the *master-less*
> running header/footer path, footnotes, and the auto-pagination break walk.
> Page masters, the general counter, leaders, endnotes, and a few related
> directives are recognized by the parser but **not yet implemented** — they are
> flagged below with the `TODO(phase…)` note from the source.

---

## 1. The `t:` directives

`t:`-prefixed elements are recognized as typed directives
(`TKind` in `crates/turbo-html2pdf-core/src/markers.rs`, re-exported as
`turbo_html2pdf_core::TKind`; the template parse, `<t:…/>` expansion and the
unknown-`t:` error are in `src/template/markup.rs`, and layout sees each directive as
an opaque turbo-dom-layout marker). The parser knows the following
local names. Their implementation status (verified against the handling code and
`TODO(phase…)` markers) is:

| Directive | Status | What it does |
|---|---|---|
| `<t:running-header>` | **Implemented** | Declares the per-page running header region (§3.0). See [below](#running-headers-and-footers). |
| `<t:running-footer>` | **Implemented** | Declares the per-page running footer region. |
| `<t:page/>` | **Implemented** (inside a region) | The current 1-based page number. Desugars to `{{ page.number }}`. |
| `<t:pages/>` | **Implemented** (inside a region) | The total page count. Desugars to `{{ page.total }}`. |
| `<t:footnote>` | **Implemented** | An inline footnote marker whose body flows to the footnote area of the page the marker lands on (§3.6). |
| `<t:footnote-separator>` | Recognized; a default separator rule is always drawn | The element kind is parsed, but the footnote band currently paints a built-in default separator rule (a thin black box). A custom separator from this element is **not yet wired in**. |
| `<t:page-master>` | **Deferred** — `TODO(phase7b)` | Page master definition. Parsed but not implemented. |
| `<t:variant>` | **Deferred** — `TODO(phase7b)` | Page-master variant. Parsed but not implemented. |
| `<t:use-master>` | **Deferred** — `TODO(phase7b)` | Apply a named page master. Parsed but not implemented. |
| `<t:region>` | **Deferred** — `TODO(phase7b)` | Named master region. Parsed but not implemented. |
| `<t:counter>` | **Deferred** — `TODO(phase7b)` | General named counter. Parsed but not implemented. |
| `<t:leader>` | **Deferred** — `TODO(phase7b)` | Dot-leader fill (e.g. for a TOC). Parsed but not implemented. |
| `<t:anchor>` | **Deferred** — `TODO(phase15)` | Section anchor (needed for `section`-reset footnotes). Parsed but not implemented. |
| `<t:endnote>` | **Deferred** — `TODO(phase15)` | Document-end note. Parsed but not implemented. |
| `<t:endnotes>` | **Deferred** — `TODO(phase15)` | Endnote collection point. Parsed but not implemented. |

The two field codes `<t:page/>` and `<t:pages/>` are only meaningful **inside a
running region**, where they desugar to `{{ page.number }}` / `{{ page.total }}`
(see `template/regions.rs`). Both the self-closing form (`<t:page/>`) and the
explicit pair (`<t:page></t:page>`) are accepted.

---

## 2. Running headers and footers

Place a `<t:running-header>` and/or `<t:running-footer>` **anywhere** in the
template. Their inner markup is lifted out of the body flow at compile time and
re-rendered once per page, so they never appear in the body. Source of truth:
`template/regions.rs` + `render.rs`.

```html
<t:running-footer>Page <t:page/> of <t:pages/></t:running-footer>
<p>… body …</p>
```

The footer above is the ergonomic "Page X of N": `<t:page/>` and `<t:pages/>`
desugar to the page-context variables, so no expression syntax is needed.

### The per-page context

Each region is rendered against a context with two top-level keys:

- `page` — the per-page state, with fields:
  - `page.number` — 1-based page number (also `<t:page/>`).
  - `page.total` — total page count (also `<t:pages/>`).
  - `page.is_first` — true on the first page.
  - `page.is_last` — true on the last page.
- `data` — the caller's original document data, so a region can interpolate both
  page state and document fields.

```html
<!-- print only on the last page -->
<t:running-footer>{% if page.is_last %}END{% endif %}</t:running-footer>

<!-- combine document data and page number -->
<t:running-footer>{{ data.doc }} p<t:page/></t:running-footer>
```

(Verified in `tests/render.rs`: the page number is late-evaluated per page, and
`page.is_last` / `data.*` work inside the footer.)

### Sizing and overflow

A region's height is measured once and reserved as a band at the top (header) or
bottom (footer) of the page, **capped at the page margin** so a region can never
push into the body or past the page edge. If a region's content is taller than
its reserved band, the overflowing lines are **clipped** and a `RegionOverflow`
lint is emitted (the body is never overlapped).

> One-pass approximation (documented in `render.rs`): the band height is measured
> using a representative page-1 context (`number = 1`, `total = 1`). A region
> whose *height* changes with the page number reserves the page-1 height on every
> page; per-page content taller than that band is clipped + linted. A full
> fixpoint over band height is `TODO(phase7b)`.

---

## 3. Footnotes

A `<t:footnote>` leaves a numbered marker inline and moves its body to the
footnote area of whatever page the marker lands on **after pagination**. Source:
`paginate/footnotes.rs` + `render.rs`; behavior verified in `tests/footnotes.rs`.

```html
<p>Cited<t:footnote>the note body</t:footnote> here.</p>
```

### Auto-numbering

Auto notes take the next integer in document order (`1`, `2`, `3`, …). The
default numbering is **document-continuous** across all pages.

### Manual marks

A `mark` attribute supplies an explicit marker that overrides auto-numbering, and
runs an **independent** sequence from the auto notes:

```html
<p>Body<t:footnote mark="*">starred note</t:footnote>.</p>
```

### Reset modes (`footnote-reset`)

The `footnote-reset` attribute controls how auto-numbering resets. The **first**
`footnote-reset` declared on any footnote sets the document policy.

- `footnote-reset="page"` — restart auto-numbering at 1 on each page. (After
  pagination, the notes that landed on a page are renumbered from their position
  on that page; manual marks still pass through unchanged.)
- absent, `none`, or `section` — **continuous** numbering. (`section` reset
  awaits section anchors and is currently treated as continuous —
  `TODO(phase15)`.)

```html
<t:footnote footnote-reset="page">policy note</t:footnote>
```

### The footnote band and separator

The footnote area sits above the bottom margin (and the footer band, if any) and
grows upward as notes accumulate. A **separator rule** — a thin full-width black
box — heads the band on every page that has notes. (A custom
`<t:footnote-separator>` element is parsed but not yet wired in; the default rule
is always used.)

### Oversized-note continuation

A footnote taller than the whole footnote area **continues onto the next page's
footnote area**, split line by line, so content is never dropped (§3.18).

### Convergence

Footnotes are a mutual constraint: a page's notes reserve area at its bottom,
which shrinks its body capacity, which can change which markers land on it. This
is resolved by a damped fixpoint (cap of 4 passes). In the common case it
converges silently. If it does not converge within the cap, the last placement is
accepted and a `FootnoteConvergence` lint is emitted — content is never dropped.

---

## 4. Pagination

The break walk (`paginate/walk.rs`) fills one page's body capacity at a time and
starts a fresh page when the current one fills. Page count is purely a function
of content and capacity.

### Page geometry — `@page`

Set the page size and margin with an `@page` at-rule in your CSS
(`paginate/geometry.rs`). The internal unit is CSS pixels at 96 dpi.

```css
@page { size: A4; margin: 20mm }
@page { size: Letter }
@page { size: A4 landscape }
@page { size: 400px 500px; margin: 10px 20px }
@page { size: 300px }            /* a single length is a square */
```

**`size`** accepts:

- A **named size**: `A3`, `A4`, `A5`, `Letter`, `Legal` (case-insensitive).
- **One length** `W` — a square `W × W`.
- **Two lengths** `W H`.
- An optional `portrait` / `landscape` keyword, which may accompany a named size
  (`A4 landscape` swaps width and height). With no dimensions and only
  `landscape`, the default A4 is used and oriented landscape.

An **unknown** named size is a fatal render error.

**`margin`** accepts the standard 1 / 2 / 4 length shorthand:

- `margin: 20mm` — all four sides.
- `margin: 10px 20px` — vertical / horizontal.
- `margin: 1px 2px 3px 4px` — top / right / bottom / left.

**Default geometry** (when there is no `@page`, or `@page` sets only one of
size/margin): **A4 portrait with uniform 20 mm margins.**

> Named *pseudo-pages* (e.g. `@page :first`) and per-page masters are not
> implemented in this slice — `TODO(phase7b)`. The first `@page` at-rule's
> `size`/`margin` declarations are what take effect.

### Break rules

Break behavior is driven by CSS on the body content (see
[css-support.md](css-support.md#break-control)). The decision precedence is
(highest first):

1. **Forced break** — `break-before: page` / `break-after: page` (the value
   `column` is treated as `page`).
2. **Atomicity** — `break-inside: avoid` keeps a block together where it fits.
3. **Widows / orphans** (paragraphs).
4. **Greedy fill** — otherwise the block goes where it fits, splitting if it
   cannot fit on any page.

A block that cannot fit even on an empty page is split internally; an atomic leaf
taller than a page overflows onto a fresh page with a `RegionOverflow` lint
(content is never dropped).

### Margins at page starts

As Chrome prints (CSS Fragmentation §5.2; turbo-html2pdf#31, #32): only margins are
truncated at a break, never padding, borders or a box's remaining height.

- The first page starts at the top of the document, so the first block keeps its top
  margin (and `<html>`/`<body>` margins, borders and padding above it).
- After an **unforced** break (content moved because it did not fit) the margin is
  truncated and the block starts at the top of the page area. If the content that moved
  is the first content of a block being split, the break is before that block: its top
  border and padding (and any margin inside them) stay above the content on the new
  page, and only the block's own margin is truncated.
- After a **forced** break the next block keeps **its own** top margin (collapsed with its
  first child's when no padding or border separates them). It does not keep a larger
  bottom margin of the block before the break.
- A forced break on a page that has no content yet starts no new page, so Chrome's
  behaviour applies: `break-before: page` on the first content makes no blank first page,
  and `break-after` on one block plus `break-before` on the next break only once.
- No break can separate a box's last content from the bottom padding and border that end
  the box (or its ancestors', `<body>`'s included). A block that would exactly fill the
  page is followed by such padding, so it moves to the next page with it. A trailing
  margin on its own does not move it.
- A box taller than its content (a definite `height`) ends in remaining height that
  breaks like content: what doesn't fit continues at the top of the next page, before
  the next block (#33).
- A `position: relative` box is painted shifted, but the flow continues from its
  unshifted position (#34).

### Split boxes

A block split across pages paints a slice of its box (background, border, shadow) on
every page it spans, like `<html>`/`<body>` above: Chrome's default
`box-decoration-break: slice` (#37). The top edge is on the first page and the bottom edge
on the last, the box running to the page area's end in between. A forced break anywhere
inside a block breaks the page there (#38). Percentage margins and paddings resolve
against each box's containing block.

### Positioned boxes in print

As Chrome prints them (#32):

- A **`position: fixed`** box repeats on **every page**. It sits at its insets from the
  page area (the print viewport), or at its static position if it has none. With only
  `bottom` set, it is anchored to the page area's bottom.
- A **`position: absolute`** box with `top` or `bottom` set sits at that offset from its
  containing block, which is the nearest positioned ancestor, or else the first page
  area. The offset is measured from where that block starts and runs on through the
  following pages, so `top: 900px` on a 320px-tall page area lands on page 3, adding
  pages after the content if needed. A box placed inside a static block that moves to a
  later page stays relative to the first page.
- An `absolute` box with neither `top` nor `bottom` stays at its static position in the
  flow and takes no space.
- Fragments paint in stacking order: a `z-index` below 0, then the flow, then positioned
  boxes, then a positive `z-index` (#35).

The page area is the page minus its `@page` margins. The root (or propagated `<body>`)
background fills only that area, as in Chrome.

### `<html>` / `<body>` boxes across pages

`<html>` and `<body>` are dissolved into the page flow before pagination; their
background (when not propagated to the canvas — e.g. `<body>`'s when `<html>` has one),
border and shadow are painted on **every page** as Chrome's default
`box-decoration-break: slice` does: from the box's top on the first page (top edge
there only), through the page area's end on every page but the last, to the end of the
content plus padding and border on the last page (bottom edge there only). A sliced box
has square corners.

### Orphans and widows

Paragraphs honor `orphans` (minimum lines before a break) and `widows` (minimum
lines after a break), default `2` each.

- **Widows** pull lines back from a break so at least `widows` lines start the
  next page.
- **Orphans**: if fewer than `orphans` lines would remain on the current page,
  the whole run is deferred to the next page.
- On an otherwise-empty page where orphans cannot be satisfied, whatever fits is
  placed anyway (no infinite deferral).

### Repeatable table headers

`<thead>` rows (UA `display: table-header-group`) and `<tfoot>` rows
(`table-footer-group`) are marked repeatable, so when a table spans multiple
pages the header is **re-emitted atop every continued page** (`layout/table.rs`,
`paginate/walk.rs`), and the next rows follow it directly (#36). This needs no extra attribute — just use `<thead>`:

```html
<table>
  <thead><tr><th>Item</th><th>Amount</th></tr></thead>
  <tbody>
    <tr><td>…</td><td>…</td></tr>
    <!-- many rows … -->
  </tbody>
</table>
```

### Documented pagination limitations

From `paginate/walk.rs`:

- A block that does not fit the rest of the page but fits on an empty one moves to the
  next page whole (AC-6.2), where Chrome would split it at the page end.
- An absolutely positioned box taller than the page remainder is placed whole (Chrome
  fragments it); a fixed box's containing block is the page body area (between any
  running header and footer bands).
- A last child's bottom margin is dropped by the paged layout (turbo-dom#41), so a block
  whose own bottom margin would push it past the page end with its parent's padding
  stays.
- Orphans/widows are enforced at each single break, but a paragraph tail spanning
  more than two pages is greedy.

Both are refined alongside the emitter in a later phase.
