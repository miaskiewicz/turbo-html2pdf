# turbo-html2pdf (Python)

Native HTML/CSS-to-PDF engine with a Jinja-compatible templating DSL. PyO3
binding over `turbo-html2pdf-core`. The local 0.8.4 candidate uses the matching prerelease turbo-dom
0.8.4 DOM/layout crates. It shares the Node package's compile/render pipeline,
with bundled fonts, PDF/A, PDF/UA, CMYK, endnotes, internal links, append and
encryption; Node's post-render `stamp` API is not exposed by this binding.

```python
import turbo_html2pdf as t

# Compile once, render many times.
prog = t.compile("<p>Hello {{ data.name }}</p>")
pdf = prog.render(data={"name": "world"})       # -> bytes, starts with b"%PDF"

# Warm fonts once, reuse the handle across renders.
fonts = t.Fonts.load([open(p, "rb").read() for p in font_paths])
pdf = prog.render(
    data={"name": "world"},
    css="body { font-family: font0 }",
    fonts=fonts,
)

# One-shot: compile + render in a single call.
pdf = t.render("<p>{{ data.x }}</p>", data={"x": 1})

# Inspect non-fatal lints + page count.
pdf, diagnostics, page_count = prog.render_full(data={"name": "world"})
```

Fatal compile/render faults raise `TurboPdfError` carrying `.code` (a stable
string such as `"TemplateSyntax"`) and `.span` (`{"line", "col", "byte_offset"}`).
Non-fatal lints are returned by `render_full`, never raised.

## API

| Surface | Signature |
| --- | --- |
| `compile(template_html, opts=None)` | `-> Program` |
| `Program.render(data=None, css="", fonts=None, images=None, meta=None, now=None)` | `-> bytes` |
| `Program.render_full(...)` | `-> (bytes, list[dict], int)` |
| `Program.has_header()` / `Program.has_footer()` | `-> bool` |
| `render(template_html, ...)` | one-shot `-> bytes` |
| `Fonts.load([bytes, ...])` | warm registry `-> Fonts` |

## Building from source

```sh
pip install maturin
maturin develop --manifest-path crates/turbo-pdf-py/Cargo.toml
python -m pytest crates/turbo-pdf-py/tests
```
