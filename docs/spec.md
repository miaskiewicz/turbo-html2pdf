# turbo-html2pdf — Implementation Specification

A native (Rust) HTML/CSS-to-PDF engine with a templating DSL, fronted by a thin TS/N-API layer. The engine's only input contract is **parseable template HTML/CSS that contains the required fields** — it is framework-agnostic and has no knowledge of React, Vue, Svelte, or any other tool. A template is compiled once to a reusable Program, then executed against data to emit PDF bytes. No browser, no DOM at the hot path, threadable concurrency.

**Two principles that shape everything below:**

1. **The core is platform/app-agnostic.** Authoring frontends (React, Vue, plain template strings, hand-written HTML) are *separate, optional* layers that all emit the same template HTML + `t:` DSL. The core neither knows nor cares which produced its input. React is documented in §8.4 as *one* such frontend, explicitly not built into the core and not required.

2. **Pagination is automatic by default.** Content defines a *flow* and (optionally) a default page geometry; the engine paginates it — overflow generates new pages on its own. Authors do **not** define pages up front. Explicit page masters, named pages, and forced breaks (§3) are *overrides* layered on top of auto-pagination, never a precondition for producing a PDF.

This document is the build target. It is written for an implementing agent. Sections are ordered so that each can be built and tested against acceptance criteria before the next is started. Wherever a behavior is observable, it has an acceptance criterion (AC-x.y). Wherever a behavior is a choice, the choice is stated and an alternative is noted.

---

## 0. Scope, non-goals, and vocabulary

### 0.1 In scope
- A **framework-agnostic** core: input is template HTML/CSS + DSL. No authoring framework is built in. Any frontend that emits valid template HTML works.
- **Automatic pagination by default**: a content flow is paginated into pages without the author declaring pages. A default page geometry (size/margins) is the only page-level input required, and even that has a built-in fallback (A4, sane margins).
- A **Jinja2-compatible** templating layer (MiniJinja) for data + control flow: interpolation, dict/array access, `if/elif/else`, `for` with `loop`, infix expressions, filters, tests, macros, includes — plus `t:` structural elements for paged-media constructs that must be DOM nodes.
- Paged-media constructs as *optional overrides* on top of auto-pagination: default + named page masters, running headers/footers with page context, footnotes, page/total-page counters, named running elements (e.g. current section title in the header), leaders, forced/avoided breaks.
- A CSS subset sufficient for documents (block + inline flow, flexbox, tables, fonts, color, borders, backgrounds, the paged-media properties below).
- A global style registry with named style tokens that nodes reference, plus normal CSS cascade.
- A compile/render split: `compile(template) -> Program`, `Program.render(data) -> Pdf`.
- N-API binding for Node; WASM build as a secondary target. Authoring-frontend packages (React first) shipped separately.

### 0.2 Non-goals (v1)
- Arbitrary modern web layout (grid, floats, multi-column flow regions, position: absolute relative to viewport, transforms, filters, animations).
- JavaScript execution inside templates. The DSL is intentionally not Turing-complete.
- Bidi/complex-script shaping beyond what `rustybuzz` gives for free (RTL is a v2 line item, see §12).
- Loading remote resources at render time. All assets (fonts, images) are provided by the caller or resolved through a caller-supplied loader. No network in the core.
- Editing or parsing existing PDFs.

### 0.3 Vocabulary
- **Template**: HTML containing DSL directives + CSS. Parsed once.
- **Program**: the compiled, data-independent artifact (an op list + style tables + page masters). Cacheable, serializable.
- **Data / context**: the JSON-like value tree a Program is rendered against.
- **Box tree**: styled, unpositioned layout nodes.
- **Fragment tree**: positioned boxes after layout, before pagination.
- **Page**: one output page with placed fragments and margin-box content.

---

## 1. Pipeline overview

```
Any authoring frontend ──(emits template HTML + t: DSL)──> Template HTML (with DSL directives)
   (React/Vue/strings/hand-written; separate, optional, NOT part of the core)  │
                                                                      compile()  [Rust]
                                                                      │
                                            ┌─────────────────────────┴───────────────────────┐
                                            │  Stage A: parse template -> directive AST          │
                                            │  Stage B: lower AST -> Program (op list)           │
                                            │  Stage C: parse CSS -> style tables + page masters │
                                            └─────────────────────────┬───────────────────────┘
                                                                      │  Program (cacheable)
                                                              render(data)  [Rust]
                                                                      │
                       Stage 1 render Jinja + parse markup ──> resolved node tree (in-memory, not serialized)
                       Stage 2 style resolution ────> styled box tree (computed values per node)
                       Stage 3 layout ──────────────> fragment tree (positioned, single infinite page)
                       Stage 4 fragmenter ──────────> page list (breaks, repeated headers, footnotes, margin boxes)
                       Stage 5 emit ────────────────> PDF bytes
```

Hard rule: **no HTML string is produced between `render` stages.** Stage 1 hands a node tree to Stage 2 in-process. (AC-1.1: a render of a 10k-row table allocates zero intermediate HTML `String` for the body; verifiable by an allocation-counting test harness or a `#[cfg(feature="alloc-audit")]` counter.)

---

## 2. The templating language — Jinja control flow + `t:` structural elements

The template language is **Jinja2-compatible** (MiniJinja semantics) for all data and control flow — interpolation, conditionals, loops, expressions with infix operators, filters, tests, macros, includes. Paged-media constructs that must survive into the parsed DOM as typed layout nodes (footnotes, running headers/footers, page masters, named running elements) are expressed as **namespaced `t:` HTML elements**, because a Jinja block tag is gone by the time you have a tree, whereas a `<t:footnote>` element is a real node the layout engine can recognize.

Rationale for this split (decision, with the alternative noted): Jinja is widely understood, has stable documented semantics, infix operators (`{% if total > 1000 %}` not helper soup), and an excellent embeddable Rust implementation (MiniJinja). The control-flow layer should be *boring and familiar* — novelty budget belongs in the layout/pagination engine. We keep stock Jinja for everything except (i) the `t:` structural elements that must be DOM nodes, and (ii) a single readability extension, `{% switch %}` (§2.6), for many-way dispatch; templates that avoid `{% switch %}` are fully portable to any Jinja engine. The rejected alternatives were (a) a fully custom `t:` DSL for everything — more to learn, no ecosystem, syntax highlighting, or prior mental model; and (b) pure Handlebars — no infix operators, and its string-in/string-out helper model fights the structural constructs even harder than Jinja's. We keep `t:` elements *only* where structure genuinely requires a DOM node.

**Implementation:** wrap/embed MiniJinja for the expression + statement layer rather than authoring a grammar. The engine intercepts `t:` elements after HTML parsing (they pass through Jinja untouched as literal markup, then html5ever yields them as element nodes). (AC-2.0: a template using only Jinja constructs — no `t:` elements — renders correctly through the MiniJinja-backed evaluator; a separate fixture confirms `t:` elements survive Jinja rendering verbatim as element nodes.)

### 2.1 Design principles
1. Templates remain parseable by a standard HTML5 parser *after* the Jinja pass. Jinja statements (`{% … %}`) and expressions (`{{ … }}`) are resolved against data first; the resulting markup — including any `t:` elements the template emitted — is then parsed by html5ever. `t:` elements are custom elements html5ever preserves as unknown-element nodes.
2. Two evaluation phases, one logical step: (a) MiniJinja renders control flow + interpolation against data, emitting markup; (b) html5ever parses that markup into the node tree, where `t:` elements become typed layout directives. Both happen inside `render`; no HTML string escapes to the caller (the §1 hard rule still holds — the intermediate markup is an internal `Cow`/rope, see AC-1.1).
3. Totality / strictness: under the default `strict` undefined behavior (§2.9), referencing an undefined variable or out-of-range index raises a typed error with template span, never silently produces empty string. This is MiniJinja's `UndefinedBehavior::Strict`.

> Note on the two-phase model vs the original single-pass op-list: rendering Jinja to intermediate markup then parsing is simpler to build (you inherit MiniJinja) and the markup is small and short-lived. The `compile` step still pays off: MiniJinja templates are *parsed and cached* once into a `minijinja::Template`, and the `t:`/CSS/page-master structural analysis that doesn't depend on data is also done once. Only the data-dependent Jinja render + parse happens per `render`. (AC-2.0b: the MiniJinja template object and all data-independent structural tables are built in `compile` and reused across renders; per-render work is render+parse+layout only.)

