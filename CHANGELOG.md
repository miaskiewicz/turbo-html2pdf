# Changelog

All notable changes to turbo-html2pdf are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/); versions follow SemVer. The npm,
PyPI, and crates.io packages release in lockstep from a `v*` tag (PyPI on `pyv*`).

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
