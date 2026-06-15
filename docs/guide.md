# turbo-html2pdf user guide

turbo-html2pdf turns a **template** + **data** (+ optional CSS and fonts) into a PDF.
The template is HTML markup with embedded [MiniJinja](https://github.com/mitsuhiko/minijinja)
logic and `t:` paged-media directives. Pagination is automatic — **page count is
an output, never an input.**

This guide is split into focused pages. Everything in them is verified against
the source and the test suite; deferred / partial features are flagged
explicitly.

## Contents

- **[Templating DSL](dsl.md)** — the Jinja base, the `{% switch %}` extension,
  the document-domain filters (`currency`, `number`, `percent`, `ordinal`, `pad`,
  `truncate`, `wordwrap`, `date`, `datetime`), and `now()`.
- **[Paged media](paged-media.md)** — the `t:` directives (with
  implemented/deferred status), running headers/footers and the per-page context
  (`page.number` / `page.total` / `page.is_first` / `page.is_last`), footnotes
  (marks, auto-numbering, reset modes, separator, oversized continuation), and
  pagination (`@page` geometry, break rules, orphans/widows, repeated table
  headers).
- **[CSS support](css-support.md)** — the supported selector / property / unit /
  color subset and the UA default stylesheet.
- **[JS / React / WASM API](api.md)** — `@turbo-html2pdf/napi`, `@turbo-html2pdf/wasm`,
  `@turbo-html2pdf/react`, `@turbo-html2pdf/template`, and the **compile-once / render-many**
  `Program` warm-start pattern.

## A minimal example

Template:

```html
<h1>{{ title }}</h1>
<t:running-footer>Page <t:page/> of <t:pages/></t:running-footer>
{% for line in lines %}
  <p>{{ line.label }}: {{ line.amount | currency("USD") }}</p>
{% endfor %}
```

Render it (Node, `@turbo-html2pdf/napi`):

```js
const { compile } = require('@turbo-html2pdf/napi')
const fs = require('node:fs')

const program = compile(templateSource)   // compile once
const { pdf } = program.render({           // render many
  data: { title: 'Invoice', lines: [{ label: 'Widget', amount: 1234.5 }] },
  css: '@page { size: A4; margin: 20mm }',
  fonts: [fs.readFileSync('font.ttf')],
  now: 0,
})
fs.writeFileSync('invoice.pdf', pdf)
```

## What is not implemented yet

So you don't build on sand, the notable deferred items (verified against
`TODO(phase…)` markers in the code) are:

- **Images** — `<img>` is not laid out and the `images` binding input is a no-op
  (`TODO(phase9b)`). There is no image overflow cap in the code.
- **Page masters** — `<t:page-master>`, `<t:variant>`, `<t:use-master>`,
  `<t:region>`, the general `<t:counter>`, and `<t:leader>` are parsed but not
  implemented (`TODO(phase7b)`).
- **Endnotes / section anchors** — `<t:endnote>`, `<t:endnotes>`, `<t:anchor>`,
  and `section`-reset footnotes are parsed/accepted but not implemented
  (`TODO(phase15)`); `section` reset is treated as continuous.
- **Custom footnote separator** — `<t:footnote-separator>` is recognized but the
  band always paints the built-in default rule.
