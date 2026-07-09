# Changelog

All notable changes to turbo-html2pdf are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/); versions follow SemVer. The npm,
PyPI, and crates.io packages release in lockstep from a `v*` tag (PyPI on `pyv*`).

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
