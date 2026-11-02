# Benchmark history

turbo-html2pdf 0.6.0 (the turbo-dom consolidation: layout, style and text now come from
the shared `turbo-dom-layout` / `turbo-dom` crates) has **not been re-benchmarked yet**.
A re-run of all four turbo-* repos together, on a quiet machine, is planned after the
release. Until then, the numbers below are the last measured ones. They describe the
engine **before** the consolidation and are kept for history only: do not read them as
0.6.0 numbers.

Provenance: the tables were added to the README on 2026-06-15 (before the `pyv0.1.1`
tag); [`benches/competitive/RESULTS.md`](../benches/competitive/RESULTS.md) is the
harness output for turbo-html2pdf 0.2.3 (its run context records the machine, Node
version and run counts).

## Pre-consolidation snapshot (turbo-html2pdf 0.1–0.2, Apple M3, 8 cores; `benches/competitive`)

Two scenarios, both measured:

- **Warm (server):** the realistic server path — the template's compiled `Program`
  is reused and **fonts are parsed once and cached** (the `Fonts` handle). This is
  the number to lead with.
- **Cold (one-shot):** a single render from a cold start (compile + parse fonts +
  render), e.g. a CLI invocation.

All engines render the **same** content (same fonts, A4 geometry, same text); a
PNG-equivalence diff (`sim` ≈ 0.98 vs the reference) confirms the outputs are
comparable, so this isn't "winning by doing less".

**`invoice` — a one-page document (the bread-and-butter case), median ms:**

| Engine | Warm (cached) | Cold | vs turbo (warm) | Memory | Ships a browser? |
|---|--:|--:|--:|--:|:--:|
| **turbo-html2pdf** | **1.27** | **1.48** | **1×** | **232 MB** | **no** |
| Playwright (Chromium) | 46.3 | 49.4 | **36× slower** | 393 MB | yes (~200 MB) |
| Gotenberg (Chromium) | 66.9 | 70.6 | **53× slower** | 424 MB | yes (~1 GB) |
| Puppeteer (Chromium) | 86.1 | 132.2 | **68× slower** | 550 MB | yes (~200 MB) |
| Typst (native, own DSL) | 63.8 | 62.7 | **50× slower** | 1.5 GB | no |
| WeasyPrint (Python) | 375.8 | 379.3 | **296× slower** | 379 MB | no |

**It's also the fastest on every other workload — not just the small one:**

| Workload | turbo-html2pdf (warm/cold) | Playwright | Puppeteer | Gotenberg | WeasyPrint | Typst |
|---|--:|--:|--:|--:|--:|--:|
| `invoice` (1 page) | **1.27 / 1.48** | 46 / 49 | 86 / 132 | 67 / 71 | 376 / 379 | 64 / 63 |
| `report-1k` (1 000-row table) | **119 / 130** | 187 / 170 | 384 / 429 | 204 / 204 | 2 550 / 2 547 | 220 / 221 |
| `legal` (prose + footnotes) | **25 / 26** | 71 / 98 | 124 / 147 | 80 / 80 | 444 / 453 | 72 / 72 |
| `mixed` (flex + table) | **8.6 / 9.2** | 57 / 58 | 94 / 229 | 73 / 73 | 432 / 437 | 69 / 71 |

Against every **HTML/CSS-to-PDF** engine, turbo-html2pdf wins on **all** workloads —
by 1.6× (big tables) up to ~300× (small docs) — at **lower memory** and with **no
browser** to install.

> **Honest footnote.** `pdfkit` (~6 ms) and `jspdf` (~2 ms) are faster on raw
> output, but they are **imperative draw APIs** — *you* compute and place every box;
> there is no HTML/CSS, no layout engine, no pagination. `@react-pdf` is a React
> component DSL (no HTML/CSS) and its flexbox layout melts on big tables (`report-1k`
> ≈ **1 300 ms**, ~10× slower than turbo). Different category from "give me HTML, get
> a PDF". `wkhtmltopdf` (legacy/unmaintained) was not installed.

Numbers are "on this machine", never absolutes — rerun with `cd benches/competitive
&& pnpm bench`. Full table + methodology in
[`benches/competitive/RESULTS.md`](../benches/competitive/RESULTS.md).
