"""End-to-end tests for the turbo_html2pdf PyO3 binding.

Compiles a real template + data, renders a PDF, asserts the magic header and a
positive page count, and exercises the warm `Fonts` handle and the typed
`TurboPdfError`. Fonts are loaded from the core crate's bundled assets.
"""

from pathlib import Path

import pytest

import turbo_html2pdf as t

# crates/turbo-pdf-py/tests/ -> repo root is three parents up.
REPO_ROOT = Path(__file__).resolve().parents[3]
FONTS_DIR = REPO_ROOT / "crates" / "turbo-html2pdf-core" / "assets" / "fonts"

# The data object is interpolated under the `data` root (mirrors `{{ data.* }}`
# in the engine's docs), so the payload is nested under a "data" key.
TEMPLATE = "<p>Hello {{ data.name }}, you have {{ data.count }} messages.</p>"
DATA = {"data": {"name": "world", "count": 3}}


def _font_blobs():
    blobs = [p.read_bytes() for p in sorted(FONTS_DIR.glob("*.ttf"))]
    assert blobs, f"no fonts found in {FONTS_DIR}"
    return blobs


def test_compile_and_render_returns_pdf_bytes():
    prog = t.compile(TEMPLATE)
    pdf = prog.render(data=DATA)
    assert isinstance(pdf, bytes)
    assert pdf.startswith(b"%PDF")


def test_render_full_reports_page_count():
    prog = t.compile(TEMPLATE)
    pdf, diagnostics, page_count = prog.render_full(data=DATA)
    assert pdf.startswith(b"%PDF")
    assert page_count > 0
    assert isinstance(diagnostics, list)


def test_one_shot_render():
    pdf = t.render(TEMPLATE, data=DATA)
    assert pdf.startswith(b"%PDF")


def test_warm_fonts_handle_reused_across_renders():
    fonts = t.Fonts.load(_font_blobs())
    prog = t.compile(TEMPLATE)
    css = "p { font-family: font0; font-size: 12pt }"
    first = prog.render(data=DATA, css=css, fonts=fonts)
    second = prog.render(
        data={"data": {"name": "again", "count": 7}}, css=css, fonts=fonts
    )
    assert first.startswith(b"%PDF")
    assert second.startswith(b"%PDF")


def test_has_header_and_footer_flags():
    prog = t.compile(TEMPLATE)
    assert prog.has_header() is False
    assert prog.has_footer() is False


def test_meta_is_accepted():
    prog = t.compile(TEMPLATE)
    pdf = prog.render(data=DATA, meta={"title": "Greeting", "author": "test"})
    assert pdf.startswith(b"%PDF")


def test_fatal_error_raises_typed_exception():
    with pytest.raises(t.TurboPdfError) as exc_info:
        t.compile("{{ data.name ")  # unterminated expression -> TemplateSyntax
    err = exc_info.value
    assert isinstance(err.code, str) and err.code
    assert set(err.span) >= {"line", "col", "byte_offset"}
