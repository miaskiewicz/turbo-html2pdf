# turbo-html2pdf templating DSL

turbo-html2pdf renders a PDF from a **template** (HTML markup with embedded
[MiniJinja](https://github.com/mitsuhiko/minijinja) template logic and `t:`
paged-media directives), some **data**, and optional **CSS**. This document
describes the templating layer: the Jinja base, the `{% switch %}` extension,
the document-domain filters, and the `now()` function.

For paged-media features (running headers/footers, footnotes, pagination) see
[paged-media.md](paged-media.md). For the supported CSS subset see
[css-support.md](css-support.md). For the JS/React/WASM bindings see
[api.md](api.md).

> **Accuracy note.** Everything here is verified against the source and the test
> suite (`crates/turbo-pdf-core/tests/`). Examples are taken from working tests.
> Where the [spec](spec.md) describes features the current code does not yet
> implement, those are flagged as *deferred* and not documented as working.

---

## 1. The Jinja base (MiniJinja)

The template engine is MiniJinja, configured at compile time in
`crates/turbo-pdf-core/src/template/mod.rs`:

- **Auto-escaping is on** (HTML). Interpolated values are HTML-escaped, so
  `{{ '<b>' }}` renders `&lt;b&gt;`. Use the `| safe` filter to emit raw markup:
  `{{ '<b>x</b>' | safe }}` → `<b>x</b>`.
- **Block trimming is on** (`trim_blocks` + `lstrip_blocks`): the newline after a
  block tag and leading whitespace before a block tag on a line are stripped.
- **Recursion / include depth is bounded** (configurable via compile options).

All the standard MiniJinja constructs work. Verified in the test suite:

### Expressions and interpolation

```jinja
Hello {{ name }}
{{ a.b[c].d }}
{{ a["b"][0]["d"] }}
{{ 'ab' | upper | length }}      {# built-in filters chain #}
```

Arithmetic and comparison with correct precedence:

```jinja
{% if 2 + 3 * 4 == 14 %}Y{% else %}N{% endif %}
{% if invoice.total > 1000 and customer.tier == "pro" %}OK{% endif %}
```

### Conditionals

```jinja
{% if s == "paid" %}P{% elif s == "overdue" %}O{% else %}D{% endif %}
{% if tier in ["pro", "plus"] %}Y{% else %}N{% endif %}
```

### Loops

`loop.index`, `loop.index0`, `loop.first`, `loop.last`, `loop.length`, and the
`{% else %}` empty fallback all work:

```jinja
{% for x in xs %}{{ loop.index }}:{{ loop.first }}:{{ loop.last }}:{{ loop.length }};{% endfor %}
{% for x in xs %}{{ x }}{% else %}EMPTY{% endfor %}
{% for k, v in d | items %}{{ k }}{% endfor %}
```

### Includes and macros (partials)

Partials are registered as named templates via compile options (`partials`):

```jinja
{% include 'addr' %}
{% import 'm' as m %}{{ m.hi('Ola') }}
```

Include recursion is capped; exceeding the depth raises an
`IncludeDepthExceeded` error.

### Built-in filters and tests

MiniJinja built-ins come through unchanged — `upper`, `length`, `default`,
`items`, `safe`, `round`, the `is` tests, etc. For example:

```jinja
{{ x | default("—") }}
```

### Missing-value policy

How undefined variables behave is governed by the `missingPolicy` compile
option (`strict` / `lenient` / `empty`). Under a strict policy `{{ missing }}`
raises an `UndefinedValue` error; under a lenient/empty policy it renders empty.

### How data is referenced

In the **body**, the caller's data fields are referenced at the top level:
`{{ name }}`, `{{ invoice.total }}` — *not* `{{ data.name }}`.

Inside a **running header/footer region**, the data is nested under `data`
(alongside the per-page `page` context), so you write `{{ data.doc }}`. See
[paged-media.md](paged-media.md#running-headers-and-footers).

---

## 2. The `{% switch %}` extension

turbo-html2pdf adds a `{% switch %}` / `{% case %}` / `{% default %}` block on top of
MiniJinja (`crates/turbo-pdf-core/src/template/switch.rs`). It is a compile-time
desugaring into `{% if %}` / `{% elif %}` / `{% else %}`, so it has standard
Jinja semantics underneath.

```jinja
{% switch tier %}
  {% case "enterprise" %}E
  {% case "pro", "plus" %}P
  {% default %}S
{% endswitch %}
```

With `tier = "enterprise"` this renders `E`; `"pro"` or `"plus"` → `P`; anything
else → `S`.

Rules (all enforced and tested):

- **The subject is evaluated once.** `{% switch (a + b) %}` binds the value once
  via an internal `{% set %}`, so a subject with side effects is not
  re-evaluated per case.
- **Comma-separated case values are membership** — `{% case "pro", "plus" %}`
  matches either. Case values can be variables, not just literals:
  `{% case lo, hi %}` matches when the subject equals `lo` or `hi`.
- **First match wins.** `{% case 1 %}A{% case 1 %}B` yields `A`.
- **`{% default %}` must be last and unique.** A `default` before a `case`, or a
  second `default`, is a `TemplateSyntax` compile error.
- **No text before the first `{% case %}`.** Whitespace is allowed; other text
  is a compile error.
- **Non-taken cases have no side effects** — a `{% set %}` inside an unmatched
  case does not leak into the namespace.
- **Whitespace control** works on the switch tags: `{%- … -%}` trims adjacent
  text just like on `if`.
- A stray `{% case %}`, `{% default %}`, or `{% endswitch %}` outside a switch,
  and an unterminated `{% switch %}`, are `TemplateSyntax` compile errors.

---

## 3. Document-domain filters

These filters are registered in addition to the MiniJinja built-ins
(`crates/turbo-pdf-core/src/template/filters.rs`). Every example below is a
verbatim test assertion.

### `currency(value, ccy, locale="en")`

Formats a money amount with thousands grouping, **two** decimals, and a currency
symbol placed per locale. A negative sign sits *outside* the symbol.

- Symbols: `USD`→`$`, `EUR`→`€`, `GBP`→`£`, `JPY`→`¥`, `PLN`→`zł`. An unknown
  code is used as a prefix: `currency(1, "xyz")` → `XYZ 1.00`.
- Locale controls grouping/decimal separators and symbol placement. The
  "euro-style" locales (`pt`, `de`, `es`, `it`, `nl`, `pl`, `fr` — matched on the
  language prefix) use `.` thousands, `,` decimal, and a **trailing** symbol
  separated by a non-breaking space. Everything else uses `,`/`.` and a
  **leading** symbol.

```jinja
{{ 1234.5 | currency("USD") }}            → $1,234.50
{{ 1234.5 | currency("EUR", "pt-PT") }}   → 1.234,50 €     (NBSP before €)
{{ -5 | currency("GBP") }}                → -£5.00
{{ 10 | currency("JPY") }}                → ¥10.00
{{ 10 | currency("PLN", "pl") }}          → 10,00 zł        (NBSP before zł)
{{ 1 | currency("xyz") }}                 → XYZ 1.00
```

### `number(value, decimals=auto)`

Grouped number. With no `decimals` it uses 0 decimals for integers and 2 for
fractional values. Negatives keep their sign. (Uses the `en` grouping: `,`
thousands, `.` decimal.)

```jinja
{{ 1000 | number }}        → 1,000
{{ 1000.25 | number }}     → 1,000.25
{{ 1000 | number(2) }}     → 1,000.00
{{ -5 | number }}          → -5
```

### `percent(value, decimals=0)`

Multiplies by 100 and appends `%`.

```jinja
{{ 0.25 | percent }}        → 25%
{{ 0.125 | percent(1) }}    → 12.5%
```

### `ordinal(n)`

English ordinal suffix.

```jinja
{{ 1 | ordinal }}    → 1st
{{ 2 | ordinal }}    → 2nd
{{ 3 | ordinal }}    → 3rd
{{ 4 | ordinal }}    → 4th
{{ 11 | ordinal }}   → 11th
{{ 22 | ordinal }}   → 22nd
```

### `pad(value, width, fill=" ")`

**Left-pads** the stringified value to `width` characters with `fill` (first
character of `fill` is used; default space). A value already at/over `width` is
returned unchanged.

```jinja
{{ 7 | pad(3) }}          → "  7"
{{ 7 | pad(3, "0") }}     → "007"
{{ 12345 | pad(3) }}      → "12345"
```

### `truncate(value, length, suffix="…")`

Shortens to `length` characters *including* the suffix, when the value is
longer; otherwise returns it unchanged.

```jinja
{{ 'hi' | truncate(5) }}                → "hi"
{{ 'hello world' | truncate(5) }}       → "hell…"
{{ 'hello world' | truncate(6, "..") }} → "hell.."
```

### `wordwrap(value, width)`

Greedy word wrap at `width` characters; words are joined by single spaces and
lines by `\n`.

```jinja
{{ 'aa bb cc' | wordwrap(5) }}    → "aa bb\ncc"
{{ '' | wordwrap(5) }}            → ""
```

### `date(value, fmt="YYYY-MM-DD", tz="UTC")`

### `datetime(value, fmt="YYYY-MM-DD HH:mm:ss", tz="UTC")`

`date` and `datetime` are the same function with different default formats. They
accept a Unix timestamp (seconds), an RFC 3339 string, or a `YYYY-MM-DD` string.

**Format tokens** (everything else passes through literally): `YYYY` `YY` `MM`
`DD` `HH` `mm` `ss`.

**Timezone** is an offset: `UTC` or `Z` for zero, or a signed `±HH:MM` offset
(the sign and the colon are required — `0200` and `+0200` are both errors).

`date`/`datetime` are also callable as **functions**, not just filters — useful
for header field codes: `{{ date(now(), "YYYY-MM-DD") }}`.

```jinja
{{ 0 | date }}                                      → 1970-01-01
{{ 0 | datetime }}                                  → 1970-01-01 00:00:00
{{ "2020-06-14" | date("DD.MM.YY") }}               → 14.06.20
{{ ts | datetime("YYYY-MM-DD HH:mm:ss", "+02:00") }}→ 2020-06-14 12:00:00
                                                      (ts = "2020-06-14T10:00:00Z")
{{ 0 | datetime("HH:mm", "Z") }}                    → 00:00
{{ 0 | datetime("HH:mm", "-05:30") }}               → 18:30
```

A value that is neither a recognized date string nor a timestamp (e.g. a
non-integer number like `1.5`, or `"not-a-date"`) is a render error.

> Note on `/` in formats: `/` is HTML-escaped by auto-escaping, so a format like
> `DD/MM/YY` would render escaped. The tests use `.` separators (`DD.MM.YY`).
> Wrap with `| safe` if you need a literal `/`.

---

## 4. `now()`

`now()` returns the **pinned** render clock as a Unix timestamp in seconds. It is
*not* the wall clock: the caller pins it per render (the `now` render option) for
deterministic output. If the caller does not pin a clock, calling `now()` is a
render error.

```jinja
{{ date(now(), "YYYY-MM-DD") }}     → 1970-01-01   (when now is pinned to 0)
```

This is the building block for "generated on" stamps that stay reproducible.
