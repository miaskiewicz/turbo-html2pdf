# Changelog

All notable changes to turbo-html2pdf are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/); versions follow SemVer. The npm,
PyPI, and crates.io packages release in lockstep from a `v*` tag (PyPI on `pyv*`).

## [0.6.0] — 2026-10-26 — turbo-dom consolidation

The engine is now built on the turbo-dom workspace crates instead of carrying its own
copies: `turbo-html2pdf-core` depends on **turbo-dom-layout 0.6** (style cascade,
layout, text/fonts incl. the bundled faces, images, SVG, layout snapshots) and
**turbo-dom 0.6** (the DOM templates are parsed into and the one Chrome-correct selector
engine). Every rendered PDF of the release corpora (templates, conformance pages, PDF/UA,
CMYK, PDF/A, the image matrix, with and without `svg`) is byte-identical to 0.3.9 through
the consolidation steps; the behaviour changes are the selector fixes and the P5b bug fixes
below, each backed by a Chrome 156 control.
The version jumps from 0.3.9 to 0.6.0 so every consolidated package (turbo-dom,
turbo-test, turbo-html2pdf, turbo-surf) releases on one version line.

- **Changed (turbo-dom pin)**: `turbo-dom-layout` + `turbo-dom` are pinned to the final
  turbo-dom consolidation head (c9f991c; via 0189e4d, f0fb306, d660013 and 443f4d8, was f860fea). Render changes, each checked
  against Chrome 156 `page.pdf()`: `pre`, `pre-wrap` and `pre-line` break at newlines and
  `pre-wrap` keeps its spaces (turbo-dom#120/#121/#122/#166); an inline-block sits on its
  last line's baseline (turbo-dom#142); `@supports` with two condition groups and no
  `and`/`or` is invalid and dropped. A table row's `break-inside: avoid` now reaches the
  break hints (turbo-dom#158), so html2pdf's `BoxFacts::avoid_break_inside` workaround is
  removed. Render parity corpus (html2pdf, html2pdf-svg): no output changes. The
  later repins bring turbo-dom#176 (font shorthand) and the turbo-dom-browser-env
  panic fixes turbo-dom#183/#184 (a crate html2pdf does not link).

- **Fixed (turbo-html2pdf#47, Chrome 156 print control)**: a content-free block (an empty
  `<div>`, a table row of empty cells; nothing inside it `break-inside: avoid`,
  positioned or a repeated header) that does not fit is sliced at the page end as Chrome
  156 slices it: the box is drawn on each page it spans, clipped to the rows it covers
  there (top border/padding on the first page, bottom ones on the last), its
  continuations as `CARRIED` copies, below the next page's repeated headers. It used to
  move to the next page whole (or overflow one taller than a page). A table row's
  `break-inside: avoid` comes from its fragment's break hints (turbo-dom#158). Controls: `slice-empty-*`.
- **Checked (turbo-html2pdf#46)**: the `svg` feature has no separate output backend — it
  rasterizes `image/svg+xml` into an image XObject painted by the same PDF painter, so a
  vector image cut at a page boundary by a split relative block (#42) is clipped there
  like any fragment. Pinned by `tests/svg.rs`
  (`a_vector_image_cut_at_a_page_boundary_paints_inside_its_clip_on_both_pages`: the
  clip path around each page's draw, and a `pdftoppm` probe of both pages).
- **Checked (turbo-html2pdf#45, Chrome 156 print control)**: a definite-height box around
  a table whose header row repeats ends where Chrome 156 ends it — the repeated header is
  the table's, inside the box, so the #43 model needs no header term. Controls:
  `remaining-height-table-header*` in `scripts/pagination-chrome-control.mjs`. (A box
  inside a repeating table's cell would need one, but the engine keeps table rows whole,
  so such a box is never split.)
- **Fixed (turbo-html2pdf#44)**: paint a split relatively offset block carries onto a
  neighbouring page (#42) repeats content its own page carries, so it is no longer tagged
  or extracted twice: its fragments carry the id `paginate::CARRIED` (re-exported at the
  crate root), a tagged render marks them `/Artifact` (no MCID, not in the structure
  tree), and a carried text line is shown inside a `/Span` with an empty `/ActualText`,
  so text extraction finds the text once, on its own page, tagged or not.
- **Fixed (turbo-html2pdf#42, Chrome 156 print control)**: a relatively offset block
  split across pages is offset in the fragmented flow, as Chrome 156 prints it: its paint
  (content and sliced background/border) shifted across a page boundary is cut there
  (fragment `clip` regions) and the part beyond paints on the neighbouring page — shifted
  above a page's area at the end of the previous page, past a page's end at the top of
  the next — with Chrome's 1px bleed above a continuation page's area. It used to paint
  into the page margin. Control: `relative-bottom-split`, `split-decorated-relative` (no
  longer `KNOWN` differences; the `KNOWN` list is empty).
- **Fixed (turbo-html2pdf#41, Chrome 156 print control)**: an absolutely positioned box at
  its static position (no `top`/`bottom`) between the children of a split block is placed
  at that position — right after the child before it, on that child's page — as Chrome
  156 prints it, not after the block's later children (the galley lists it after them).
  Control: `absolute-static-mid-split-relative` (no longer a `KNOWN` difference).
- **Fixed (turbo-html2pdf#43, Chrome 156 print control)**: a split block with a definite
  height counts the space it takes on each page it breaks on — to the page end,
  including the gap left when a child moves to the next page — toward its height, as
  Chrome 156 prints it, so it ends (and the next block follows it) that far down the
  page it ends on, above an overflowing last child; a height used up a page early ends
  the block at that page's top. It used to carry its whole galley remaining height
  below its last content (a page too late). An auto-height box taller than its content
  (`min-height`, a table's border spacing) keeps carrying its remaining height below its
  content (`BoxFacts::definite_height` tells them apart). Control: `remaining-height-pushed-child*`,
  `remaining-height-used-up` in `scripts/pagination-chrome-control.mjs`.
- **Changed (turbo-dom pin)**: `turbo-dom-layout` + `turbo-dom` are pinned to the final
  turbo-dom consolidation head (f860fea, was e888342). The only engine changes reaching
  html2pdf are turbo-dom#107 (turbo-dom-layout's AVIF size probe treats a truncated AVIF
  — primary item's `iloc` extents missing from the data — as broken, like Chrome 156)
  and the additive `rtdom::color` canvas serialization of turbo-dom#108; the rest
  (turbo-dom#99, #109–#112, turbo-test#125/#127) is in turbo-dom-browser-env /
  turbo-dom-raster, which html2pdf does not link. Render parity (html2pdf, html2pdf-svg,
  html2pdf-extra): no output changes.
- **Fixed (turbo-html2pdf#39, Chrome 156 print control)**: the page area is the print
  initial containing block for layout too: the body flow is laid out with
  `turbo_dom_layout::layout_recorded_in_viewport` at the page area's height, so a
  `position: fixed` box (on every page) and an absolutely positioned box of the initial
  containing block resolve `%` insets (`top: 50%`), `%` heights and `top` + `bottom`
  stretching against it, as Chrome prints them (they collapsed to the page top / to no
  height). Control: `icb-*` in `scripts/pagination-chrome-control.mjs`
  (`tests/pagination_chrome.rs`). Pinned to turbo-dom `consol/p5b-w7` (db8c699; the entry
  since e76da05), which also brings the P5b-W6 / W7 layout fixes (turbo-dom#75–#89,
  #93, #100–#102: margins, heights, intrinsic widths, inline-block margins, floats,
  column flex, flow-root, positioned boxes, block-in-inline).
- **Changed (turbo-dom-layout P5b-W5, Chrome 156 controls)**: through the turbo-dom pin,
  PDFs lay tables and alignment out as Chrome does — `<table>` is `box-sizing:
  border-box` (turbo-dom#62); `border-spacing` is inherited and 0 except on `<table>`
  (turbo-dom#63); a table's extra height goes to its rows (turbo-dom#64) and its extra
  width to its auto columns, so empty cells of a `width:100%` table get their share
  (turbo-dom#55); `box-sizing: border-box` widths are not counted twice in intrinsic and
  column widths (turbo-dom#65); only the legacy `-webkit-*` alignment (`<center>`,
  `align` attributes) moves block children, `text-align: center` aligns inline content
  only (turbo-dom#69); `<th>` is bold and centred (turbo-dom#70). The corpus outputs each
  changes are listed in turbo-dom's `tools/parity/expected/p5b-w5-turbo-dom-*.json`.
- **Fixed (turbo-dom#41)**: paged layouts keep a last child's bottom margin inside a
  padded / bordered parent (and collapse it through an auto-height one) as Chrome does —
  turbo-dom-layout's generic policy used to drop it. The pagination control's
  `exact-fill-child-margin` case (#32 item 6) now matches Chrome 156 (no longer pending).
  Changes the galley of every document whose blocks end with a margined child.
- **Fixed (turbo-dom#52)**: a partially clipped box (`position: absolute` with
  `clip: rect(…)`, e.g. a CSS sprite) no longer vanishes from the PDF: turbo-dom-layout
  keeps it and records its region in `Fragment::clip`, and the emitter paints the box and
  its subtree inside a saved graphics state with that rectangle as the clipping path
  (`q … re W n … Q`). A box whose clip keeps nothing (the sr-only pattern) is still left
  out of layout, so its text stays out of the PDF and its tags; `clip` on a box that is
  not absolutely positioned is ignored, as in Chrome. Control: turbo-dom's `clip-*`
  Chrome 156 cases; `tests/clip_rect.rs` (clip operators, `pdftoppm` probes).
- **Fixed (turbo-dom#51)**: a table `<caption>` is laid out (by turbo-dom-layout) as
  Chrome does — in the table wrapper box above or below the grid, with its own box,
  widening the table to its min-content; before, its content became grid rows and its
  text was dropped. Under PDF/UA it is tagged as the `Caption` child of its `Table`
  (`UaRole::Caption`), and the anonymous grid box of a captioned table
  (`NodeSources::table_grids`) is no structure element, so the rows stay the table's `TR`
  children (it was an unsourced box tagged `P`). Control: turbo-dom's `caption-*` Chrome
  156 cases; `tests/pdf_ua.rs` (`Table > Caption, TR`, veraPDF ua1).
- **Fixed (#32)**: pagination matches Chrome 156 `page.pdf()` for positioned boxes and
  breaks. `position: fixed` boxes repeat on every page at their page-area position.
  `position: absolute` boxes with `top`/`bottom` sit at their offset from their
  containing block's start, continued across pages (new pages past the content);
  static-position ones take no space. A forced break before the first content (or a
  second one at the same break point) makes no blank page. A split block starting a page
  keeps its top border and padding. After a forced break only the next block's own top
  margin is kept. A block's last content moves to the next page with its ancestors'
  bottom padding/border. The canvas already painted only the page area (now covered by
  the control). `paginate_document` takes a `Galley { root, wrappers, boxes }`;
  `render_pages` records the body's used box values (`FlowBoxes`/`BoxFacts`). Control:
  `scripts/pagination-chrome-control.mjs` (41 cases); corpus changes in turbo-dom
  `tools/parity/expected/p5b-w4-turbo-html2pdf-32-*.json`.
- **Fixed (#33)**: a split block taller than its content keeps its remaining height (and
  bottom padding) at the top of the next page.
- **Fixed (#34)**: a relatively positioned block's offset no longer shifts the rest of
  the paginated flow. The content of a split positioned block keeps its paint level.
- **Fixed (#37)**: a block split across pages paints a slice of its box (background,
  border, shadow) on every page it spans (`box-decoration-break: slice`, as for
  `<html>`/`<body>`). The fragmenter's percentage margins and paddings resolve against
  each box's containing block, not the page width.
- **Fixed (#38)**: a forced break inside a block that fits on the page now breaks the
  page.
- **Fixed (#36)**: on a continued page the rows of a split table follow its repeated
  header directly. Before, they sat at their distance from the header's original
  position: `template-report` printed 4 pages, and now prints 2 like Chrome.
- **Fixed (#35)**: the PDF emitter paints fragments in stacking order (negative
  `z-index`, flow, positioned, positive `z-index`), as the raster does. Tagged PDFs
  number their marked content in that order and keep the structure in document order.
- **Fixed (#31)**: paginated PDFs place blocks at page starts as Chrome 156 prints them
  (CSS Fragmentation §5.2): the first page keeps the leading top margins (and the
  `<html>`/`<body>` margin, border and padding), a forced break keeps the next block's
  gap, an unforced break still truncates it. `<html>`/`<body>` backgrounds, borders and
  shadows are painted on every page fragment (`box-decoration-break: slice`: top edge on
  the first page, bottom edge on the last, the box running to the page area's end in
  between); new `paginate_document`. Control: `scripts/pagination-chrome-control.mjs`
  (Chrome `page.pdf()` rasterized at 96 dpi) → `tests/pagination_chrome.rs`. Every
  rendered PDF whose first block sits below the top of the page area changes (page 1
  moves down by the body margin + first margin) — turbo-dom
  `tools/parity/expected/p5b-w2-turbo-html2pdf-31.json`.
- **Fixed (#28)**: CI tests and lints the core in the configurations the workspace run
  never builds: `--features svg`, `--all-features` (the `perf` hot-path budgets) and
  `--no-default-features` (the font-less wasm build). The last `--no-default-features`
  failure (`native_inline_coverage`, which shapes text) is gated on `bundled-fonts`, and
  the `perf` shape budget is restated for the bounded shape cache (distinct runs are
  shaped at most twice: 6096 for the 3000-run table on every platform, budget
  `<= 2 x 3000 + 100`), with a new repeated-runs test pinning the dedupe (3 shapes).

- **Fixed (#29)**: HTML documents render like Chrome. `<head>` `<style>` sheets apply (in
  the PDF render as well as `layoutBoxes`/`layout_html`; document sheets come before the
  caller's `css`, a caller `@page` rule takes precedence over the document's), `<html>` and
  `<body>` are real boxes (UA `body { margin: 8px }`, html/body margin, padding, width),
  `rem` follows the root font size, `<style>`/`<script>`/`<noscript>` text is no longer
  painted into the PDF, and the root/`<body>` background colour fills each page's page area
  (`Page::canvas`). The `<html>`/`<body>` boxes are dissolved into the page flow before
  pagination (their own borders were not painted, as for any block split across pages;
  sliced onto every page since #31, below).
  Nearly every PDF/`layoutBoxes` output of the corpora changes (body margin); the
  conformance pages' boxes now match Chrome 156 for 172 of 209 tagged elements (was 1).
- **Fixed (#30)**: EXIF orientation (JPEG APP1, PNG eXIf — as Chrome 156; WebP EXIF is
  ignored there too) is honoured: images lay out at their displayed size (orientations
  5-8 swap width/height) and the embedded image is drawn turned (the stored JPEG still
  rides through as `DCTDecode`; the placement `cm` rotates/flips it). `image-orientation:
  none` keeps the stored orientation.
- **Image formats (turbo-surf#30, shared image layer)**: BMP, ICO/CUR and AVIF `<img>` /
  background images are now sized from their headers (as Chrome loads them); like GIF and
  WebP they are not embedded in the PDF yet. `layoutBoxes` decodes percent-encoded
  `data:` image URLs too (e.g. inline `data:image/svg+xml,…` icons under the `svg` build).
- Summary for users: same public paths (`turbo_html2pdf_core::style::…`, `::layout::…`,
  `::Tag`, `::Fragment`, …, re-exported from turbo-dom-layout); selectors match like
  Chrome (escapes, namespaces, `:has()`, `of S`, `:nth-last-*`, `:lang()`, `:defined`,
  `:read-only`/`:read-write`, `:required`/`:optional`, unknown pseudo-classes drop their
  rule, Selectors 4 specificity); fonts come from turbo-dom-layout (no duplicated
  `assets/fonts` in this repo); the legacy owned-`Node` API is removed (see below).
- `turbo-html2pdf-mcp` depends on core `version = "0.6"`; workspace
  `rust-version = "1.89"` (was 1.88; turbo-dom-layout's floor).
- Release: `release-crates.yml` waits for turbo-dom-layout 0.6.0 / turbo-dom 0.6.0 on
  the crates.io index before publishing core, and a tag-only `release-guard` job fails
  while `[patch.crates-io]` or git dependency sources remain. The npm package's
  `assets/fonts/Go-Regular.ttf` is no longer tracked: `package-notices.mjs` writes it
  from turbo-dom-layout's font tree at pack time (the published file list is unchanged).

- turbo-dom-layout's legacy owned-`Node` input is gone, so are the re-exports built on it:
  `node`, `Node`, `Element`, `style_tree`, `collect_style_css` (the Tree versions are
  `style_input` and `html_layout::collect_style_css_tree`; `Attr` / `MarkerKind` / `Tag`
  stay, from turbo-dom-layout's `tag`), `Flow::to_nodes`; `parse_html` now returns the
  parsed `Flow` (`t:` directives as markers). The S2 legacy-vs-Tree equivalence test is
  retired. Output is byte-identical.
- No vendored fonts: `crates/turbo-html2pdf-core/assets/fonts` (a copy of
  turbo-dom-layout's) is deleted; Rust tests and benches read the fixtures through
  turbo-dom-layout's `test-support` API, JS scripts, the napi tests and the benches
  through `scripts/layout-crate-dir.mjs` (`cargo metadata`). `package-notices.mjs`
  builds the shipped notices from turbo-dom-layout's font tree and now also writes and
  checks the npm package's `Go-Regular.ttf`.

- The layout half of `turbo-html2pdf-core` (style cascade, layout, text/fonts incl.
  the bundled faces, layout snapshots, `layout_html`, images, SVG, perf counters,
  errors, the `t:` markup parse) moved verbatim to the `turbo-dom-layout` crate in
  the turbo-dom workspace. `turbo-html2pdf-core` depends on it and re-exports it at
  the same paths, so `turbo_html2pdf_core::style::…`, `::layout::…`, `::Fragment`,
  … are unchanged (the legacy `::Node` input went at S5, first bullet); output is
  byte-identical. Its `bundled-fonts`,
  `perf`, `svg`, `xref`, `pdf-ua` and `print-color` features forward to the layout
  crate. Layout tests and their Chrome fixtures, and the `font_startup` example,
  moved with the code.
- html2pdf no longer keeps its own DOM: rendered markup is parsed into a turbo-dom
  document and styled through turbo-dom-layout's Tree input (output byte-identical).
  `Program::render_nodes` / `render_region` return a `Flow` (`Flow::style`; the
  interim `Flow::to_nodes` legacy copy is gone again, first bullet); `TKind` lives in the new `markers`
  module and directives are `Tag::Marker` / `FragmentContent::Marker` in the layout
  types. PDF/UA roles/alt text and xref links/anchors are derived after layout into
  `Page::semantics` (`Semantics::role` / `alt` / `link_href` / `anchor`) instead of
  `Fragment` fields; `UaRole` moved to `semantics`. The `xref`, `pdf-ua` and
  `print-color` features no longer forward to turbo-dom-layout (they set its
  runtime options). The root `layout_html` / `layout_boxes` / `parse_html` stay
  html2pdf-mode; `html_layout::*` is turbo-dom-layout's directive-free drive.
- Selectors are parsed and matched by turbo-dom's one selector engine (the engine
  behind turbo-dom's `matches` / `querySelector*`), through its stylesheet entry points
  `parse_for_cascade` / `match_for_cascade`; html2pdf's own selector parser and matcher
  are gone. Behaviour changes only where the old matcher disagreed with Chrome 156, each
  backed by turbo-dom's Chrome control: CSS escapes, `*|`/`|` namespace prefixes, `of S`,
  `:has()`, `:read-only`/`:read-write`, `:required`/`:optional`, `:nth-last-*`,
  `:lang()`, `:defined` and case-insensitive pseudo-class names now work; an unknown
  pseudo-class or pseudo-element drops its rule (it used to be ignored, so the rest of
  the compound matched); pseudo-elements never style the element; `[href]` no longer
  matches a namespaced `xlink:href`; specificity follows Selectors 4 (`:where()` = 0,
  `:is()`/`:not()`/`:has()` = their most specific argument). Every rendered output of the
  release corpora is byte-identical. `Rule::selectors` is a `RuleSelectors` (prelude +
  the parsed list) instead of `Vec<Selector>`. See docs/css-support.md.

## [0.3.9] — 2026-10-25 — iterate computed style pairs

- Add `ComputedStyle::iter` to iterate computed property name/value pairs, small
  and additive. Consumed by turbo-surf's standardized CSS `zoom` (turbo-surf#28),
  which scales absolute lengths after the cascade; verified there against a
  Chrome 156 zoom/layout control.

## [0.3.8] — 2026-10-21

- Share each loaded system font program across its collection faces instead of
  copying the whole TTC for every index. Keep per-face parsing, shaping caches,
  registration order and public byte constructors intact. Distinguish collection
  indexes when grouping PDF resources and merging inline source runs. Add a
  reusable `font_startup` example reporting eager registry timing, byte ownership
  and selected-face/shaping fingerprints without changing font discovery.

- Split ordinary native inline content around in-flow blocks, including result
  links containing headings. Preserve zero-height continuations, nested source
  ownership, first/last decoration slices and collapsed block margins, so an
  unrelated following control can still be measured. Inline owners reference
  the real lifted block fragment without duplicating paint or DOM identities.
  Admit auto inline-block widths to the native physical LayoutUnit grid.
  Six fresh Chrome 155.0.8059.85 graphs cover exact client rectangles and
  neighboring control geometry; generic/PDF layout policy stays unchanged.
  Positioned/floating split contexts remain explicit unsupported errors.
  Partial source-ID requests and hidden descendants preserve that ownership and
  geometry without inventing inline records; normalized flow types keep the
  separate generic atomic policy free of obsolete native boundary branches.

- Correct README status after verified0.3.7 publication and preserve the actual
  merge/tag/registry release proof.

- Cover the unchanged generic/PDF source-range compatibility path with actual
  raw placed gaps, extra collapsed separators, caret boundaries and soft wraps.
  These public API controls preserve the separate legacy source policy while
  native snapshots retain their complete whitespace cells.

- Preserve native leading whitespace inside its actual inline owner after images
  and inline blocks. Keep the rendered first space and subsequent collapsed
  UTF-16 positions, including direct text, decorated/nested spans and prefixes
  relocated by wrapping. Preserve actual internal whitespace cells in bare text,
  including both line affinities for separator-only selections at soft wraps.
  Nineteen pinned-font normal Chrome controls also cover paragraph starts,
  forced breaks and one/two-space ordinary text selections. Expose actual CSS
  collapse provenance in source clusters so caret hits keep rendered leading
  spaces and follow only zero-space suffixes anchored at a real same-line glyph. Generic/PDF whitespace policy stays intact.

- Admit native HTML text-source items to the physical LayoutUnit grid before
  line breaking and placement. Keep native glyph/partial-selection positions in
  real signed 16.16 advances and aggregate across words and collapsed spaces of
  the same text node; source boundaries and inline decorations advance neighboring
  boxes consistently. Pinned IBM Plex Mono and Fira Code controls use normal
  headed Chrome, including whole versus split text nodes and five-glyph runs.
  Ordinary PDF measurement, glyph advances and generic source snapshots keep
  their previous unquantized policy.
- Preserve real flat-transform FloatQuad boundaries for native HTML text Range
  and block/replaced Element geometry. Ordinary inline Element quads retain their
  accumulated-matrix path. Normal headed Chrome controls distinguish body margins
  from padding, viewport positioning, nested transforms and integer translations;
  the existing fixed-containing-block rejection remains explicit.

- Expand portable regression controls for fixed viewport percentages, adjacent
  source-run boundaries, continued inline struts, own super/sub atomic alignment,
  preserved whitespace and directive markers. Malformed closing markers retain
  their documented advance without inventing glyphs or source element boxes.
  These coverage additions leave production layout code unchanged.

- Size native HTML iframes as replaced viewports using the real 300×150 default
  object dimensions, author attributes, CSS sizing, aspect ratio and box model.
  Preserve definite zero containing-block heights and normal-flow percentage
  sizing on relatively positioned replacements. Accept finite zero native HTML
  viewports, including genuine zero-width regions under explicit HTML/body roots,
  while preserving positive-only PDF snapshot admission and its legacy width floor. Twenty
  independent Chrome controls cover defaults, invalid attributes, zero axes,
  percentage bases, padded positioned containing blocks, excluded iframe fallback
  sources and standards-mode zero-viewport descendants. Reject negative
  and non-finite presentational dimension hints on both HTML and PDF paths.
- Add portable source/Range/inline-edge coverage and actual font-face selection
  controls for native versus PDF aliases. Preserve the CI 100% coverage gate and
  upload its Cobertura report even when the threshold fails for diagnosis.

- Select physical Times for macOS native HTML generic serif, matching fresh
  Chrome's actual font selection. Resolve legacy ascent policy from a face's
  real platform name when Unicode family records are absent. Fifteen independent
  Chrome controls cover serif, Times, Times New Roman, Helvetica and Courier at
  three sizes, including physical ascent/descent, advances, source ranges and
  normal line heights. Explicit families and the existing PDF alias policy stay
  intact; the PDF loader still selects its previous Times New Roman alias.

- Preserve native HTML inline boundaries through shaping and line breaking.
  Capture own-font border fragments, sliced decorations, transparent ancestor
  regions, replaced-element metrics, relative offsets and original text ranges.
  Resolve per-inline physical leading, parent struts and nested super/sub shifts
  for text and atomic children without changing the ordinary PDF layout policy.
  Forty-three source graphs match independently recorded Chrome155 border boxes
  and container heights, including empty inlines, forced breaks, whitespace,
  nested fonts and top/bottom aligned atomic children. Reject unsupported inline
  block splitting, positioning, alignment and writing modes before capture.

- Add an explicit native HTML system-font loader that admits genuine platform-only
  family records before choosing generic aliases. Resolve macOS Menlo as the
  native monospace primary while preserving ordinary PDF font loading and caller
  priority. Cover platform-only name records, physical face identity and the real
  Menlo collection; keep existing PDF behavior under its original loader. Exercise
  the public HTML loader on every test platform, including physical face bytes
  and explicit caller priority, rather than relying only on the macOS control.

- Add a pre-matched declaration cascade for native embedders that match selectors
  against original document and shadow scopes. Preserve normal/important origin,
  encapsulation, inline, specificity and source ordering while reusing existing
  variable, inheritance and font resolution. Expand margin/padding before cascade
  selection so winning shorthands correctly replace lower-ranked longhands.
- Generate no wrapper box for `display:contents`, retaining source ancestry and
  inherited text styles while promoting children into flow and flex/table/grid
  formatting. Suppress replaced HTML elements under the unusual-element rules.
  Fourteen fresh installed-Chrome155 geometry controls cover block/flex promotion,
  replaced elements and real-font anonymous flex/grid items. Preserve NBSP and
  fractional intrinsic widths in native HTML layout instead of applying PDF
  whitespace/whole-pixel rounding. Pass explicit native grid widths to the grid
  sizing algorithm. Scoped cascade and text/source contracts are offline.
  This does not complete SVG or general browser layout parity.

## [0.3.7] — 2026-10-15

- Round actual HTML font ascent/descent at the live snapshot physical scale and
  floor physical half-leading before native line placement. Retain generic/PDF
  policies. Source Range queries encompass nonempty horizontal edges and floor
  equal offsets in physical LayoutUnits before applying native transforms. Fresh
  independently captured Go-font13/15 controls verify exact Range geometry and
  caret hits, while raw source glyph metadata retains real shaper advances.

- Support real inverse-affine caret queries under finite invertible rotation and
  skew, with physical LayoutUnit hit coordinates and native double CSS transform
  coefficients/composition captured during actual box placement. Preserve legacy
  Fragment/PDF float matrices and expose `native_world_transform` for consumers.
- Keep HTML flex intrinsic measurement on the same font policy as final placement,
  and fail before publishing nonfinite native text geometry. Real decoded-image
  regressions ensure the parent transform applies once to its paint fragment.

- Resolve native fractional used-value placement and first-line text indentation,
  including negative/percentage indentation and first lines displaced by floats.
  Reused boxes reset intrinsic, measurement and source policies across HTML scales
  and generic/PDF layout. Controlled graphs verify 391 Range lists, 273 caret hits
  and 1141 fractional hit sweeps.
- Capture native paint hit transforms separately from layout/Range matrices, using
  real transform-scope origin snapping and axis-specific residual propagation.
  Fractional negative origins, nested rotations and one-axis scaling are covered.
- Add bounded original UTF-16 inputs keyed by native source identity, preserving
  existing scalar/public variants. Original-unit cache keys, safe platform cascade
  lookup and real installed fallback fonts reproduce the mixed astral/lone source
  control's 26 Range differences and twelve font/size controls. Source-line Range
  boxes span adjacent fallback runs. Reject invalid source identities/projections.

## [0.3.6] — 2026-10-14

- Add optional live-tree snapshots with genuine glyph/source cluster provenance,
  original byte/UTF-16 ranges and real pen advances. Metadata follows unique
  private native text-fragment IDs through wrapping, transforms and inline atoms,
  while caller-owned source IDs retain the original text node. Existing public
  Fragment/GlyphRun layouts and PDF itemization remain unchanged.
- The text-enabled path checks actual primary-font shaping before per-character
  fallback, retaining normalized combining clusters even when an individual mark
  is absent from the font cmap. Authored Chrome controls with the identical bundled
  font verify full cluster rectangles across wrapping, transforms and four sizes.
  Add a separate HTML font-metric policy with native baseline/ascent/descent and
  transformed cluster boxes, keeping generic/PDF metrics unchanged. This is
  groundwork for genuine Range/
  caret and fragmented inline geometry, not complete CSSOM View support.
- Retain bounded original native text inputs and actual shaper direction for
  Range/caret groundwork. Reject more than 16 MiB or one million Unicode scalars
  of source text before capture.
  The source-enabled route preserves NBSP/em-space glyphs rather than collapsing
  all Unicode whitespace; two additional authored Chrome controls reproduce the
  old defect and verify the correction. Full whitespace-mode/line-breaking and
  caret semantics remain open.
- Add source-scoped native Range rectangles from actual placed clusters and
  collapsible pen gaps, plus caret selection at genuine cluster edges. All offset
  pairs and recorded caret hits match the twelve controlled-font Chrome graphs.
  Missing/unplaced sources, invalid offsets/points and rotated/skewed/degenerate
  caret transforms fail explicitly. DOM hit testing, cross-node Range assembly,
  full bidi and whitespace modes remain separate integration work.
- Honor explicit HTML line heights below the loaded font height through actual
  negative leading. A fresh independently recorded overlapping-line control
  reproduces the former 29px displacement and wrong caret selection. Native line
  precedence and nearby cluster edges now determine caret hits across separate
  fallback-font runs; source queries avoid materializing unrelated glyph cells.
  PDF line heights and the existing atomic-inline policy are retained.
- Share immutable source ancestry between native cluster boxes rather than copying
  an ancestor vector for every glyph; retained geometry remains valid after the
  snapshot is dropped.
- Bound shared native font-shaping memoization to 128 entries/8 MiB per face,
  releasing old run payloads during churn and skipping oversized cache entries
  while returning their genuine glyph results.
- Add an explicit outer-SVG viewport snapshot route with real native replaced
  sizing, CSS/attribute/intrinsic ratio handling and source ownership. Preserve
  every SVG descendant identity while marking shape/nested/foreignObject geometry
  unsupported. Forty-nine independently captured Chrome graphs verify full
  parent/viewport rectangles and client sizes. Physical layout scale is measured
  separately from JS DPR; native used-size conversion precedes fragment allocation
  and leaves returned affine geometry and existing PDF policies intact.
- Treat invalid CSS declaration fallback separately from computed variable
  invalidation and SVG animated-length defaults; parse real viewBox numeric
  grammar without accepting malformed repeated/trailing commas. Fresh browser
  controls retain invalid, signed, exponent, decimal and mixed-calc cases.
- Reject incomplete nonwhitespace glyph coverage instead of returning partial
  source selections. Verify real captured glyph-loss/capacity boundaries and
  cross-platform font policy; duplicate concurrent native shaping results leave
  shared cache accounting unchanged.
- Package the pinned Blink155 layout-unit notice with native viewport attribution.
- Attribute the HTML font-metric policy to pinned Blink155 sources and include
  their BSD notice in the reproducible npm, Python, WASM and binary notices.

## [0.3.5] — 2026-10-14

- Ship the MIT license and existing embedded-font notices with the actual npm,
  Python and MCP binary artifacts; validate their reproducible source copies.

- Add native live-tree layout snapshots with caller-owned source IDs, all-fragment
  provenance, nested affine transforms and real block/replaced geometry. The live
  path retains nonpainting layout boxes and removes PDF image-fit caps while
  existing PDF filtering, image page-fit and root layout policy remain unchanged. Twenty-one offline contracts
  and 69 independently recorded Chrome 155 geometry cases cover the seam; full
  inline/caret/scroll geometry remains explicit follow-up work. Distinguish actual
  display-none exclusion through unmapped ancestors from visible empty/flattened
  inlines so callers cannot invent zero rectangles from absent fragments.
- Fix live relative/sticky percentage heights and min/max heights to use the
  normal-flow containing block, and fixed boxes to use the viewport. Hidden
  directives and descendants now agree with their exclusion metadata.
- Fix live absolute positioning to use the padding-box containing block, including
  percentage dimensions, opposing insets and measured auto-height parents.
  Explicitly reject fixed descendants of transformed ancestors in both snapshot
  APIs until their full containing-block geometry is implemented; 67 supported
  Chrome graphs match numerically and two unsupported controls verify admission.
- Expand inset and quad border declarations before cascade winner selection so
  their side values retain declaration order, specificity and importance.
  Validate the complete shorthand before applying sides and substitute inherited
  custom properties before expansion; invalid computed variables unset winning
  sides instead of reviving older declarations.
- Retain real live HTML document-root formatting boundaries so body margins do
  not collapse through the root and both first/last margins contribute to its
  actual box. Legacy PDF root policy remains covered separately.
- Correct stale release instructions and remove an unsafe optional regex capture
  read in the conformance CLI; benchmark detection reports the current release.

## [0.3.4] — searchable and copyable text in ordinary PDFs

### Fixed
- **Corrupted text extraction with subsetted fonts** ([#17](https://github.com/miaskiewicz/turbo-html2pdf/issues/17)):
  every embedded Type0 font now includes a `/ToUnicode` CMap mapping the actual
  subset glyph codes to UTF-16BE text. Search, selection/copy, and `pdftotext`
  work without enabling PDF/UA tagging, including `turbo-html2pdf-wasm-fonts`.
  Font object allocation reserves the mapping stream in every render to keep
  page resources, images, metadata, and conformance objects from colliding.

## [0.3.3] — text watermark centred on the page (both axes)

### Fixed
- **Render-time text watermark landed off the page / rode high**: `paint_text` rotated
  the mark about the page centre but drew the word around the origin, so it swung off
  the page at every angle; and once that was corrected the baseline sat *on* the centre,
  leaving the whole word riding ~14–18 pt above it (DRAFT at 48 pt on A4). The word is now
  centred on both axes — horizontally on its advance, vertically on the font's metric box
  (`baseline = cy − (ascent + descent) / 2`) — and rotates in place at any angle.

### Merged
- The 0.3.x layout-conformance line and the 0.2.15 `stamp()` feature are unified on this
  release: `stamp()` (and its N-API binding) ship alongside the conformance harness and
  the `layoutBoxes` seam.

## [0.3.2] — selector-list comma split respects `:is()`/`:not()` arguments

### Fixed
- **Comma inside a functional pseudo split the selector list**: `parse_selector_list`
  split the prelude on every `,`, so a comma *inside* `:is(...)` / `:where(...)` /
  `:not(...)` / `[attr="a,b"]` was treated as a list separator. Wikipedia's collapse
  sheet ships
  `.client-js .mw-collapsed:not(.mw-made-collapsible) > :is(p,table,thead + tbody){display:none}`;
  the naive split tore `:is(p,table,thead + tbody)` into a **bare `table` selector with
  no ancestor context**, so `display:none` matched *every* table on the page. On the Nike,
  Inc. article this dropped the `float:right` infobox entirely — its logo/photo images and
  facts table vanished and the body text spanned the full column instead of wrapping beside
  it. The list is now split on **top-level commas only** (paren/bracket/quote aware), so
  `:is()`/`:where()`/`:not()` argument lists and attribute-value commas stay intact and the
  infobox floats right with its images + rows, matching Chromium.

## [0.3.1] — `<br>` inline forced line-break + sub/sup conformance

### Fixed
- **`<br>` phantom line-height**: the UA sheet declared `br { display: block; height: 1em }`,
  making every `<br>` a separate 1em-tall block *on top of* the flow break, so multi-line
  `<br>` text was double-spaced / too tall vs Chromium. A `<br>` is now a proper **inline
  forced line break** — it ends the current line and continues on the next, contributing no
  box of its own. Each line box carries the block's **line-box strut** (its `line-height`),
  so the break advances by the line-height *in effect* and honors the cascade: default
  (font line-height), a parent/class `line-height`, and an inline-style `line-height` on the
  line's content all match Chromium (empirically ≤ 1.3px). An empty line from consecutive
  `<br><br>` is exactly one strut tall (not a 1em phantom, not zero); a trailing `<br>` adds
  no phantom line. Matches Chromium's behavior that a `<br>`'s *own* `line-height` is ignored
  — the surrounding inline content governs the terminated line.
- **`vertical-align: super`/`sub` on inline-blocks**: atoms now honor `super`/`sub` (shifted
  off the baseline by `valign_shift`), previously ignored (they sat on the baseline).

### Added
- **Conformance fixtures (68 → 73)**: four `<br>` cases — `69-br-default-line-height`,
  `70-br-parent-line-height-override`, `71-br-inline-line-height-override`,
  `72-br-double-empty-line` (the three cascade cases + the empty-line case) — and
  `73-sub-sup-vertical-align`, which asserts the `super`/`sub` baseline-shift offset against
  Chromium's measured `~0.3383em` / `~0.2050em`. The engine's existing `valign_shift`
  factors (`0.33` / `0.2`) already match within ~1.3px, so **no factor tuning was needed**.
  All 74 boxes across the new/old gated fixtures stay within 2px (no regression).

## [0.3.0] — layout conformance harness + foundational layout fixes

A box-geometry **conformance harness** (`benches/conformance/`) diffs laid-out element
geometry against Chromium per standard HTML/CSS feature. It surfaced — and this release
fixes — **8 foundational layout bug families**, then was **expanded to 68 fixtures** across
the modern-CSS constructs real pages exercise (Grid, calc, aspect-ratio, transforms,
`@media` height, logical props, viewport units, replaced images), surfacing **7 more
engine fixes**.

> ⚠️ **Behavioral change to DEFAULT layout output** (UA margins + table-cell padding,
> margin collapsing, flex / grid / inline-block / float placement, transform geometry,
> `vh`/height-`@media` resolution). Shared engine: it affects both the PDF pipeline and
> turbo-surf's raster. **Validate against downstream real-site renders (nike / wiki /
> google) before shipping.** 655 workspace tests pass, but unit tests ≠ real-site fidelity.

### Added
- **Conformance harness** (`benches/conformance/`): now **68 fixtures** (was 30) — block
  flow → adversarial flex/float/abs combos, plus full coverage of the six previously-zero
  families: **CSS Grid** (span, template-areas, single-auto-column, justify-items,
  minmax/fr/gap, min-content rows), **calc()** (`calc(% ± px)` length + inset, `calc()`
  media feature), **aspect-ratio**, **2D transforms** (translate, scale + `transform-origin`),
  **`@media` height/width queries**, and **logical / flow-relative props** — plus viewport
  units, `box-sizing:inherit`/border-box height, `white-space:nowrap`, tables
  (border-spacing, empty rows), replaced `<img>` sizing, sticky/overflow/positioning combos,
  and UA-default guards. Box-geometry diff vs Chromium, fonts pinned to bundled Inter on
  both sides, skip-safe when Chromium is absent. **Result: 189/189 boxes within 2px, all 65
  gated fixtures fully clean**; 3 documented deferrals (below) are tracked, not gated.
- **Harness deferral mechanism**: a `<!-- conformance:defer <reason> -->` marker reports a
  known-hard fixture's box deltas as `XFAIL` without reding the suite — so open gaps are
  tracked honestly instead of hidden or force-passed.
- **Image intrinsic-size probing for GIF and WebP** (`image::probe`): header-only,
  dependency-free (GIF logical-screen descriptor; WebP `VP8 `/`VP8L`/`VP8X` chunks). Sizes
  the box for layout; pixel decode/paint for these formats remains a follow-up (an
  unsupported decode emits nothing, never a wrong image). PNG/JPEG (and gated SVG) unchanged.
- **Data-URI image decode in the conformance seam** (`layout_boxes`): base64 `data:` image
  URIs are decoded so replaced-`<img>` fixtures reach their real intrinsic size. Self-
  contained (no I/O); the default render path still takes a host resolver.
- **`layoutBoxes(html, css, width, height)` napi export** + `layout_boxes` / `CidBox` core
  helper: dumps laid-out `{cid,x,y,width,height}` for elements tagged `data-cid`. Additive
  — drives the harness, no effect on render output. The read-back now applies a box's CSS
  2D `transform` (AABB of the transformed corners) so it matches `getBoundingClientRect`.

### Fixed (default layout output — behavioral)
- **UA default stylesheet**: browser-standard `h1`–`h6` / `p` margins, `ul` / `ol` margins
  + `padding-left:40px`.
- **Negative margins**: CSS 2.1 collapse (largest-positive + most-negative).
- **Parent/child margin collapse**: a first child's top margin collapses through a
  borderless/paddingless parent (recursive); the document root is excluded (root margins
  never collapse — matches browsers).
- **Inline-block**: honor `vertical-align` (top/bottom/middle/baseline), reserve inter-atom
  whitespace, apply atom margins in the line box.
- **Flex sizing**: apply taffy's resolved item height on read-back; parse the `flex`
  shorthand `<basis>`; a nested flex container inherits its parent-assigned definite height
  so `align-items:center` centers.
- **Floats + absolute**: float margins offset placement and register the margin box; an
  absolute `bottom` inset anchors against a definite-height positioned ancestor.
- **`border-collapse: collapse` (fixed layout)**: shared cell edges now merge onto a
  collapsed grid — each border counted once, half on each side (CSS 2.1 §17.6.2) — so
  table/row/cell boxes size correctly across a `colspan` (fixture 29 was ~4–5px wide of
  Chromium; now ≤0.6px). Non-colspan collapsed tables (fixture 07) went from 1px to exact.
  Auto-layout collapsed tables remain a documented deferral.

### Fixed (corpus expansion — 7 more engine fixes)
- **Grid `justify-self`**: a grid item's own `justify-self:center`/`start`/`end` is mapped
  to taffy — without it a `width:80px; justify-self:center` cell inherited the default
  `stretch` and filled its whole track instead of centering (google's home-logo cell).
- **`transform-origin`**: parsed into `(x, y)` `<length-percentage>` (keywords `left`/`top`
  =0, `center`=50, `right`/`bottom`=100; either keyword order) and resolved against the box
  size — previously the origin was hard-pinned to the box centre, so `transform-origin:top
  left` (and any non-centre origin) placed a `scale`/`rotate` wrong.
- **Viewport threading in the layout seam**: `vh`/`vmin`/`vmax` units and `@media
  (min/max-height)` conditions now resolve against the actual layout-viewport height
  (previously both fell back to the 800px default, so `20vh` measured against 800 and every
  height-`@media` matched at any viewport).
- **Shrink-to-fit floors at min-content**: an auto-width `inline-block` is sized
  `min(max-content, max(available, min-content))` — a `white-space:nowrap` box wider than
  its parent now overflows at its full width instead of being clamped and dropping text
  (Wikipedia menu tabs). Was a bare `.min(available)`.
- **UA default `td` / `th` padding: `1px`**: matches browsers, so an unpadded table cell's
  border box is 2px larger per axis (Chromium parity) — surfaced by the new
  `border-spacing` and empty-spacer-row fixtures.
- **Grid `min-content` / `max-content` tracks**: a `grid-template-rows:min-content` (or
  `-columns`) track now sizes to its items' content instead of mapping to `auto` and
  stretching to fill the container (the row hugs its tallest cell).
- **Inline replaced-image sizing** in a `<div>`/flex wrapper is exercised end-to-end now
  that the conformance seam decodes `data:` intrinsics.

### Deferred (tracked in the harness, not gated)
- **Auto-layout collapsed-table min-content width** (`53-table-auto-min-content`): the
  separate-border case is close, but auto-layout collapsed widths still diverge — the same
  `table.rs` follow-up noted above.
- **`visibility:hidden` reserves layout space** (`65-visibility-hidden-reserves-space`): the
  engine drops the box from layout (conflating it with `display:none`) so the following
  sibling pulls up. Paint-drop is correct; layout-space reservation is the open nuance.
- **Auto-inset absolute flex child** (`63-abs-auto-inset-flex-child`): an intentional
  divergence — the engine anchors it to its static start (0.2.14 fix for google's AI-Mode
  icon), while Chromium spec-centers it via `justify-content` in this isolated repro.
  Reverting to match the minimal repro would regress the shipped real-site fix.

## [0.2.15] — stamp an existing PDF

A new `stamp()` entry point that overlays a page watermark onto an **already-rendered
PDF** — no re-layout, the input PDF is taken as-is and a mark is drawn on every page.

### Added
- **`stamp(pdf, opts)`** — overlays a faded watermark (shaped-word or text-only) on
  every page of an existing PDF. Reads each page's media box and composites the mark
  behind/over the content, then re-emits the document.
- **Bundled stamp font** — the overlay shapes and embeds a bundled font (replacing the
  base-14 Helvetica placeholder), with an optional caller `font` (TrueType/OTF bytes)
  override on the text mark.
- **Encrypted-PDF support** — an encrypted input is decrypted around the overlay and
  the stamped output is re-encrypted through turbo's own encryptor (qpdf-compatible).
- **N-API binding** — `stamp()` and its `StampOptions` / text-mark types are exposed
  through `turbo-html2pdf` for Node.

## [0.2.14] — real-site rendering, round 2 (nike.com cards / google.com search bar)

More layout fixes found rendering production home pages faithfully.

### Added
- **`calc(% ± px)`** length support (a new mixed `LengthPct::Calc { pct, px }`), so
  `width:calc(100% - 616px)` and `top:calc(66.6% - 48px)` resolve instead of falling
  to `auto` — Nike's nav width and its editorial-card text overlays.
- **`aspect-ratio`**: an auto-height box derives its height from the content width
  (`width / ratio`), so a square media tile is sized by ratio rather than collapsing
  to a `min-height` fallback (Nike's editorial cards were too short, and the image +
  overlay overflowed and were covered by the next card).
- **Flow-relative margins/padding**: `*-inline-start/-end` and `*-block-start/-end`
  longhands map to the physical sides (LTR / `horizontal-tb`), so Google's search-bar
  labels honor their `margin-inline-start`.

### Fixed
- An `absolute` overlay with a `%`-height / `bottom` inset inside an **auto-height
  positioned** card now resolves against the card's measured content height (a
  deferred second pass) instead of collapsing and dropping below the card.
- `top`/`bottom` insets on an out-of-flow box resolve against the containing block's
  **height**, not its width (only square containing blocks landed right before).
- An auto-inset **absolute flex child** sits at its static start position instead of
  being centered by the container's `justify-content` (Google's "AI Mode" sparkle
  icon was printing over the label).

## [0.2.13] — real-site rendering (google.com / nike.com home pages)

A batch of layout + paint fixes that make production home pages render faithfully.

### Added
- **`flex` shorthand** grow/shrink expansion, so a `flex:1` item grows without an
  explicit `flex-grow` longhand (google's search box no longer crams its icons left).
- **Button `<input>` value as a text label** — `<input type="submit|button|reset">`
  renders its `value` (google's "Google Search" / "I'm Feeling Lucky" were blank).
- **Viewport units** `vw`/`vh`/`vmin`/`vmax` in length resolution.
- **Lazy `<img>`** reads a `data-*-url`/`srcset` when it has no `src` (nike's hero
  images ship the URL only in `data-landscape-url`).
- **CSS Grid `grid-column`/`grid-row` line placement** including `span N` — nike's
  12-column header (`grid-column:span 6`) no longer collapses each item to one track.

### Fixed
- **CSS Grid alignment**: `justify-items` is mapped, and a grid's `justify-content`
  defers to taffy's `normal`=stretch default so a single auto column fills the
  container — google's home logo now centers instead of pinning to the left padding.
- **`box-sizing:inherit`** is resolved (the explicit keyword), and the `<html>`/`<body>`
  shell styles are threaded as the inheritance parent, so the reset
  `html{box-sizing:border-box}` + `*{box-sizing:inherit}` reaches the page — nike's
  `width:50%` editorial cards now sit two-up instead of stacking half-width.
- **`box-sizing` for `px` `height`/`min-height`/`max-height`** — a border-box length
  includes its padding+border, so google's "Sign in" pill is a 40px pill, not a 62px oval.
- **`.ttc` face index** is carried through subsetting and outline tracing, so a glyph
  traces from its selected sub-font (macOS Arial/Helvetica are collections) instead of
  a shifted one.
- **Inline-block/atomic width** counts toward a line's natural + min-content, and
  **`align-self`** + a shrink-to-fit box's **`min`/`max-width`** are honored — google's
  footer "Settings", the header "Sign in", and the search-bar buttons no longer clip.
- **Text max-content is rounded up** so a shrink-wrapped box doesn't drop its last
  glyph to a second line (nike utility links, google's "How Search works").
- **Replaced `<img>`** flex/grid items and a `<div>` wrapping an `<img>` size to the
  image's intrinsic width; `align-items:center` items size to content, not the offer.
- **In-flow `%` height** resolves against a definite parent height.

## [0.2.12] — `@media` height conditions (min-height/max-height)

### Fixed
- **`@media (min-height:…)` / `(max-height:…)` are now evaluated.** The cascade only
  tested `min-width`/`max-width` and silently ignored height features, so a rule like
  `@media (max-height:575px){.x{display:none}}` matched at every viewport — Google's
  homepage hides its tall search box below `max-height:575px`, so the box was always
  `display:none` and the page rendered blank. Height is threaded from the caller
  (`set_media_viewport_height`, defaulting to 800px; the screenshot tier passes its
  canvas height) and `min/max-height` are evaluated alongside width.

## [0.2.11]

CSS `transform`: carousels, slide decks, and overlays that position with
`translate`/`rotate`/`scale`/`matrix` now transform instead of piling up.

### Added
- **CSS 2D `transform`.** `transform: translate*/scale*/rotate/skew*/matrix`
  parses into a `RawTransform` (linear part + `<length-percentage>` translate) on
  `FragmentContent::Box`, resolved at layout (with `transform-origin`, default box
  centre) into a `Transform2D` for the raster to apply to the box + its subtree.

## [0.2.10]

Real-page sizing: images and flex boxes no longer collapse to nothing on
content-driven layouts (Nike's hero/product imagery, search bars, hero banners).

### Fixed
- **Image `%` sizing.** `<img>` `width`/`height` percentages resolved against a 0
  basis, collapsing every `width:100%` responsive image to a 0×0 (invisible) box.
  Percentages now resolve against the containing block (`SizeCtx::cb_height` added
  for `%` height against a definite CB height); `auto`/unresolvable `%` falls back
  to the intrinsic aspect ratio.
- **Absolute containing-block height.** A positioned ancestor's definite content
  height is threaded (`abs_cb_h`) so an `<img height:100%>` in a sized hero card
  gets a real height.
- **Flex `height`/`min-height`/`max-height`.** The taffy flex container and item
  styles dropped these, so a flex box sized only by a `min-height` (`min-height:50px`
  search bar, hero banner) collapsed to its content height. Now passed through.

## [0.2.9]

Overlay chrome, round two: `linear-gradient(...)` backgrounds so hero sections,
buttons, and cards paint their gradient fills instead of a flat colour.

### Added
- **CSS `linear-gradient(...)` backgrounds.** `background`/`background-image:
  linear-gradient(...)` parses into a `LinearGradient` (angle + positioned colour
  stops) on `FragmentContent::Box`, exported as `turbo_html2pdf_core::{LinearGradient,
  GradientStop}` for raster consumers. Supports an angle (`<deg>`) or `to <side>`/`to
  <corner>` direction (default `to bottom`), `rgb()/rgba()/#hex/named` stops with
  optional `%` positions (unpositioned stops spread evenly; endpoints default 0/1),
  and nested-comma-safe parsing. `radial-`/`conic-gradient` are unsupported (→ no
  gradient, the solid `background` colour still applies). A gradient hides the solid
  background colour, matching CSS paint order.

## [0.2.8]

Overlay chrome: `box-shadow` so cards and modals read as raised chrome instead
of flat rectangles.

### Added
- **CSS `box-shadow`.** The first (topmost) shadow layer parses into a `BoxShadow`
  on `FragmentContent::Box` — `[inset]? <offset-x> <offset-y> <blur>? <spread>?
  <color>?` in any color position, comma-separated layers (first kept), color
  defaulting to `currentColor`, bare `0` accepted as a length. Exported as
  `turbo_html2pdf_core::BoxShadow` for raster consumers to stamp + blur behind the
  box. Inset shadows are flagged but painted outer-only by the v1 raster.

## [0.2.7]

Real-page fidelity, round two: the fixes that make Wikipedia render like Chromium,
plus the CSS features complex sites rely on (mask-image icons, `white-space`,
data-URI values, self-referential design tokens).

### Added
- **CSS `mask-image` icons.** `mask-image`/`-webkit-mask-image: url(...)` on a box
  paints its `background-color` (falling back to `color`) *through* the mask's alpha
  (a tinted `Image` fragment) instead of a solid rectangle — Wikipedia's UI glyphs
  (language/menu/ellipsis/edit), Codex icon fonts.
- **`white-space: nowrap` / `pre`.** A nowrap run's inter-word spaces are no longer
  line-break opportunities, so menu tabs/buttons stay on one line and size to their
  full text.
- **Data-URI values.** The declaration parser no longer splits on a `;`/`,` inside
  `url()`, so a `data:image/svg+xml;utf8,<svg…>` mask (or background) survives whole.

### Fixed
- **Pseudo-element selectors don't leak onto their element.** `::before`/`::after`/
  `::first-line`/… now match nothing (turbo generates no pseudo-elements) instead of
  applying their declarations to the originating element. Wikipedia's
  `.vector-page-titlebar::after{height:1px}` was collapsing the real title bar so the
  `<h1>` overlapped the tabs.
- **Self-referential `var()` resolves to its fallback.** `--x: var(--x, 1rem)`
  (Codex's redefine-from-inherited idiom) no longer spins to the depth cap and leaves
  an unresolved `var()` inside `calc()` — which had zeroed every `.vector-icon` box.
- **Flex fidelity.** An item's `width` drives its basis when `flex-basis:auto`; a
  row's max-content includes item margins; flex/grid items and table cells contain
  their floats (independent formatting context). Fixes full-width headers, non-wrapping
  margin-spaced tab rows, and float-clearfix bars that collapsed.
- **Tables** grow to their columns' min-content (no clipped/squeezed infoboxes),
  respect a separate-model `border-spacing`, and never shrink a column below its
  min-content.
- **`<input type=hidden>`** no longer paints a default input box.
- **`background`/`mask` images fill the resolved content-box height**, so an empty
  icon box sized only by `height` paints.

### Performance
- Memoize per-box intrinsic widths (`natural_width`/`min_content_width`) and the
  flex/grid measure-by-width — a deeply nested flex tree was exponential (Wikipedia's
  header/menus); now linear.

## [0.2.6]

Real-page fidelity: a large batch of layout + cascade fixes that let complex sites
(Wikipedia, Hacker News) render faithfully. (0.2.5 was staged but never released;
0.2.6 supersedes it and carries everything below.)

### Added
- **CSS custom properties + `var()`.** `--*` properties inherit; `var(--name,
  fallback)` is substituted in every value after the cascade (balanced-paren aware,
  multiple/nested refs, depth-guarded). Design-system / CSS-in-JS layouts that
  drive widths/flex via `var()` now resolve instead of collapsing to defaults.
- **Sibling combinators (`+`, `~`) + stateful/structural pseudo-classes**:
  `:not()`, `:checked`, `:enabled`, `:disabled`, `:root`, `:empty`,
  `:only-child`, `:first/last/only-of-type`, `:link`/`:any-link`. Interactive
  pseudos (`:hover`/`:focus`/`:active`/`:target`/`:visited`) parse but never match
  (static resting state), so hover-revealed menus stay hidden. The lexer is
  paren-aware (`:nth-child(2n+1)` / `:not(…)`).
- **Inline-block / `<img>` flow within the line** (were stacked below): atomic
  inlines share the line box with text, baseline-aligned, wrapping as words.
- **`grid-template` shorthand** (`<rows> / <cols>` + named areas) — Vector's whole
  page grid. Without it the axes fell back to AUTO tracks which, with named areas,
  made taffy content-size a huge subtree per track (a >90s hang + a giant
  zero-height container). Also **legacy table `cellpadding`** and **`<br>`/`<hr>`**
  rendering, plus basic **form-control** styling (`input`/`textarea`/`select`/
  `button`).
- **Optional system-font loading** (`FontRegistry::load_system_fonts`, opt-in).

### Fixed
- **Visually-hidden / sr-only content no longer paints.** `clip:rect(...)`,
  `clip-path:inset(50%|100%)`, and 0/1px `overflow:hidden` boxes are dropped —
  otherwise their (usually `position:absolute`) text rendered and piled at the
  containing block's origin. This was the Wikipedia "overlapping text" pile.
- **Auto-inset `position:absolute` uses its static position**, not the containing
  block's origin, per CSS. Every no-offset absolute deep in a page (navbox labels,
  decorations) was jumping to the top-left and piling. In isolation the static
  position ≈ the CB origin, which is why minimal repros passed while real pages
  broke.
- **Float text wrap**: in-flow content flows *beside* a float (narrowed column)
  instead of clearing below it, so text wraps next to a `float:right` infobox.
- **`@media(min-width:…)` with no space after `@media`** now parses (was dropping
  the whole block — the desktop infobox-float rule never applied).
- **`:link` colours apply** (were lumped with never-match pseudos), **percentage
  table width no longer double-applied** (`width:85%` columns collapsed to 85% of
  85%), **`<style>` text is stripped** from the visible flow, **empty table rows
  honor explicit `height`** (spacer rows), **`text-align` resets inside tables**,
  and **width-constrained blocks center** (`margin:auto` / `text-align` /
  `<center>`).
- **`visibility:hidden` / `opacity:0` boxes are dropped**, and the **`background`
  shorthand** is honored.

## [0.2.5]

### Added
- **CSS positioning + z-index in layout** (drives turbo-surf's synthetic
  screenshots; PDF benefits from the out-of-flow placement). Boxes now honor
  `position: relative | absolute | fixed | sticky` and their `top`/`right`/
  `bottom`/`left` insets:
  - **Out-of-flow** (`absolute`/`fixed`) boxes are removed from normal flow (they
    no longer push their siblings) and placed against their containing block — the
    nearest positioned ancestor's content box, or the page origin for `fixed`.
  - **`relative`** boxes are painted shifted by their insets while still reserving
    their normal-flow space; `sticky` is treated as `relative` (no scroll
    container in a paged/snapshot render).
  - Every `Fragment` carries the used `z-index` and an `is_positioned` flag and
    exposes `Fragment::paint_z()` + `Fragment::paint_order()` — a *stable*
    back-to-front child ordering (CSS 2.2 §9.9: negative-z, then in-flow
    non-positioned, then `z:auto`/`0` positioned, then positive-z) that painters
    use so overlapping menus/modals layer correctly.
  - The children `Vec` keeps its top-down layout order, so the paginator's flow
    walk is unchanged, and PDF emit is not reordered (its walk also drives
    pdf-ua marked-content reading order, which stays logical).
  - Known approximations (documented in code): `%` `top`/`bottom` insets and
    `bottom`-anchoring resolve against the containing block *width* (its height is
    unknown mid-layout); positioning is special-cased for block flow only.
- **`layout_html_with_images`** in the Jinja-free `html_layout` drive: like
  `layout_html` but takes an `ImageCtx`, so a caller holding final HTML *and*
  fetched image bytes (e.g. turbo-surf screenshots) gets `<img>`/`background-image`
  boxes sized into `Image` fragments to paint. Unresolvable images fall back to
  the image-free box exactly as `layout_html`.

- **`float: left`/`right`** (was ignored → boxes stacked full-width). Floated
  boxes are pulled out of block flow and packed to the left/right edge in a float
  band (wrapping to a new row when full; auto-width floats shrink to content).
  Following in-flow content clears below the band. A pragmatic model — no per-line
  text wrap around a float — but it fixes float-based columns / horizontal float
  navs that previously stacked vertically.
- **`inline-block` flows horizontally** (was stacked one-per-row). Atomic inlines
  now lay left-to-right on a row and wrap when the row fills; an auto-width
  `inline-block` shrinks to its content (via the flex `natural_width` measurement)
  instead of filling the whole line. Replaced `<img>` and explicit-width boxes keep
  their own sizing. Fixes nav bars / button rows / badge strips that previously
  stacked vertically.
- **CSS Grid layout** (`display: grid`/`inline-grid`). taffy (already the flex
  backend) owns the grid algorithm; the engine maps `grid-template-columns`/
  `-rows` (`fr`, `px`/`rem`, `%`, `auto`, `minmax(min, max)`, and integer
  `repeat(N, …)` tracks), **`grid-template-areas` + `grid-area` named placement**,
  `gap`/`row-gap`/`column-gap`, and `justify-content`/`align-items`. Items place by
  `grid-area` name (resolved to line spans) or auto-flow. Named areas are how
  content-heavy sites (Wikipedia's Vector skin: sidebar + body + rail) lay out.
  Numeric `grid-row`/`grid-column` line placement is still deferred. `inline-flex`
  also maps to flex. Modern pages are grid/flex-heavy — a large fidelity win.

- **Legacy presentational attributes** map to CSS (presentational hints, just
  above the UA sheet, below any author rule): `bgcolor` → `background-color`,
  `width`/`height` → lengths, `<font color>` → `color`. Old table-layout sites
  (Hacker News' orange `<td bgcolor>` header, sized `<img>`) now paint their
  backgrounds/sizes.

- **`@media` queries are now applied** (were parsed then dropped, so every rule
  inside a media block was ignored — i.e. the entire responsive/desktop layer of
  real sites). A matching `@media` block's rules join the cascade at their sheet's
  level (after the top-level rules). Conditions supported: comma lists (OR), the
  `screen`/`all` types (`print` never matches — we render screen), and
  `min-width`/`max-width` in `px`/`em`/`rem`. Width is evaluated against the
  viewport: `build_cascade` defaults to 1280px desktop; new
  `build_cascade_with_width` lets the screenshot tier pass its real viewport so
  responsive stylesheets pick the right breakpoint (this is what lets Wikipedia's
  desktop grid layout apply at all).

- **Optional system-font loading** (`FontRegistry::load_system_fonts`, opt-in —
  **not** used by default PDF rendering, which keeps only shipped/bundled faces).
  Registers every installed OS font under its own family (macOS/Linux/Windows
  dirs, `.ttf`/`.otf`/`.ttc`) and aliases the CSS generics to system families
  (`sans-serif`→Helvetica/Arial, …). Lets a screenshot match a browser on the same
  machine when a page names installed/system fonts. `FontFace::from_bytes_index`
  (`.ttc` faces) + `face_count`/`describe` read a font's own family/weight/style.

### Fixed
- **`visibility:hidden` / `opacity:0` boxes are dropped** (were rendered). These
  hide an element + subtree; painting them dumped content meant to be revealed on
  hover/click — e.g. Wikipedia's nav dropdowns, whose reveal rule
  (`:checked ~ …`) we don't apply, rendered fully expanded. Now they don't paint.
- **`background` shorthand is now honored.** The cascade only read the
  `background-color`/`background-image` longhands, so `background: #fff url(...)
  no-repeat` (which real stylesheets use pervasively) set neither the box's
  background colour nor its background-image — a page laid out with no backgrounds
  at all. Both are now recovered from the shorthand (a colour token, and/or a
  `url(...)`), with the longhand still winning when both are set.

## [0.2.4]

### Added
- **Public Jinja-free HTML→Fragment drive** in `turbo-html2pdf-core`: `parse_html`,
  `collect_style_css`, and `layout_html(html, extra_css, width, fonts, diags)` —
  lay a raw/final HTML string out into a positioned `Fragment` galley **without**
  the minijinja templating pass. For callers that already hold final HTML (e.g. a
  hydrated DOM snapshot) where `{{ }}`/`{% %}` are page content, not template
  syntax. Lets external consumers (e.g. turbo-surf's synthetic screenshots) reuse
  the native layout + font engine and paint the `Fragment` display list
  themselves. No PDF/emit/pagination/template-render code is touched; the default
  build and its byte output are unchanged.

### CI
- aarch64-linux release builds run on native `ubuntu-24.04-arm` runners (napi,
  wasm/svg, and the mcp binary) instead of cross-compiling.

## [0.2.3]

### Changed
- Renamed the core crate `turbo-pdf-core` → **`turbo-html2pdf-core`**.

## [0.2.2]

### Added
- Publish `turbo-html2pdf-core` + `turbo-html2pdf-mcp` to crates.io.

## [0.2.1]

### Added
- Publish the `turbo-html2pdf-mcp` server binary as per-platform archives on each
  GitHub Release.

## [0.2.0]

### Added
- **`turbo-html2pdf-mcp`** — a native MCP server (stdio JSON-RPC 2.0) exposing
  `render` / `append_pdf` / `check_template` to agents.
