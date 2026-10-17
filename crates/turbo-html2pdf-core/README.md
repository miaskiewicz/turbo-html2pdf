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

## Features

Default = `bundled-fonts` (embeds Inter/Roboto, Liberation Serif/PT Serif,
Fira Code/IBM Plex Mono so documents render with no caller fonts). Opt-in:
`endnotes`, `xref`, `print-color`, `pdf-a`, `pdf-ua`, `append`, `encrypt`,
`svg`. See the repo for the full specification.

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
elements. Unsupported block-in-inline splitting, ordinary-inline top/bottom/middle
alignment, out-of-flow positioning and non-horizontal/LTR writing return typed
snapshot errors. The generic and PDF paths retain their existing layout policy.
