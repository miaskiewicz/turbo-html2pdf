# turbo-html2pdf 0.8.4 candidate vs jsPDF: ABBA

Measured 2026-11-13, after integrating the ASCII direction fast path and paired
intrinsic text preparation. The cascade Cow experiment passed correctness but
showed no repeatable latency improvement and is excluded.

## Source and environment

- turbo-html2pdf native version: **0.8.4 candidate**, ordinary release profile.
- HTML implementation: `a7da7ed0a8a431351c1c8921af37f61a6fb8114f`, with the dependency update recorded by this release branch.
- turbo-dom and turbo-dom-layout: **`8266b30a366758a591f4d9952823576a468637a3`**.
- Rust 1.98.0; opt-level 3, thin LTO, one codegen unit; no perf instrumentation.
- Apple M5 Max, 18 logical cores, 128 GiB RAM, Darwin 27.3.0 arm64.
- Node 26.11.0, jsPDF 2.5.2, jspdf-autotable 3.8.4.
- Native addon SHA256: `4a44140061dd3d560399c5670c8daa0e6cfba2c8b34943eca8aa9363e2b272f1`.

## Method

Three complete **Turbo / jsPDF / jsPDF / Turbo** cycles: six blocks per engine.
Each block runs all four workloads, with 10 untimed warmups then 100 renders per
cold/warm mode. This is 600 measured renders per engine/workload/mode. Values
below are medians of the six block p50 and p95 statistics, not pooled percentiles.
Raw samples, hashes and per-block CPU snapshots are in the
[measurement receipt](measurements/2026-11-13-v0.8.4-abba.json).

Warm Turbo uses compiled HTML and a reusable font handle. Cold Turbo compiles
HTML and prepares fonts per document. jsPDF creates/registers each document's
fonts in both modes; package loading occurs outside timed renders. This measures
render latency, not process startup. Both use Evolventa regular/bold, A4 geometry
and the same complete source content. All-page Poppler text extraction, page
counts and native tagged/plain checks run outside timers. These are HTML/CSS
layout and manual drawing implementations, not pixel-identical renderings. No
images are included in these four workloads.

Coordinated builds and test jobs were paused during measurement. OS/VM and
background-service activity still occurred, including a slower block in the
second cycle; this was retained, not discarded. The receipt records the activity.
The results are machine/run-specific, not a claim of an isolated host. There are
no new peak-RSS or multicore-throughput measurements in this report.

## Latency (milliseconds, lower is better)

| Workload | Mode | Turbo p50 / p95 | jsPDF p50 / p95 |
|---|---|---:|---:|
| invoice | warm | 1.39 / 1.50 | 2.57 / 3.83 |
| invoice | cold | 1.85 / 2.09 | 2.44 / 3.55 |
| report-100 | warm | 10.55 / 12.41 | 6.96 / 8.27 |
| report-100 | cold | 12.74 / 14.59 | 6.74 / 8.45 |
| legal | warm | 11.81 / 14.16 | 7.81 / 8.50 |
| legal | cold | 11.96 / 14.24 | 8.13 / 9.78 |
| mixed | warm | 5.52 / 5.84 | 5.06 / 6.39 |
| mixed | cold | 6.04 / 7.06 | 4.82 / 6.11 |

Turbo is faster on the invoice. jsPDF remains faster on the 100-row report,
legal text and mixed workload. Older cross-version timings are not paired with
this run and must not be used to infer a precise percentage improvement.

## Correctness and document sizes

| Workload | Pages, both engines | Turbo PDF bytes | jsPDF PDF bytes |
|---|---:|---:|---:|
| invoice | 1 | 41,191 | 50,571 |
| report-100 | 3 | 146,758 | 158,433 |
| legal | 8 | 625,447 | 187,912 |
| mixed | 3 | 190,811 | 105,420 |

The combined native build passes 29 binding tests and all 526 workspace tests,
strict all-targets Clippy and formatting. DOM's combined layout gate passes
1,198 tests. All 16 native preflight outputs (four workloads, tagged/plain,
cold/warm) are byte-identical to the frozen 0.8.4 baseline from before these two changes.
Full source-word occurrence counts are checked on every PDF page for both engines,
including legal paragraphs and footnotes. This is not a formal PDF/UA validation
or a new screenshot-equivalence measurement.

## Reproduce

Install the benchmark optional dependencies plus Poppler (`pdftotext`, `pdfinfo`),
then run from the repository root with no competing builds/tests:

```sh
cargo +1.98.0 build --locked --release -p turbo-pdf-napi
node crates/turbo-pdf-napi/scripts/copy-addon.mjs
node benches/competitive/scripts/abba/fair-paired.mjs \
  crates/turbo-pdf-napi/turbo-pdf-napi.node \
  benches/competitive/results/abba-rerun invoice,report-100,legal,mixed 100
python3 benches/competitive/scripts/abba/summarize.py \
  benches/competitive/results/abba-rerun/fair-paired.json
```

Use the same fonts and complete workloads. Preserve all repetitions and load
records. Historical multi-engine numbers remain in [RESULTS.md](RESULTS.md) with
their original release labels.
