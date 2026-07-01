"""Conformance / encryption / append exposure tests for the PyO3 binding.

Proves the merged-engine capabilities the binding now surfaces as render kwargs
(and the standalone ``append_pdf``):

  * ``pdf_a=True``    -> ``/OutputIntents`` + the ``GTS_PDFA`` subtype.
  * ``pdf_ua=True``   -> ``/StructTreeRoot`` + ``/MarkInfo`` + ``/Lang``.
  * ``cmyk=True``     -> a DeviceCMYK fill operator (``k``).
  * ``encryption=``   -> an ``/Encrypt`` dictionary (and a wrong password is
                         rejected, while the right one opens, when qpdf is
                         present).
  * ``append_pdfs=`` / ``append_pdf`` -> the page count grows.

These run against the BUILT extension (``maturin develop`` or an installed
wheel). The module-not-importable case is skipped at collection time.
"""

import re
import shutil
import subprocess
import tempfile
from pathlib import Path

import pytest

t = pytest.importorskip("turbo_html2pdf")

REPO_ROOT = Path(__file__).resolve().parents[3]
FONTS_DIR = REPO_ROOT / "crates" / "turbo-html2pdf-core" / "assets" / "fonts"
CSS = "@page { size: 200px 200px; margin: 10px } p { font-size: 12px }"
TEMPLATE = "<p>Conformance body text</p>"


def _fonts():
    return t.Fonts.load(sorted(p.read_bytes() for p in FONTS_DIR.glob("*.ttf")))


def _render(**kwargs):
    return t.render(
        TEMPLATE, css="p { font-family: font0; font-size: 12px } " + CSS,
        fonts=_fonts(), now=0, **kwargs,
    )


def _qpdf():
    return shutil.which("qpdf")


def test_pdf_a_emits_output_intent():
    pdf = _render(pdf_a=True)
    assert pdf.startswith(b"%PDF")
    assert b"/OutputIntents" in pdf
    assert b"GTS_PDFA" in pdf


def test_pdf_a_off_by_default():
    assert b"/OutputIntents" not in _render()


def test_pdf_ua_emits_struct_tree_and_markinfo():
    pdf = _render(pdf_ua=True, lang="en-US")
    assert b"/StructTreeRoot" in pdf
    assert b"/MarkInfo" in pdf
    assert b"/Lang" in pdf


def test_cmyk_emits_device_cmyk_operator():
    pdf = _render(cmyk=True)
    text = pdf.decode("latin1")
    assert re.search(r"\b\d?\.?\d+ \d?\.?\d+ \d?\.?\d+ \d?\.?\d+ k\b", text), "DeviceCMYK k op"


def test_encryption_writes_encrypt_dict():
    pdf = _render(encryption={"user_password": "open-sesame"})
    assert pdf.startswith(b"%PDF")
    assert b"/Encrypt" in pdf


def test_encryption_off_by_default():
    assert b"/Encrypt" not in _render()


@pytest.mark.skipif(_qpdf() is None, reason="qpdf not installed")
def test_encryption_round_trips_through_qpdf():
    pdf = _render(encryption={"user_password": "open-sesame"})
    with tempfile.TemporaryDirectory() as d:
        path = Path(d) / "enc.pdf"
        path.write_bytes(pdf)
        # Right password: clean check.
        subprocess.run(
            ["qpdf", "--password=open-sesame", "--check", str(path)],
            check=True, capture_output=True,
        )
        # Wrong password: qpdf fails.
        wrong = subprocess.run(
            ["qpdf", "--password=nope", "--check", str(path)], capture_output=True,
        )
        assert wrong.returncode != 0


def test_encryption_permission_flags_accepted():
    # A permission override must not error and still produce an /Encrypt dict.
    pdf = _render(encryption={"user_password": "pw", "copy": False, "print": True})
    assert b"/Encrypt" in pdf


def test_encryption_requires_user_password():
    with pytest.raises(t.TurboPdfError):
        _render(encryption={"owner_password": "only-owner"})


def _page_nodes(pdf: bytes) -> int:
    return len(re.findall(rb"/Type\s*/Page\b", pdf))


def test_append_pdfs_grows_page_count():
    extra = _render()
    merged = _render(append_pdfs=[extra])
    assert _page_nodes(merged) >= 2
    if _qpdf():
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "merged.pdf"
            path.write_bytes(merged)
            out = subprocess.run(
                ["qpdf", "--show-npages", str(path)], check=True, capture_output=True,
            )
            assert out.stdout.strip() == b"2"


def test_standalone_append_pdf_merges():
    a = _render()
    b = _render()
    merged = t.append_pdf(a, [b])
    assert merged.startswith(b"%PDF")
    assert _page_nodes(merged) >= 2


def test_append_pdf_raises_on_malformed_input():
    a = _render()
    with pytest.raises(t.TurboPdfError):
        t.append_pdf(a, [b"not a pdf"])