### 2.2 Expression language
MiniJinja's expression grammar — documented and stable. Key points the implementing agent must preserve (do not subset these away):
- **Infix operators:** `+ - * / // % **`, comparisons `== != < <= > >=`, logical `and or not`, membership `in`, string concat `~`, ternary `a if cond else b`. (AC-2.1: `{% if invoice.total > 1000 and customer.tier == "pro" %}` evaluates with correct precedence.)
- **Path access:** `a.b`, `a["b"]`, `a[i]` over dicts/arrays; `a.b` and `a["b"]` are equivalent for dict keys. (AC-2.2: `{{ a.b[c].d }}` resolves left-to-right.)
- **Filters:** `{{ value | filter(args) }}`, chainable. Custom filters registered at compile (`currency`, `date`, `number`, `default`, etc. — §2.12). (AC-2.3.)
- **Tests:** `{% if x is defined %}`, `is none`, `is even`, etc.
- **No arbitrary side effects / no I/O** in expressions — MiniJinja with the default (no `py`/unsafe) feature set; we expose only our registered filters/functions. The language is not Turing-complete in a way that can hang (loops are bounded collections; recursion via macros/includes is depth-capped, §2.8).

### 2.3 Value model
The data passed to `render` is `serde`-compatible and maps onto MiniJinja's `Value`. Internally:
```rust
// Caller passes any serde Serialize value; we hold it as minijinja::Value.
// Conceptual shape mirrors JSON:
//   none/unit, bool, number (i64/u64/f64), string, seq (array), map (object, insertion-ordered)
```
Truthiness follows Jinja: `none`/`false`/`0`/`""`/empty seq/empty map are falsy; everything else truthy. (AC-2.4: empty array is falsy; `[0]` is truthy; `"0"` is truthy — non-emptiness, not numeric parse — matching Jinja.) Note this differs slightly from the earlier custom model and is intentional: we adopt Jinja semantics wholesale so behavior matches developer expectations and the published Jinja docs.

### 2.4 Interpolation
- `{{ expr }}` — evaluates and **auto-escapes for HTML by default** (MiniJinja autoescape on for our `.html`-like templates). (AC-2.5: `{{ "<b>" }}` emits `&lt;b&gt;`.)
- Raw/unescaped output uses the `safe` filter or `{% autoescape false %}` block — the Jinja-native mechanism, not triple-mustache: `{{ prerendered_fragment | safe }}`. Discouraged; linted. Unescaped output that is then parsed as markup must be well-formed or it errors at the parse phase. (AC-2.6: `{{ "<b>x</b>" | safe }}` becomes real markup → a `<b>` element node; malformed `{{ "<b>" | safe }}` raises a parse error pointing at the producing expression.)
- Stringify rules for interpolated values follow Jinja, with our number formatting deferred to filters (`number`, `currency`); a bare map/seq stringifies via Jinja's repr — but for documents that is almost always a mistake, so we **lint** interpolation of a non-scalar and recommend a filter or iteration. (AC-2.7: interpolating a bare map emits a lint with the template span.)

### 2.5 Conditionals
Native Jinja:
```html
{% if invoice.status == "paid" %}
  …
{% elif invoice.status == "overdue" %}
  …
{% else %}
  …
{% endif %}
```
Exactly one branch renders; non-taken branches emit no markup, so no hidden nodes reach layout (contrast CSS `display:none`). (AC-2.8: a `{% set %}`/counter side effect inside a non-taken branch does not occur.)

### 2.6 Switch / multiway
Jinja has no `switch`; the idiomatic form is `{% if/elif %}`. For the common "match one of N" case we register a small set of equality/membership helpers so it stays readable:
```html
{% if customer.tier == "enterprise" %} … 
{% elif customer.tier in ["pro", "plus"] %} … 
{% else %} … {% endif %}
```
`in` is native Jinja membership. (AC-2.9: `tier in ["pro","plus"]` matches either; first matching branch wins.)

