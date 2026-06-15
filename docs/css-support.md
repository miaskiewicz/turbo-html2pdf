# Supported CSS subset

turbo-html2pdf styles the document with a small, well-defined CSS subset. This page
documents exactly what the code parses and applies, verified against
`crates/turbo-pdf-core/src/style/` and `crates/turbo-pdf-core/src/layout/value.rs`.
Anything not listed here is **not** supported and is ignored.

The internal unit is **CSS pixels at 96 dpi**. The default font size is `16px`.

---

## 1. Selectors

Supported simple selectors (`style/selector.rs`):

| Form | Example |
|---|---|
| Type / tag | `div` |
| Universal | `*` |
| Class | `.total` |
| ID | `#footer` |
| Attribute | `[data-x]`, `[type="text"]` |
| Pseudo-class | `:first-child`, `:last-child`, `:nth-child(…)`, `:nth-of-type(…)` |

Attribute operators: `[a]` (exists), `[a=v]`, `[a~=v]` (word), `[a|=v]`
(dash-match), `[a^=v]` (prefix), `[a$=v]` (suffix), `[a*=v]` (substring). Values
may be quoted.

`:nth-child` / `:nth-of-type` accept `even`, `odd`, an integer, or `An+B`
(`2n+1`, `n`, `-n`, `3n`, …).

**Combinators:** descendant (whitespace, `div p`) and child (`>`, `div > p`).
**Sibling combinators (`+`, `~`) are not supported.**

**Not supported:** pseudo-elements (`::before`), and any pseudo-class other than
the four above (`:hover`, `:not()`, `:checked`, … are silently dropped).

**Specificity** is the standard `(ids, classes+attrs+pseudo-classes, types)`
tuple. The cascade layers UA styles below author CSS below inline node styles.

---

## 2. Syntax the parser accepts

`style/parser.rs`:

- `/* … */` block comments (no `//` line comments).
- Qualified rules `selector-list { decl; decl; … }`; comma-separated selector
  lists; `;`-separated `property: value` declarations.
- `!important` on a value.
- At-rules `@name prelude { body }` are sliced out and kept verbatim for later
  phases (e.g. `@page`); the parser itself does not interpret them.

**Not supported:** `@import`, custom properties / `var()`, CSS nesting, escapes.

---

## 3. Units

`layout/value.rs`:

| Unit | Notes |
|---|---|
| `px` or unitless | base unit |
| `pt` | 1pt = 1/72 in |
| `pc` | 1pc = 1/6 in |
| `in` | 1in = 96px |
| `cm` | |
| `mm` | |
| `%` | resolved against the relevant context basis |
| `em` | relative to the element's font size |
| `rem` | **treated the same as `em`** (resolves against the parent font size, not the root) |

No `ex`, `ch`, `vh`, `vw`, `vmin`, `vmax`, `Q`, `fr`. Unknown units cause the
value to be rejected.

---

## 4. Colors

Three forms (`layout/value.rs`):

1. **Hex** — `#rgb`, `#rrggbb`, `#rrggbbaa`. (4-digit `#rgba` is **not**
   supported.)
2. **Functional** — `rgb(r, g, b)` / `rgba(r, g, b, a)`. Channels are numbers
   (0–255, clamped); the optional alpha is 0–1. **Percentage channels (`50%`) are
   not supported.** Channels may be comma- or slash-separated.
3. **Named colors** — exactly these (case-insensitive):
   `black`, `white`, `red`, `green` (`#008000`), `blue`,
   `gray` / `grey` (`#808080`), `transparent`.

Any other name (`orange`, `navy`, `currentColor`, …) is unrecognized. The default
text color is `black`.

---

## 5. Properties

Properties resolved into layout (`layout/value.rs`). Anything not listed is
ignored.

### Box model

| Property | Accepted values |
|---|---|
| `display` | `block`, `inline`, `inline-block`, `flex`, `none`, `table`, `table-row`, `table-cell`, `table-header-group`, `table-footer-group`, `list-item` (anything else → `block`) |
| `position` | only `relative` is meaningful (offsets a relatively-positioned box); other values are not positioned |
| `margin`, `margin-top/right/bottom/left` | 1–4 length shorthand; longhands override; `auto` supported |
| `padding`, `padding-*` | 1–4 length shorthand + longhands |
| `border`, `border-top/right/bottom/left`, `border-*-width`, `border-*-color` | width as a length or keyword (`thin`=1, `medium`=3, `thick`=5, `none`=0) plus a color |
| `width`, `height` | length / `%` / `auto` |
| `min-width`, `min-height` | length (default `0`) |
| `max-width`, `max-height` | length / `%` / `auto` |
| `box-sizing` | `border-box` or `content-box` (default) |
| `background-color` | any color (the `background` shorthand is **not** parsed) |

### Text

| Property | Accepted values |
|---|---|
| `font-family` | comma-separated list; quotes stripped |
| `font-size` | length / `em` / `rem` / `%` (relative to parent) |
| `font-weight` | `normal` (400), `bold` (700), or a number |
| `font-style` | `italic` or `oblique` → italic; else normal |
| `color` | any color |
| `line-height` | `normal`, a unitless multiplier, or a length |
| `text-align` | `left` (default), `right`, `center`, `justify` |
| `white-space` | `normal` (default), `pre`, `nowrap` |
| `letter-spacing` | a length (default `0`) |
| `vertical-align` | `baseline` (default), `sub`, `super`, `middle`, `top`, `bottom` |

### Break control

| Property | Accepted values |
|---|---|
| `break-before` | `auto` (default), `avoid`, `page` (`column` is treated as `page`) |
| `break-after` | same |
| `break-inside` | `avoid` is the only meaningful value |
| `orphans` | integer (default `2`) |
| `widows` | integer (default `2`) |

See [paged-media.md](paged-media.md#break-rules) for how these drive pagination.

---

## 6. UA (default) stylesheet

The built-in user-agent stylesheet (`style/mod.rs`) sits at the bottom of the
cascade. **Any tag with no rule defaults to `display: block`.** Non-block /
styled defaults:

**Inline elements:** `a`, `span`, `small`, `b`, `strong`, `i`, `em`, `code`,
`kbd`, `samp`, `sub`, `sup`, `abbr`, `cite`, `q`, `mark`, `u`, `s`, `label`,
`time`.

**Table elements:** `table` → `table`, `thead` → `table-header-group`, `tfoot` →
`table-footer-group`, `tr` → `table-row`, `td`/`th` → `table-cell`. `li` →
`list-item`.

**Typographic defaults:**

| Selector | Declaration |
|---|---|
| `b`, `strong` | `font-weight: bold` |
| `i`, `em` | `font-style: italic` |
| `a` | `color: #0000ee` |
| `h1` | `font-weight: bold; font-size: 2em` |
| `h2` | `font-weight: bold; font-size: 1.5em` |
| `small` | `font-size: 0.8em` |
| `sub` | `vertical-align: sub` |
| `sup` | `vertical-align: super` |

Note: the UA sheet declares **no default margins, padding, or list markers**, and
`h3`–`h6` get no size/weight bump — set those yourself in author CSS.

---

## 7. Images — deferred

> **Image embedding is not implemented in the current code.** The layout layer
> explicitly defers an `Image` content type to a later phase, and there is **no
> image overflow cap** (no "max-width 100% of containing block / max-height ~60%
> of page body height") in the source. `<img>` content is not laid out yet.
>
> The [spec](spec.md) describes raster image support (PNG/JPEG XObjects) and the
> JS/WASM bindings accept an `images` input, but that input is currently a no-op
> (`TODO(phase9b)`). Do not rely on images rendering yet.
