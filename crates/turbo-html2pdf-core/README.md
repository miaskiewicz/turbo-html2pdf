# turbo-html2pdf-core

The native **HTML/CSS + Jinja → PDF** engine behind
[turbo-html2pdf](https://github.com/miaskiewicz/turbo-html2pdf) — a from-scratch
document engine: templating → HTML/CSS layout → automatic pagination → PDF 1.7.
No headless browser, no Chromium, deterministic byte-for-byte output.

This is the pure-Rust core crate, for in-process Rust consumers. The Node
(N-API), Python (PyO3), browser (WASM), and MCP-server surfaces are thin shims
over it — see the [repo](https://github.com/miaskiewicz/turbo-html2pdf) for those.

```rust
use turbo_pdf_core::{
    build_cascade, compile, emit_pdf, render_pages, CompileOptions, Diagnostics,
    EmitOptions, RenderInputs, NoImages,
};
use turbo_pdf_core::style::{parse_stylesheet, TokenSet};

let (program, _) = compile("<h1>{{ title }}</h1>", &CompileOptions::default()).unwrap();
let data = serde_json::json!({ "title": "Hello" });
let css = "@page { size: A4; margin: 1in; }";
let cascade = build_cascade(css, "", TokenSet::default());
let at_rules = parse_stylesheet(css).at_rules;

let mut diags = Diagnostics::default();
let pages = render_pages(
    &RenderInputs {
        program: &program,
        data: &data,
        cascade: &cascade,
        at_rules: &at_rules,
        fonts: &turbo_pdf_core::FontRegistry::new(),
        images: &NoImages,
        now: None,
    },
    &mut diags,
).unwrap();
let pdf: Vec<u8> = emit_pdf(&pages, &EmitOptions::default());
```

## Layout engine: turbo-dom-layout

The HTML/CSS layout half — style cascade, block/inline/flex/grid/table layout,
text and fonts (incl. the bundled faces), image sizing, `layout_snapshot` and the
Jinja-free `layout_html` drive — lives in the
[`turbo-dom-layout`](https://github.com/miaskiewicz/turbo-dom/tree/main/crates/turbo-dom-layout)
crate. This crate depends on it and re-exports it at the same paths
(`turbo_html2pdf_core::style::…`, `::layout::…`, `::text::…`, `::layout_snapshot::…`,
`::Tag`, `::Fragment`, …). Templating, `t:` directives, pagination/footnotes/endnotes
and PDF emit stay here. A rendered template is a turbo-dom document (`Flow`:
`Program::render_nodes` returns it, `parse_html` parses raw HTML into one; `Flow::style`
styles it, `Flow::view` / `Flow::roots` read it); the `t:` directives are turbo-dom-layout markers (`markers`), and
PDF/UA roles + links (internal and URI) and destinations are derived after layout (`Semantics`, on each `Page`).
The root `layout_html` / `layout_boxes` / `parse_html` are the html2pdf-mode HTML
drive (directives classified, unknown `t:` rejected); `html_layout::*` is
turbo-dom-layout's directive-free one. The
layout tests and their Chrome control fixtures (referenced below as
`tests/fixtures/…`) live in turbo-dom-layout.

## Features

Default = `bundled-fonts` (embeds Inter/Roboto, Liberation Serif/PT Serif,
Fira Code/IBM Plex Mono so documents render with no caller fonts). Opt-in:
`endnotes`, `xref`, `print-color`, `pdf-a`, `pdf-ua`, `append`, `encrypt`,
`svg`. `bundled-fonts`, `svg` and `perf` forward to turbo-dom-layout; `xref` and
`print-color` set turbo-dom-layout's runtime options, and `pdf-ua`/`xref` derive the
fragment semantics after layout. See the repo for the full specification.

## License

MIT

## Live-tree layout provenance

The additive `layout_snapshot` API accepts the caller's live `Node` or styled
forest plus external stable source IDs and retains all native fragments with
composed transforms. It preserves hidden layout boxes and sizes replaced images
without PDF page caps. PDF filtering, page-fit and legacy layout policies are preserved; shared CSS
cascade fixes also improve valid PDF output. See
[the snapshot contract and current geometry boundaries](../../docs/live-layout-snapshot.md);
native text-source range/caret queries do not implement DOM Range assembly,
browser hit testing or scroll metrics for the embedding application.

The native text-enabled HTML path retains ordinary inline boundaries and real
per-line border fragments, including sliced decorations, empty inlines, nested
font metrics, relative positioning and super/sub atomic children. Its explicit
`UsedBoxMetrics.inline_fragment` flag distinguishes these from inline replaced
elements. Ordinary static block-in-inline content uses real anonymous flow splits,
including nested and decorated continuations. Positioned/floating split contexts,
ordinary-inline top/bottom/middle
alignment, out-of-flow positioning and non-horizontal/LTR writing return typed
snapshot errors. The generic and PDF paths retain their existing layout policy.

Native HTML source items admit their aggregate advance to the physical
LayoutUnit grid before line breaking and placement, while glyph positions and
partial selections retain their real signed 16.16 advances. Aggregation crosses
words and collapsed spaces within one text source; adjacent text nodes remain
separate items. The pinned-font controls in turbo-dom-layout's `tests/fixtures/native-font-runs`
record normal headed Chrome155 with real IBM Plex Mono and Fira Code bytes.
Headless automation with forced font-rendering flags is a separate profile and
is not substituted for that control. Generic and PDF paths retain raw advances.

Leading spaces after inline atoms remain inside their real text/inline owner.
The first rendered space retains its raw advance; extra collapsed source cells
follow that advance, and wrapping relocates a collapsed opening prefix to the
text's new line. The nineteen observations in turbo-dom-layout's
`tests/fixtures/native-leading-spaces`
cover direct text, nested/decorated spans, paragraph starts, forced breaks and
ordinary bare-text separators. At soft wraps, a separator-only Range retains
both real line affinities; positive selections keep their nonempty fragments.
These source-mapping rules apply only to native HTML, preserving PDF whitespace.

`GlyphSourceCluster` and `SourceClusterBox` expose `collapsed_whitespace`
provenance. The rendered first collapsed space can still have positive advance;
native caret canonicalization follows only a zero-space source suffix ending at
an actual same-line glyph edge. Preformatted glyph spaces keep ordinary provenance.
In v0.3.8 these structs gain a field: callers constructing them with
Rust struct literals must initialize the new flag (false for ordinary glyphs).

Native text Range and block/replaced Element quads preserve each flat transform
boundary and subsequent float ancestor offsets. Ordinary inline Element quads
retain their accumulated global matrix, as in Chrome. The sibling
turbo-dom-layout's `tests/fixtures/native-range-transforms` controls retain the distinct margin,
padding, viewport-positioned and nested-transform observations. A fixed child
under a transformed containing block remains an explicit unsupported snapshot;
its Chrome observation stays in the fixture rather than being claimed as parity.

Native HTML iframe replacements use the CSS 300×150 default object size when
attributes and CSS do not provide dimensions. Valid zero-width/height viewports
retain fixed-size descendants, viewport units and definite-zero percentage bases,
including zero-width bodies under real HTML document roots;
normal-flow relative positioning does not substitute the absolute containing block.
Iframe fallback descendants retain source identities and text while being excluded
from the parent layout. Embedders use that explicit exclusion to return empty DOM Range geometry.
The lower-level glyph query retains its `UnresolvedSource` error when no glyphs
are placed; invalid UTF-16 offsets and genuinely lost visible glyphs remain errors.
Native viewport admission accepts finite dimensions at least zero. Generic/PDF
snapshot admission continues to require positive dimensions.

The snapshot input does not yet carry HTML document compatibility mode. Its line
layout follows the standards policy: an independently measured zero-viewport
standards document with fixed and inline-iframe descendants has a 34px body;
Chrome's BackCompat sibling suppresses the inline-only line strut and has a 30px
body. Both raw Chrome controls are retained in turbo-dom-layout's `tests/fixtures/native-iframe`.
Quirks-mode line-strut/aggregate-height parity requires a real document-mode input
and remains outside this API's current contract; fixed descendant boxes match in
both measurements. No rectangle is clipped or normalized to conceal this gap.