For genuine many-way dispatch where a chain of `{% elif %}` reads poorly, we ship a **`{% switch %}` extension** — a custom MiniJinja statement registered by the engine, not part of stock Jinja. It is sugar over the same `==`/`in` semantics and compiles to the same branch selection:
```html
{% switch customer.tier %}
  {% case "enterprise" %}
    <p>Dedicated support included.</p>
  {% case "pro", "plus" %}        {# multiple values = membership; first match wins #}
    <p>Priority support included.</p>
  {% default %}
    <p>Standard support.</p>
{% endswitch %}
```
Semantics, precisely:
- The `{% switch EXPR %}` subject `EXPR` is any Jinja expression, evaluated once. (AC-2.9b: subject evaluated exactly once even with many cases.)
- Each `{% case V1, V2, ... %}` lists one or more expressions; the case matches if the subject equals any listed value (`==`), i.e. comma = membership. (AC-2.9c: `{% case "pro", "plus" %}` matches either.)
- **First matching case wins**; remaining cases are not evaluated for match and their bodies do not render. (AC-2.9d: with two cases that both match, only the first body renders and the second case's value expressions are not evaluated.)
- `{% default %}` is optional, must be last if present, renders when no case matched. A `{% case %}` after `{% default %}`, or more than one `{% default %}`, is a compile error with span. (AC-2.9e.)
- Only `{% case %}`/`{% default %}` may be direct children of `{% switch %}`; text/markup directly between `{% switch %}` and the first `{% case %}` (other than whitespace) is a compile error. Whitespace-only is ignored. (AC-2.9f.)
- Non-taken case bodies emit nothing into the node tree (same guarantee as `{% if %}`, §2.5) — no hidden nodes, no side effects from `{% set %}` inside them. (AC-2.9g.)
- Cases may use complex value expressions, not just literals: `{% case lo, hi %}` compares the subject against the *values* of `lo`/`hi`. (AC-2.9h: a `case` with variable values matches by evaluated value.)
- Whitespace control markers work as elsewhere: `{%- switch -%}`, `{%- case -%}`, `{%- endswitch -%}`. (AC-2.9i.)

Decision: provide `{% switch %}` as a registered extension while keeping the rest of the language stock Jinja. It costs one custom statement (implemented against MiniJinja's extension/`add_*` API or a thin pre-parse, whichever the crate version supports cleanly) and buys real readability for tier/status/type dispatch, which is common in documents. Templates that avoid it remain 100% portable to any Jinja engine; templates that use it require turbo-html2pdf (documented as the one non-portable construct in the data/control-flow layer). Implementation note for the agent: if MiniJinja's public API in the pinned version doesn't allow a true custom block statement, implement `switch` as a source-level desugaring to `{% if/elif/else %}` performed before handing the template to MiniJinja, preserving spans for error reporting. (AC-2.9j: a `switch` template and its hand-written `{% if/elif/else %}` equivalent produce identical node trees.)

### 2.7 Iteration
Native Jinja `for` with the built-in `loop` object:
```html
{% for line in invoice.lines %}
  <tr>
    <td>{{ loop.index }}</td>           {# 1-based; loop.index0 for 0-based #}
    <td>{{ line.description }}</td>
    <td>{{ line.amount | currency(invoice.ccy) }}</td>
  </tr>
{% else %}
  <tr><td colspan="3" class="muted">No line items.</td></tr>
{% endfor %}
```
- Iterates seq or map (`{% for k, v in dict | items %}`). Map iteration order is insertion order (we preserve it through serde). (AC-2.10: dict iteration order stable = insertion order.)
- `loop` object is Jinja's: `loop.index`, `loop.index0`, `loop.first`, `loop.last`, `loop.length`, `loop.revindex`, `loop.cycle(...)`, and `loop.previtem`/`loop.nextitem`. (AC-2.11: `loop.first`/`loop.last` both true for length-1; `loop.length` correct.)
- `{% else %}` inside `{% for %}` is Jinja's empty-collection block — replaces the earlier `<t:empty>`. (AC-2.12: `for` over `[]` with `{% else %}` renders the empty block once.)
- Nested loops: inner `loop` shadows outer per Jinja; outer reachable via `loop` aliasing if needed. (AC-2.13: nested `for` keep distinct `loop` state.)

### 2.8 Partials, macros, includes
Native Jinja composition:
- `{% include "address-block" %}` — pulls a registered template into the current scope. (AC-2.14: a partial included 1000× is parsed once; MiniJinja caches the compiled template.)
- `{% import "macros" as m %}` then `{{ m.address(customer.billing, "Bill to") }}` — macros are the typed, parameterized reuse mechanism (cleaner than the old `with=` dict). (AC-2.15: a macro called with positional/keyword args renders with its own local scope; only passed args + globals visible.)
- `{% include %}` scope is the caller's context by default; use `{% with %}` or macros for isolation — Jinja-native. (AC-2.16.)
- Include/macro recursion is allowed but depth-capped (default 64) to prevent runaway. (AC-2.17: exceeding depth raises an `IncludeDepthExceeded`-class error with the chain.)

### 2.9 Undefined / missing policy
Maps directly to MiniJinja `UndefinedBehavior`:
- `strict` (default): undefined variable or out-of-range access raises a render error with span. (AC-2.18.)
- `lenient`/`empty`: undefined renders as empty (Jinja `Undefined` printing to `""`). (AC-2.19.)
- For "keep the literal for debugging," provide a debug render mode that emits `{{ path }}` placeholders; not a Jinja-native mode, implemented as a custom Undefined that stringifies to its own path. (AC-2.20.)
- The `default` filter is the per-expression fallback: `{{ x.maybe | default("—") }}` substitutes only for undefined/none (Jinja semantics), not for `""` or `0` unless `default(x, true)` boolean form is used. (AC-2.21.)

### 2.10 Whitespace control
Native Jinja whitespace control: `{%- … -%}` and `{{- … -}}` trim markers, plus `trim_blocks`/`lstrip_blocks` engine settings (we default both **on** for clean document markup). (AC-2.22: `{%- for -%}` trims surrounding whitespace per Jinja; `trim_blocks` removes the newline after a block tag.) After the Jinja pass, residual whitespace handling follows the CSS `white-space` of the containing box.

### 2.11 Comments
Native Jinja `{# … #}` — stripped during the Jinja pass, never reaches markup. (AC-2.23.)

### 2.12 Registered filters & functions (document-domain helpers)
Beyond Jinja built-ins (`upper`, `lower`, `length`, `join`, `default`, `round`, `abs`, `replace`, etc.), we register a documented set for documents, all locale-aware where relevant:
- `currency(value, ccy, locale=?)`, `number(value, opts)`, `date(value, fmt, tz=?)`, `datetime`, `percent`, `ordinal`, `pad`, `truncate`, `wordwrap`.
- Document/page functions usable in expressions: `page()`, `pages()`, `counter(name)`, `counters(name, sep)`, `ref(id, what)` (with the xref feature). These bridge into the paged-media state (§3). (AC-2.24: `currency(1234.5, "EUR", "pt-PT")` formats per locale; `date(ts, "YYYY-MM-DD")` formats correctly; unknown filter is a compile error with span.)

---

## 3. Pagination & paged-media DSL

Pagination is **automatic and on by default**. Everything in this section other than §3.0 is an *override* that refines the automatic behavior. A template with no page-related DSL at all still produces a correctly paginated multi-page PDF. These constructs are DSL (not pure CSS) because the optional pieces — running headers, footnotes, page counters — need template context (data + page state).

### 3.0 Auto-pagination (default model)

The author provides a **flow** (the document body) and, optionally, a default page geometry. The fragmenter (§6) breaks the flow into as many pages as the content needs. No page is declared in advance; pages come into existence as content overflows.

- **Zero-config:** a template with only body content and no geometry uses the built-in default geometry (size A4, margins `20mm`, no header/footer regions) and paginates automatically. (AC-3.0.1: a 5,000-word body with no page DSL renders to N≥2 pages, no content lost, no author-declared pages.)
- **Default geometry without masters:** geometry may be set without naming or declaring a master, via a bare `@page` rule or the `defaultPage` render/compile option:
  ```css
  @page { size: Letter; margin: 1in; }
  ```
  or
  ```ts
  compile(tpl, { defaultPage: { size: "Letter", margin: "1in" } })
  ```
  Both feed auto-pagination. No `<t:page-master>` needed. (AC-3.0.2: `@page` alone changes page size/margins for the whole document with no master declared.)
- **Headers/footers without masters (common case):** a `<t:running-header>` / `<t:running-footer>` element placed once anywhere in the flow attaches that content to the default geometry's top/bottom margin band for *every* auto-generated page. This is the ergonomic path so authors don't touch masters just to get "Page X of N":
  ```html
  <t:running-footer>Page <t:page/> of <t:pages/></t:running-footer>
  ```
  Region extent auto-sizes to the content's measured height (capped at the available margin) unless an explicit `extent` is given. (AC-3.0.3: a `<t:running-footer>` with no master produces the footer on all pages; extent derived from content height.)
- **Capacity per page** = page box − margins − (auto or declared header/footer extents) − footnote area (§6.4). Computed automatically each page. (AC-3.0.4.)
- **Precedence:** if any of {`<t:page-master>`, `<t:use-master>`, `t:master`} is present, the named-master machinery (§3.1+) governs the affected flow ranges; everywhere else, auto-pagination with default geometry applies. The two coexist in one document. (AC-3.0.5: a document with a `cover` master on its first section and nothing else still auto-paginates the remaining sections under default geometry.)

Masters (§3.1) exist for cases auto-pagination can't infer: distinct first-page/duplex geometry, multiple page sizes in one document, named margin-box layouts (e.g. four-corner running content), and mid-document geometry switches. If you don't need those, you never write a master.

### 3.1 Page masters (override)
A page master is an **optional** named bundle of geometry + margin boxes, used only when auto-pagination's default geometry is insufficient (§3.0 lists when). Declared once, referenced by name. Declaring a master does not turn off auto-pagination — content under a master is still auto-paginated into as many pages as needed; the master only supplies *geometry and regions*, never a fixed page count.

```html
<t:page-master name="default"
    size="A4" orientation="portrait"
    margin="20mm 18mm 22mm 18mm">
  <t:region slot="header" extent="14mm">
    <div class="hdr">
      <span>{{ doc.title }}</span>
      <span class="sec">{{ running.section }}</span>
    </div>
  </t:region>
  <t:region slot="footer" extent="12mm">
    <div class="ftr">
      Page <t:page/> of <t:pages/>
    </div>
  </t:region>
</t:page-master>
```

- `size`: named (`A4`, `Letter`, `Legal`, `A3`, `A5`) or explicit `WIDTHxHEIGHT` with units. (AC-3.1: unknown named size is a compile error listing valid names.)
- `margin`: CSS shorthand (1–4 values), units in `mm|cm|in|pt|px`. Margin boxes (header/footer regions) live inside the margin area.
- `slot`: one of `header`, `footer`, `top-left|top-center|top-right`, `bottom-left|bottom-center|bottom-right`, `left`, `right`. `header`/`footer` are sugar for full-width top/bottom regions. (AC-3.2: a `header` region and a `top-center` region in the same master is a conflict error — they occupy the same band.)
- `extent`: the height (for top/bottom) or width (for left/right) reserved for the region. Content area = page minus margins minus region extents. (AC-3.3: body content never overlaps a declared region; verified by a fixture where footer text and last body line would collide — they must not.)
- Multiple masters allowed. Selection per §3.4.

### 3.2 First / left / right / blank variants
```html
<t:page-master name="default" ...>
  <t:variant kind="first"> … overrides regions … </t:variant>
  <t:variant kind="left"> … </t:variant>
  <t:variant kind="right"> … </t:variant>
  <t:variant kind="blank"/>   <!-- intentionally empty page -->
</t:page-master>
```
- `kind` ∈ `first`, `left` (verso), `right` (recto), `blank`. The base master is the fallback. (AC-3.4: page 1 uses `first` if present else base; even/odd pages use `left`/`right` if present.)
- This is the mechanism for distinct first-page headers and mirrored margins for duplex print. (AC-3.5: `left`/`right` variants with mirrored `margin` produce mirrored geometry on facing pages.)

### 3.3 Region content and page context (Word-equivalent header/footer variables)
Inside any region (and inside any `t:running-header`/`t:running-footer`), an implicit `page` context and document `meta` are available. This is the equivalent of Word's header/footer field codes. All are usable as Jinja expressions and most have a convenience element.

Page-state variables:
- `page.number` — 1-based current page. Element: `<t:page/>`. (AC-3.6.)
- `page.total` — total pages, resolved after pagination (back-patched, §6.5). Element: `<t:pages/>`. (AC-3.7.)
- `page.roman` / `page.roman_upper` — current page as roman numerals (for front-matter numbering). (AC-3.6b.)
- `page.master` — active master name (or `"(default)"` under auto-pagination). 
- `page.kind` — `first | left | right | normal | blank`.
- `page.is_first` / `page.is_last` — booleans (e.g. suppress "continued" on the last page). (AC-3.6c.)
- `page.section_number` / `page.section_pages` — page number *within the current section* and total pages of that section, for "Page X of Y" scoped to a section (Word's `SECTIONPAGES`). Available when sections are declared (§3.4). (AC-3.6d.)

Document/meta variables (from the `meta` render input, §7):
- `meta.title`, `meta.author`, `meta.subject`, `meta.keywords`. 
- `now` and friends: `now()` plus `date(now(), fmt, tz)` for current date/time in the header (Word's `DATE`/`TIME`). Pinned via `RenderOptions.now` for determinism. (AC-3.6e: a header showing `{{ date(now(), "YYYY-MM-DD") }}` renders the pinned date when `now` is set.)
- `doc.*` — the full render data is in scope, so any field (e.g. `doc.invoice.ref`, a filename you passed in) can appear in a header. (AC-3.9: a footer `{% if doc.confidential %}CONFIDENTIAL{% endif %}` evaluates per page.)

Running content:
- `running.<name>` — current value of a named running element (§3.5), e.g. the current chapter/section title (Word's `STYLEREF`). (AC-3.8.)

Counters:
- `counter("figure")`, `counters("heading", ".")` — arbitrary named counters (§3.8) for "Figure 4" / "1.2.3" style values in regions. (AC-3.8b.)

Regions may use the full Jinja language (if/for/filters/macros) and reference all of the above. Convenience elements `<t:page/>`, `<t:pages/>` exist so non-Jinja authors can drop them in plain HTML, but `Page {{ page.number }} of {{ page.total }}` is the canonical form. (AC-3.7b: `<t:page/>` and `{{ page.number }}` produce identical output.)

### 3.4 Master selection from body
```html
<t:use-master name="default"/>          <!-- sets master for subsequent content -->
<section t:master="cover">…</section>     <!-- scoped override for one subtree -->
```
- `<t:use-master>` switches the active master from that flow position onward; forces a page break if mid-page. **Decision:** switching master forces a break (cleaner page accounting). (AC-3.10.)
- `t:master` attribute on a block scopes the master to that block's pages; pages generated for that block use the named master, reverting after. (AC-3.11.)

### 3.5 Named running elements
Lets a header pull "the current chapter/section title" — the classic running-header feature missing in browser-print.

```html
<h1 t:running="section">{{ chapter.title }}</h1>
…
<t:region slot="header"><span>{{ running.section }}</span></t:region>
```
- `t:running="name"` marks an element whose text content becomes the value of `running.name`. (AC-3.12.)
- Resolution rule (CSS `string-set`/`content: string(x, ...)` semantics): on any given page, `running.name` is the value as of the **last assignment that began on or before that page** (`first` policy variant available via `t:running-policy`). (AC-3.13: a section starting mid-page sets the running value for that page and onward until the next assignment; the page where it starts shows the new value if assignment is at/above the region's reference, else the previous — define and test the `start` policy: page shows the value in effect at page start, and a same-page assignment takes effect next page. **Decision: `start` policy is default**, matching most word processors.)
- `t:running-policy="start|first|last"` per element overrides. (AC-3.14: `last` makes the page show the final assignment occurring on that page.)

### 3.6 Footnotes
Footnote placement is **fully automatic and content-driven**: the author marks a footnote inline where the citation belongs, and the engine puts the note body on whatever page the marker ends up on after pagination — no page numbers, no manual association. This is the Word/LaTeX behavior. The author never says "this goes on page 4."
```html
<p>… as held in <t:footnote>Smith v. Jones, 123 F.3d 456 (9th Cir. 1999).</t:footnote> the court …</p>
```
- `<t:footnote>` emits an automatically-numbered superscript reference at its inline position and moves its children into the footnote area of the page **where the reference lands after pagination**. The footnote body may itself contain Jinja (`{{ }}`) and inline markup. (AC-3.15: a footnote whose reference is pushed to page 4 renders its body at the bottom of page 4, not page 3.)
- Numbering: document-continuous by default; `t:footnote-reset="page|section|none"` controls reset. (AC-3.16: `page` reset restarts numbering at 1 each page.)
- Footnote area: a reserved band above the bottom margin/footer region; grows upward as footnotes accumulate, shrinking the body content area on that page. The fragmenter must account for footnote height when deciding the page's body capacity (mutual constraint — see §6.4). (AC-3.17: a page with three footnotes reserves their combined measured height plus the separator; body text reflows to fit.)
- A footnote longer than remaining page space splits: as much as fits stays, the remainder continues in the footnote area of the next page with a "continued" affordance. **Decision:** support footnote continuation (legal documents need it). (AC-3.18: an oversized footnote splits across two pages' footnote areas without losing content.)
- Separator: `<t:footnote-separator>` region (default: a short rule). (AC-3.19.)
- `<t:footnote mark="*">` allows manual marks (symbols) instead of auto-numbers; manual and auto can coexist with independent sequences. (AC-3.20.)

### 3.7 Endnotes (v1 optional, behind feature)
`<t:endnote>` collects to a `<t:endnotes/>` sink placed anywhere in the flow. Lower priority; gate behind `feature = "endnotes"`. (AC-3.21: when enabled, endnotes appear at the sink in reference order.)

### 3.8 Counters (general)
A general named-counter facility underlying page/footnote numbering, usable for figures/tables.
```html
<t:counter name="figure" action="increment"/>
<figcaption>Figure {{ counter.figure }}: …</figcaption>
```
- `action` ∈ `increment` (default, by 1 or by `step`), `reset` (to `start`, default 0), `set`. (AC-3.22.)
- `counter.<name>` reads current value; `counters('name','sep')` filter produces nested values for hierarchical numbering (e.g. `1.2.3`). (AC-3.23.)
- Page and footnote counters are predefined and read-only via `page.*`. (AC-3.24: writing `page` counter is a compile error.)

### 3.9 Cross-references (v1 optional, behind feature)
`<t:anchor id="fig-1"/>` and `{{ ref('fig-1', 'page') }}` / `ref('fig-1','counter')`. Requires a two-pass resolution because target pages aren't known until pagination. Gate behind `feature = "xref"`. (AC-3.25: `ref(...,'page')` resolves to the final page number after pagination; unresolved id is a typed error.)

### 3.10 Leaders
`<t:leader>` or CSS `leader('.')` fills horizontal space between two inline boxes with a repeating glyph — for TOCs and footers. (AC-3.26: `Title <t:leader/> 42` dot-fills to the page number flush right within its container.)

---

## 4. Style system

### 4.1 CSS subset (v1 supported)
Layout: `display` (`block`, `inline`, `inline-block`, `flex`, `none`, `table`, `table-row`, `table-cell`, `table-header-group`, `table-footer-group`, `list-item`); `flex-direction|wrap|grow|shrink|basis`, `justify-content`, `align-items|self|content`, `gap`; box model (`margin`, `padding`, `border`, `width`, `height`, `min/max-*`, `box-sizing`); `position: static|relative` only (no absolute/fixed in v1; see §0.2). Text: `font-family|size|weight|style`, `line-height`, `color`, `text-align`, `text-decoration`, `letter-spacing`, `word-spacing`, `white-space`, `text-transform`, `vertical-align` (baseline/sub/super/middle/top/bottom for inline + table cells), `text-indent`, `hyphens`. Backgrounds/borders: `background-color`, `background-image` (raster only v1), `border-*`, `border-radius`, `box-shadow` (optional, behind feature). Paged: `size` (in `@page`), `margin` (page), `break-before|after|inside` (`auto|avoid|page|column`), `orphans`, `widows`. Lists: `list-style-type|position`.

Explicitly unsupported v1 (parse → ignore + lint, never error): `float`, `clear`, `grid-*`, `position: absolute|fixed|sticky`, `transform`, `filter`, `clip-path`, `mix-blend-mode`, CSS animations/transitions, `@media screen`. (AC-4.1: unsupported property emits a lint with property + node, output still renders.)

### 4.2 Cascade
- Standard origin/specificity/order cascade implemented over `cssparser` + `selectors`. Inline `style=""` highest non-`!important`. `!important` honored. (AC-4.2: specificity ordering matches a fixture table of 20 selector pairs.)
- Inheritance per CSS inheritance rules for inheritable properties. (AC-4.3.)
- `:first-child`, `:last-child`, `:nth-child()`, `:nth-of-type()`, attribute selectors, descendant/child/sibling combinators supported. Pseudo-elements: `::before`, `::after` with `content` (string, `counter()`, `attr()`, `string()`); `::first-line`, `::first-letter` (optional, behind feature). (AC-4.4: `td:nth-child(even)` zebra fixture matches.)
- Paged pseudo-classes `:first`, `:left`, `:right`, `:blank` apply within `@page` and map onto master variants (§3.2). (AC-4.5.)

### 4.3 Global style registry + named tokens
The "inject styles into nodes from a global set" requirement. Two cooperating mechanisms:

**(a) Stylesheets** — ordinary `<style>` blocks or external sheets supplied at compile:
```html
<style data-scope="global">.total{font-weight:700}</style>
```
Compiled into the cascade. `data-scope="global"` is documentation-only (all sheets are global in v1; scoped sheets are a v2 item).

**(b) Style tokens** — named, semantic bundles resolved by reference, decoupled from selectors:
```ts
compile(tpl, {
  tokens: {
    "emphatic":  { fontWeight: 700, color: "#0a0a0a" },
    "muted":     { color: "#666" },
    "total":     { extends: ["emphatic"], fontSize: "14pt" },
    "tabular":   { fontVariantNumeric: "tabular-nums" }
  }
})
```
```html
<span t:style="total tabular">{{ grand_total | currency }}</span>
```
- `t:style="a b c"` applies tokens in order; later tokens win on conflict; `extends` composes. Token-applied properties sit at a defined cascade level: **above author stylesheets, below inline `style=`**. **Decision:** tokens beat sheets so a global token can theme nodes predictably, but a one-off inline `style` still wins. (AC-4.6: node with class matched by a sheet rule AND `t:style` token setting the same property → token wins; adding inline `style` for that property → inline wins.)
- Tokens may reference CSS custom properties; `--token` variables defined globally resolve in tokens and sheets uniformly. (AC-4.7.)
- Programmatic injection: the caller can pass `nodeStyles: Array<{ match: Selector | NodeId, props }>` at **render** time (not just compile) to theme a specific render without recompiling. These enter at a level above tokens, below inline. (AC-4.8: same template, two renders with different `nodeStyles`, both correct, Program compiled once.)

Cascade level summary (low→high): UA defaults < author sheets < style tokens (`t:style`) < render-time `nodeStyles` < inline `style=` < `!important` (within its origin). (AC-4.9: a 6-level fixture verifies the full order.)

### 4.4 Fonts
- Caller supplies fonts via `fonts: [{ family, weight, style, data: Bytes | path, subset?: bool }]`. No system font lookup in core (deterministic output). A `systemFonts: true` opt-in may be added in the binding layer, never in core. (AC-4.10: identical inputs → byte-identical PDF given identical font bytes; determinism test.)
- Shaping via `rustybuzz`; metrics via the same face. Fallback chain per `font-family` list; a final caller-designated fallback covers missing glyphs; unresolved glyph → visible `.notdef` and a lint. (AC-4.11: a glyph absent from all provided fonts produces `.notdef` + lint naming the codepoint.)

---

## 5. Layout engine (Stage 3)

### 5.1 Box generation
Map computed `display` to box types: block, inline, inline-block, flex container/item, table/row/cell, list-item (marker box). Anonymous box generation per CSS (e.g. block context wrapping inline runs; anonymous table parts). (AC-5.1: a `<div>` mixing raw text and `<span>`s wraps bare text in anonymous inline boxes; fixture checks line count.)

### 5.2 Inline layout & text
- Itemize runs by script/font/direction; shape with `rustybuzz`; break opportunities via `unicode-linebreak`; optional hyphenation via `hyphenation` crate (Knuth-Liang) when `hyphens: auto` and a language is set. (AC-5.2: justified paragraph with `hyphens:auto` and `lang=en` hyphenates at valid points only.)
- Line breaking: greedy by default; `text-wrap: pretty|balance` (optional, behind feature) uses a secondary pass. Track `orphans`/`widows` constraints as hints consumed by the fragmenter. (AC-5.3.)
- Baseline alignment for mixed font sizes / `vertical-align`. (AC-5.4.)

### 5.3 Block & flex
- Block flow: vertical stacking, margin collapsing (adjacent + parent/child + empty-block collapse, per CSS). (AC-5.5: margin-collapse fixture set, 8 cases.)
- Flex: use `taffy` for flex (and block, if its block impl suffices; otherwise own block, taffy for flex). Document which engine owns which box type. **Decision:** `taffy` owns flex; engine owns block/inline/table to keep paged-media break hooks under our control. (AC-5.6: flexbox fixtures — grow/shrink/basis/wrap/gap/justify/align — match reference renders within 0.5pt.)

### 5.4 Tables
First-class because tables are the universal pain point.
- Two width algorithms: `fixed` (`table-layout:fixed`, from first row / explicit col widths) and `auto` (content-driven min/max width resolution). (AC-5.7: both algorithms match fixtures.)
- `<thead>`/`<tfoot>` recognized as `table-header-group`/`table-footer-group` and marked **repeatable** for pagination (§6.3). (AC-5.8.)
- `colspan`/`rowspan`, border-collapse vs separate, per-cell `vertical-align`, caption. (AC-5.9.)
- Cells participate in break decisions: `break-inside: avoid` on a row keeps it whole; a row taller than a page is allowed to split with a documented policy (split at line boundaries, repeat header). (AC-5.10: a row taller than the page splits; header repeats above each part.)

### 5.5 Output
A fragment tree: each fragment has absolute (x,y) in a single continuous coordinate space (the "galley"), size, the source node id, computed style ref, and break metadata (allowed break points, avoid regions, widow/orphan counts, repeatable flags, footnote payloads, running-element assignments). This galley is the fragmenter's input. (AC-5.11: fragment tree round-trips node ids so emitted PDF can be mapped back to template nodes for debugging.)

---

## 6. Fragmenter / auto-pagination (Stage 4)

The differentiator, and the component that makes pagination automatic (§3.0). It takes a single continuous galley and **decides page boundaries itself** — the author never supplies them. Geometry comes either from the default (§3.0) or from masters (§3.1) where present; the fragmenter consumes whichever applies to each flow range and emits as many pages as the content requires. Its job: turn the galley into discrete pages with margin boxes, repeated headers, footnotes, and break correctness, generating page N+1 the moment page N fills.

### 6.1 Inputs / outputs
- In: galley fragment tree, the active geometry per flow position (default geometry, or a resolved master where one applies), footnote payloads, running assignments, counter ops.
- Out: ordered `Vec<Page>` whose length is determined by content overflow, each with: resolved geometry (+ master/variant if any), body fragments (offset into page content box), header/footer/margin-box laid-out content, footnote-area fragments, resolved page counters. (AC-6.0: page count is an output, never an input; a fixture asserts the same template produces different page counts purely as a function of data volume.)

### 6.2 Break algorithm
- Walk the galley top-down accumulating height into the current page's available body height (page content height minus footnote-area height for this page, see §6.4).
- At each potential break point (between block-level siblings, between lines, between table rows), decide: fit-on-current vs push-to-next.
- Honor `break-before/after: page` (force), `break-inside: avoid` (treat subtree as unbreakable unit; if it can't fit on an empty page, allow internal break and lint), `orphans`/`widows` (don't leave fewer than N lines of a paragraph at a page boundary; if violated, push earlier lines forward). (AC-6.1: orphans=2/widows=2 fixture — no paragraph leaves 1 line stranded.)
- Forced and avoided breaks interact deterministically; document precedence: forced break > avoid > widows/orphans > greedy fit. (AC-6.2: precedence fixture.)

### 6.3 Repeated table headers/footers
- When a table spans a break, re-emit its `thead` at the top of the continued region and `tfoot` at the bottom of each page-part if `tfoot` is marked repeating (`t:repeat-footer`), else only on the final part. **Decision:** `thead` repeats by default; `tfoot` repeats only with opt-in (matches most expectations). (AC-6.3.)

### 6.4 Footnote ↔ body mutual constraint
- Footnotes referenced by fragments placed on page *p* must have their bodies laid out in *p*'s footnote area, which reduces *p*'s body capacity, which can change which fragments land on *p* — a fixpoint.
- Algorithm: tentatively place body to capacity assuming zero footnotes; collect referenced footnotes; measure their height; reduce capacity; re-place; iterate until stable or a max of K=4 iterations, then accept and lint if not converged. (AC-6.4: a page where adding a footnote pushes its own reference to the next page converges to a consistent placement — the reference and body are on the same page — or lints `FootnoteConvergence` and degrades gracefully without losing content.)
- Footnote continuation (§3.6) handled when even alone they exceed area. (AC-6.5.)

### 6.5 Running elements & counters resolution
- During the walk, maintain a live map of `running.*` per the active policy (start/first/last) and snapshot it per page for region rendering. (AC-6.6.)
- Page counters known only after the walk; total pages back-patched. Anything depending on totals (`<t:pages/>`, `ref(...,'page')`) is resolved in a final pass. (AC-6.7: `Page X of N` correct on every page including when content length is data-dependent.)

### 6.6 Region (header/footer/margin-box) layout
- Each page's regions are laid out via Stages 2–3 against a per-page context (`page.*`, `running.*`, document data). Region content height must fit its declared `extent`; overflow is clipped and linted. (AC-6.8: region content taller than `extent` is clipped at the extent boundary + lint, never overlaps body.)

---

## 7. PDF emitter (Stage 5)

- Use `pdf-writer` for low-level object/stream emission. Build: catalog, page tree, per-page content stream, resource dicts (fonts, XObjects for images), font programs (subsetted), optional document outline from `t:running`/headings, optional tagged-PDF structure tree (behind `feature = "pdf-ua"`, see §11). (AC-7.1: output opens in Acrobat/pdf.js/Preview without repair prompts; validated by `qpdf --check` clean.)
- Text: emit text-showing ops with shaped glyph ids and positions; embed subset fonts (`subsetter` crate or equivalent). (AC-7.2: a doc using 10 glyphs of a 3000-glyph font embeds a subset under ~10% of full size.)
- Color: device RGB v1; CMYK + ICC behind `feature = "print-color"`. (AC-7.3: RGB colors round-trip to expected values.)
- Images: raster (PNG/JPEG) as image XObjects; JPEG passed through as DCTDecode where possible. SVG support behind `feature = "svg"` (rasterize via `resvg`, or emit vector — vector is v2). (AC-7.4: a PNG with alpha emits with SMask; visual fixture matches.)
- Links: `<a href>` → link annotations; internal `t:anchor` → GoTo destinations (with `feature=xref`). (AC-7.5.)
- Metadata: title/author/subject/keywords/creation date from a `meta` input; XMP packet. Determinism: creation date defaults to a fixed sentinel unless caller sets it, so byte-identical output is achievable. (AC-7.6: two renders with identical inputs + pinned date → identical bytes.)
- PDF version target: 1.7 default; PDF/A-2b behind `feature = "pdf-a"`. (AC-7.7: with `pdf-a`, veraPDF validation passes for the conformance fixtures.)

---

## 8. Public API

### 8.1 Rust core
```rust
pub struct CompileOptions {
    pub partials: HashMap<String, String>, // name -> template source; Jinja include/import registry
    pub tokens: HashMap<String, StyleToken>,
    pub missing_policy: MissingPolicy,   // Strict | Empty | Keep
    pub include_max_depth: u32,          // default 64
    pub default_page: Option<PageGeometry>, // None => built-in default (A4, 20mm); feeds auto-pagination (§3.0)
    pub features: FeatureSet,
}
pub struct RenderOptions<'a> {
    pub data: &'a Value,
    pub fonts: &'a [FontFace],
    pub images: ImageResolver,           // trait: name -> bytes
    pub node_styles: Vec<NodeStyleRule>, // render-time injection
    pub meta: DocMeta,
    pub now: Option<DateTime>,           // determinism control
}
pub struct Program { /* opaque, Send + Sync, serializable */ }
pub struct Diagnostics { pub lints: Vec<Lint>, /* non-fatal */ }
pub struct RenderOutput { pub pdf: Vec<u8>, pub diagnostics: Diagnostics, pub page_count: u32 }

pub fn compile(template: &str, opts: &CompileOptions) -> Result<(Program, Diagnostics), CompileError>;
impl Program {
    pub fn render(&self, opts: &RenderOptions) -> Result<RenderOutput, RenderError>;
    pub fn to_bytes(&self) -> Vec<u8>;             // cache to disk
    pub fn from_bytes(b: &[u8]) -> Result<Program, ProgramDecodeError>;
}
```
- `Program: Send + Sync` so one compiled template renders concurrently across threads with no shared mutable state. (AC-8.1: render the same `Program` from 16 threads with distinct data; outputs are independent and correct; no data races under TSan/loom-style test.)
- All errors carry source spans (line/col/byte). Lints are non-fatal and collected. (AC-8.2: every error variant includes a span; snapshot test of error formatting.)

### 8.2 N-API (Node) binding
```ts
import { compile } from "turbo-html2pdf";
const program = compile(templateHtml, {
  partials, tokens, missingPolicy: "strict"
});                                  // returns a Program handle (native)
const { pdf, diagnostics, pageCount } = program.render({
  data, fonts, images, nodeStyles, meta
});                                  // pdf: Buffer (zero-copy where possible)
```
- `render` returns the PDF as a Node `Buffer` backed by Rust-owned memory (no copy) where the N-API version permits; otherwise one copy. (AC-8.3: a 5MB PDF render does not copy the buffer more than once across the boundary; measured.)
- `Program` is a JS object wrapping a native handle; serialize via `program.toBytes()` / `Program.fromBytes()`. (AC-8.4.)
- Errors map to a typed `TurboPdfError` subclass hierarchy with `.span`, `.code`. Diagnostics returned, not thrown. (AC-8.5.)

### 8.3 WASM binding (secondary)
- Same surface, async (`init()` to load the module), images/fonts passed as `Uint8Array`. No threads (or `wasm-bindgen-rayon` behind a flag). Document the perf delta vs native. (AC-8.6: WASM build renders the smoke-test fixtures identically to native, modulo font subsetter nondeterminism which must be pinned.)

### 8.4 Authoring frontends (separate, optional, app-agnostic)

The core consumes template HTML + `t:` DSL and **nothing else**. How that HTML is produced is outside the core. Frontends are independent packages shipped on their own cadence; none is a dependency of the core, and the core has no code path that knows which frontend (if any) was used. (AC-8.8: the core test suite contains no reference to any frontend package; a hand-written `.html` template is a first-class, fully-supported input with no frontend involved.)

**The frontend contract** (what any frontend must emit): well-formed, parseable HTML containing the required structural fields, with DSL expressed as `t:` elements and `{{ }}`/attribute expressions as **strings** (frontends do not evaluate the expression language — that happens in Rust at render). Any tool meeting this contract is supported: hand-authored HTML, template-string builders, Vue, Svelte, Astro, JSX, etc.

**React frontend** (`@turbo-html2pdf/react`) — the *first* frontend we ship, not a privileged one:
- Components rendering to the DSL via `renderToStaticMarkup`: `<If cond>`, `<ElseIf>`, `<Else>`, `<Switch on>`, `<Case value>`, `<Each of as index>`, `<Include src with>`, `<RunningHeader>`, `<RunningFooter>`, `<PageMaster>`, `<Region slot>`, `<Footnote>`, `<Page/>`, `<Pages/>`, `<Running name>`, `<Counter/>`.
- Each emits the corresponding `t:` element with attributes carrying **expression strings** (not evaluated JS) — e.g. `<Each of="invoice.lines" as="line">`. The React layer is a typed authoring convenience; expressions are still the DSL, resolved in Rust. (AC-8.7: a React template and its hand-written `t:` equivalent compile to byte-identical Programs.)
- Boundary: React runs once to produce the template; it is **not** in the render hot path and has no access to render data.

**Planned additional frontends** (post-v1, same contract, no core change required to add them): Vue SFC, Svelte, and a framework-free typed template-string builder. Listed here only to make the agnostic boundary concrete — building them is out of v1 scope. (AC-8.9: adding a new frontend requires zero changes to the core crate; demonstrated by a minimal second frontend — even a 50-line template-string helper — exercising the full DSL through the unchanged core.)

---

## 9. Errors, diagnostics, and limits

- **Compile errors** (fatal): Jinja syntax errors (unbalanced `{% %}`, bad expression, unknown filter/test) surfaced from MiniJinja with template span; malformed markup after the Jinja pass; unknown `t:` element; unknown page size; recursion-not-resolvable-statically. All with spans. (AC-9.1: error fixture corpus, each asserts code + span.)
- **Render errors** (fatal): missing var under `strict`, type errors, index out of range under `strict`, include depth exceeded, unresolved xref. (AC-9.2.)
- **Lints** (non-fatal, collected): unsupported CSS property, `track` attribute present, raw-mustache usage, region overflow clipped, footnote non-convergence, `.notdef` glyph, deep nesting warnings. (AC-9.3.)
- **Resource limits** (configurable, DoS guards): max nodes after expansion, max pages, max include depth, max iterations per loop (guards data-driven blowups), max footnote fixpoint iterations. Exceeding a hard limit is a render error naming the limit. (AC-9.4: a template that would expand to >limit nodes errors before OOM.)

---

## 10. Performance requirements & methodology

### 10.1 Absolute targets
- **Targets** (single core, native, warm Program, mid-range CPU; these are acceptance gates, tune during impl):
  - Simple 1-page invoice (≤50 nodes): compile < 2ms, render < 1ms. (AC-10.1.)
  - 1000-row table report (~10k nodes, paginated to ~30 pages): render < 50ms. (AC-10.2.)
  - 10,000-row table (~100k nodes, ~300 pages): render < 500ms, memory < 200MB. (AC-10.3.)
- **Amortization:** `compile` cost paid once; rendering N documents from one Program scales sub-linearly in per-doc fixed cost. (AC-10.4: 1000 renders of one Program total < 1000× a single cold compile+render.)
- **Concurrency:** linear-ish scaling rendering independent documents across threads up to core count. (AC-10.5: 8-thread throughput ≥ 5× single-thread on an 8-core box.)
- **Methodology:** `criterion` benches checked into `/benches`; a fixture generator produces the row counts; regressions > 10% fail CI. (AC-10.6: bench harness committed; CI gate active.)
- Zero-intermediate-HTML invariant from §1 enforced by an allocation-audit test. (AC-10.7 = AC-1.1.)

### 10.2 Competitive benchmarks (vs existing libraries)
A committed, reproducible benchmark suite comparing turbo-html2pdf against the incumbents, because "super fast" is only meaningful relative to what people use today. This is a deliverable, not a one-off.

**Contenders** (grouped by approach):
- *Headless-browser (HTML/CSS → PDF):* Puppeteer (`page.pdf()`), Playwright, Gotenberg (Chromium via HTTP). These are the fidelity baseline and the main thing we claim to beat on speed/footprint.
- *Programmatic React:* `@react-pdf/renderer` (its Yoga layout engine) — the closest "stay in JS" competitor.
- *Programmatic draw-API:* PDFKit (Node), jsPDF + jspdf-autotable (browser/Node).
- *Typesetting subprocess:* Typst (`typst compile`), and optionally `wkhtmltopdf` as a legacy reference.
(WeasyPrint, the Python HTML→PDF engine, is the closest architectural sibling and a useful correctness/speed reference even though it's not a JS-ecosystem competitor.)

**Standardized workload corpus** — the same logical documents implemented for each tool (or as close as the tool allows; if a tool can't express a workload, that's recorded as a capability gap, not skipped):
1. `invoice` — 1 page, light table, header/footer with page number.
2. `report-100` / `report-1k` / `report-10k` — N-row table paginated, repeating header row.
3. `legal` — long flowing text with automatic footnotes and a running section header (exercises the constructs browsers can't do natively).
4. `mixed` — headings, images, flex layout, page breaks, TOC-ish leaders.

**Metrics captured per tool × workload:**
- Cold start / process-or-engine init time (Chromium launch is the killer here; measured explicitly).
- Per-document render latency, warm (p50 and p95 over ≥100 runs).
- Throughput: documents/sec at concurrency = core count.
- Peak RSS (memory) per worker, and steady-state under sustained load.
- Output file size (with and without font subsetting where applicable).
- Distribution footprint: installed binary/dependency size and whether it ships a browser.

**Harness rules** (so numbers are honest and not cherry-picked):
- One repo, `/benches/competitive`, with a runner that builds each tool's environment (Node deps, Chromium download, Typst binary, Gotenberg container) reproducibly via pinned versions, and emits a single results table + JSON. (AC-10.8: `cargo xtask bench-competitive` (or `make bench-competitive`) runs the full matrix and produces the table from scratch on a clean machine.)
- Warm vs cold separated and labeled; for browser tools, report both "with per-doc browser launch" and "with a reused browser/page pool," since real deployments differ. (AC-10.9: both cold and pooled numbers reported for Puppeteer/Playwright.)
- Same fonts, same page geometry, same logical content across tools; visual output diffed to confirm the renders are actually comparable (not winning by doing less). (AC-10.10: each competitive workload has a side-by-side rendered-PNG comparison committed; gross layout divergence fails the comparison as "not equivalent.")
- Hardware + OS + tool versions recorded in the results artifact; numbers are presented as "on this machine," never as absolutes. (AC-10.11.)
- Results regenerated in CI on a fixed runner and published as an artifact; a markdown summary table lives in the repo and is regenerated, not hand-edited. (AC-10.12.)

**Claim gating:** any public performance claim ("Nx faster than Puppeteer") must cite a specific workload + machine from this harness and link the reproducing command. No freestanding multipliers. (AC-10.13: README perf claims map 1:1 to a harness workload id.)

**Expected shape of results** (hypotheses to verify, not promises): turbo-html2pdf should win decisively on cold start and memory/footprint (no Chromium), win on throughput for high-volume simple docs, and win on the `legal` workload's *capability* (native footnotes/running headers) regardless of raw speed. Headless browsers may remain ahead on exotic CSS fidelity — that's the honest tradeoff and gets documented, not hidden.

---

## 11. Accessibility & correctness extras (feature-gated)

- `feature = "pdf-ua"`: tagged PDF structure tree from semantic HTML (`h1..h6`, `p`, `ul/ol/li`, `table` with `th` scope, `figure/figcaption`), alt text from `alt`/`aria-label`, reading order = flow order. (AC-11.1: PAC/veraPDF UA checks pass on the tagged fixtures.)
- `feature = "pdf-a"`: PDF/A-2b (font embedding mandatory, no transparency violations, XMP). (AC-11.2.)
- Language tagging from `lang` attributes into structure + text. (AC-11.3.)

---

## 12. Roadmap / explicit v2+ deferrals
- RTL/bidi and complex-script shaping (Arabic, Indic) beyond rustybuzz defaults; `direction`/`unicode-bidi`.
- CSS grid, floats, multi-column flow regions, `position: absolute/fixed`.
- Vector SVG emission (not rasterized).
- Scoped/component-local stylesheets.
- Incremental re-render using the reserved `track` key (re-paginate only changed regions).
- System font discovery in the binding layer.

---

## 13. Build order for the implementing agent

Each step ends at a green test suite for its ACs before the next begins.

1. **Templating layer: embed MiniJinja** (§2). Wire data → MiniJinja `Value`, register document filters/functions (§2.12), set strict undefined + trim defaults, parse+cache templates in `compile`. ACs 2.0, 2.0b, 2.1–2.4, 2.18–2.24. Fuzz template inputs.
2. **`t:` element recognition + `switch` extension after Jinja render** (§2.1, §2.6, §3). Jinja renders to intermediate markup; html5ever parses; `t:` elements become typed nodes. Implement the `{% switch %}` extension (custom statement or pre-parse desugaring). Compile errors with spans. ACs 2.5–2.17, 2.9b–2.9j, 2.23, 9.1.
3. **Execute Jinja+parse → node tree, in-process** (§1 Stage 1). No HTML string escapes to caller; intermediate markup is internal/short-lived. Includes/macros + recursion bound. ACs 2.14–2.17, 9.2, 1.1.
4. **CSS parse + cascade + tokens + node-style injection** (§4). `cssparser`/`selectors`. ACs 4.1–4.9.
5. **Fonts + inline/text layout** (§4.4, §5.2). rustybuzz/unicode-linebreak/hyphenation. ACs 4.10–4.11, 5.2–5.4.
6. **Block/flex/table layout** (§5.1, 5.3–5.5). taffy for flex. ACs 5.1, 5.5–5.11.
7. **Auto-pagination (the default model)** (§3.0, §6.1–6.2, 6.5–6.6). Default geometry, `@page`, `<t:running-header/footer>`, full Word-style page-context variables (§3.3), the break walk with widows/orphans, page-count-as-output. Must work with **zero** master DSL. ACs 3.0.1–3.0.5, 3.6–3.9, 6.0–6.2, 6.6–6.8.
8. **Masters + regions + counters/running (overrides on auto-pagination)** (§3.1–3.5, 3.8, §6.3). Named/first/left/right geometry, named margin boxes, mid-doc switches, repeated table headers; layered on step 7 without disabling it. ACs 3.1–3.14, 3.22–3.24, 6.3, 3.10–3.11.
9. **Footnotes + fixpoint** (§3.6–3.7, §6.4). Automatic content-driven placement. ACs 3.15–3.21, 6.4–6.5.
10. **PDF emitter** (§7). pdf-writer + subsetter. ACs 7.1–7.7.
11. **N-API binding** (§8.2). ACs 8.1–8.5, concurrency 10.5.
12. **Authoring frontends** (§8.4): React package first; then prove agnosticism with a second minimal frontend (template-string builder) through the unchanged core. ACs 8.7–8.9.
13. **WASM binding** (§8.3). AC 8.6.
14. **Perf: absolute targets + benches + CI gates** (§10.1). ACs 10.1–10.7.
15. **Competitive benchmark harness** (§10.2). Build the reproducible matrix vs Puppeteer/Playwright/Gotenberg/@react-pdf/PDFKit/jsPDF/Typst; capture cold-start, latency, throughput, memory, file size, footprint; commit results + PNG equivalence diffs; CI artifact. ACs 10.8–10.13.
16. **Feature gates: xref, pdf-ua, pdf-a, svg, print-color, endnotes** (§3.9, §11, §7). ACs 3.25, 11.1–11.3, 7.3/7.4/7.7.

### 13.1 Recommended crates
`minijinja` (templating: expressions, control flow, filters, macros, includes), `html5ever` (parse the rendered markup), `cssparser` + `selectors` (CSS), `taffy` (flex), `rustybuzz` (shape), `ttf-parser`/`swash` (metrics/faces), `unicode-linebreak` + `unicode-bidi` (breaking), `hyphenation` (Knuth-Liang), `pdf-writer` (emit), `subsetter` (font subset), `resvg` (svg raster, feature-gated), `serde` (data input), `napi`/`napi-derive` (Node), `wasm-bindgen` (WASM), `criterion` (bench), `insta` (snapshot tests).

---

## 14. Test corpus (deliverable alongside code)
- **Golden PDFs**: render fixtures to PDF, rasterize via a pinned renderer, compare against committed reference PNGs with a perceptual diff threshold. (AC-14.1: visual-regression suite green; diffs > threshold fail CI.)
- **DSL unit fixtures**: one per AC in §2–§3, table-driven.
- **Cascade fixtures**: §4 selector/specificity/token-order tables.
- **Pagination fixtures**: orphans/widows, forced/avoid breaks, repeated headers, footnote convergence/continuation, running-element policies.
- **Determinism**: identical-input byte-equality (AC-7.6) across native and (pinned) WASM.
- **Fuzz**: expression parser and template parser under `cargo fuzz`; no panics, only typed errors. (AC-14.2.)
- **Conformance** (gated): veraPDF for pdf-a/pdf-ua.

---

## Appendix A — worked templates

### A.0 Minimal (auto-pagination, zero page config)

The smallest useful template. No master, no `@page`, no declared pages — auto-pagination + default geometry do all the work, and one `<t:running-footer>` gets page numbers on every page.

```html
<t:running-footer>Page <t:page/> of <t:pages/></t:running-footer>

<h1>{{ invoice.title }}</h1>
<table class="lines">
  <thead><tr><th>Description</th><th class="right">Amount</th></tr></thead>
  <tbody>
    {% for line in invoice.lines %}
      <tr><td>{{ line.description }}</td><td class="right">{{ line.amount | currency(invoice.ccy) }}</td></tr>
    {% endfor %}
  </tbody>
</table>
```
A 3-line-item invoice renders one page; a 500-line-item invoice renders however many pages it takes, header row repeating, footer numbering correct — with no change to the template. That is the default behavior, not a feature you opt into.

### A.1 Full (masters/regions/footnotes — overrides, covers most DSL surface)

This uses the override machinery (§3.1+) for a distinct first page, running section header, and footnotes. Everything here is optional sugar on top of the A.0 default.
```html
<t:page-master name="default" size="A4" margin="22mm 18mm 24mm 18mm">
  <t:region slot="header" extent="14mm">
    <div class="hdr">
      <span>{{ doc.title }}</span>
      <t:leader/>
      <span class="muted">{{ running.section }}</span>
    </div>
  </t:region>
  <t:region slot="footer" extent="12mm">
    <div class="ftr">
      <span>{{ doc.confidential ? 'CONFIDENTIAL' : '' }}</span>
      <t:leader/>
      <span>Page <t:page/> of <t:pages/></span>
    </div>
  </t:region>
  <t:variant kind="first">
    <t:region slot="header" extent="0mm"/>   <!-- no header on first page -->
  </t:variant>
</t:page-master>

<style data-scope="global">
  body { font-family: "Inter", sans-serif; font-size: 10pt; color:#111; }
  .hdr,.ftr { display:flex; align-items:center; }
  .muted { color:#888; }
  table.lines { width:100%; border-collapse:collapse; }
  table.lines th, table.lines td { padding:4pt 6pt; border-bottom:0.5pt solid #ccc; }
  table.lines tbody tr:nth-child(even){ background:#fafafa; }
  .right { text-align:right; }
</style>

<section t:master="default">
  <h1 t:running="section">{{ invoice.title }}</h1>

  {% if invoice.status == "paid" %}
    <p class="muted">Paid in full on {{ invoice.paid_on | date("YYYY-MM-DD") }}.</p>
  {% elif invoice.status == "overdue" %}
    <p style="color:#b00">Overdue by {{ invoice.days_overdue }} days.</p>
  {% else %}
    <p>Due {{ invoice.due_on | date("YYYY-MM-DD") }}.</p>
  {% endif %}

  <table class="lines">
    <thead>
      <tr><th>#</th><th>Description</th><th class="right">Qty</th><th class="right">Amount</th></tr>
    </thead>
    <tbody>
      {% for line in invoice.lines %}
        <tr>
          <td>{{ loop.index }}</td>
          <td>
            {{ line.description }}
            {% if line.note %}<t:footnote>{{ line.note }}</t:footnote>{% endif %}
          </td>
          <td class="right">{{ line.qty | number }}</td>
          <td class="right tabular" t:style="tabular">{{ line.amount | currency(invoice.ccy) }}</td>
        </tr>
      {% else %}
        <tr><td colspan="4" class="muted">No line items.</td></tr>
      {% endfor %}
    </tbody>
    <tfoot>
      <tr><td colspan="3" class="right">Total</td>
          <td class="right" t:style="total tabular">{{ invoice.total | currency(invoice.ccy) }}</td></tr>
    </tfoot>
  </table>

  {% switch customer.tier %}
    {% case "enterprise" %}
      <p>Dedicated support included.</p>
    {% case "pro", "plus" %}
      <p>Priority support included.</p>
    {% default %}
      <p>Standard support.</p>
  {% endswitch %}

  {% include "remittance" %}   {# remittance.html reads company.bank, invoice.ref from caller scope; or use a macro for explicit args #}
</section>
```
