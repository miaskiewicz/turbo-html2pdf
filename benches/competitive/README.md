# Competitive benchmark harness (Phase 14, spec §10.2)

A reproducible matrix comparing **turbo-pdf** against the incumbent PDF engines on a
shared corpus of logical documents — same fonts, same A4 geometry, same text — so the
numbers are honest and not cherry-picked (spec §10.2 harness rules, AC-10.8–10.13).

> Status: external tooling is built now for every OTHER engine. The **turbo-pdf
> adapter is a stub** (`src/adapters/turbopdf.ts`) until Phase 10 (N-API) lands, and
> the `cargo xtask bench-competitive` Rust wiring is **deferred** — see
> [Deferred work](#deferred-work). The harness runs end-to-end today with whatever
> subset of engines is installed; missing engines are skipped gracefully, never
> hard-failed.

## What it measures

Per `(engine × workload)`:

- **Cold start / init** — process or engine spin-up (Chromium launch is the killer,
  measured explicitly, AC-10.9).
- **Warm render latency** — p50 and p95 over ≥100 runs (configurable).
- **Throughput** — documents/sec at concurrency = core count.
- **Peak RSS** — sampled process RSS during the measured runs.
- **Output PDF size** — bytes of the produced PDF.
- **Distribution footprint** — whether the engine ships a browser, dependency notes.
- **PNG equivalence** — page-1 rasterized via poppler and `pixelmatch`-diffed against
  a reference engine; a similarity score proves the renders are comparable and flags
  "winning by doing less" (AC-10.10).

## Workloads (claim-gating ids)

Every public perf claim must cite one of these ids (AC-10.13):

| id           | shape                                                         |
|--------------|--------------------------------------------------------------|
| `invoice`    | 1 page, light table, header/footer.                          |
| `report-100` | 100-row table, paginated, repeating header row.             |
| `report-1k`  | 1 000-row table.                                             |
| `report-10k` | 10 000-row table (~hundreds of pages).                      |
| `legal`      | long flowing text + footnotes (constructs browsers can't do natively). |
| `mixed`      | headings + table + flowing prose.                           |

Content is generated deterministically from a pinned seed and a fixed date
(`src/workloads/data.ts`) so reruns are byte-stable (spec §0 determinism). Fonts are
the engine crate's bundled Evolventa faces under
`crates/turbo-pdf-core/assets/fonts/` (referenced by relative path, never copied).

## Engines

| Engine                 | Approach            | Install needs                          |
|------------------------|---------------------|----------------------------------------|
| turbo-pdf **(stub)**   | native N-API        | pending Phase 10                       |
| Puppeteer              | headless Chromium   | npm + Chromium download                |
| Playwright             | headless Chromium   | npm + `npx playwright install chromium`|
| Gotenberg              | Chromium over HTTP  | docker container                       |
| `@react-pdf/renderer`  | React + Yoga        | npm                                    |
| PDFKit                 | Node draw API       | npm                                    |
| jsPDF (+autotable)     | draw API            | npm                                    |
| Typst                  | typesetting binary  | `typst` on PATH                        |
| wkhtmltopdf (legacy)   | QtWebKit            | `wkhtmltopdf` on PATH                  |
| WeasyPrint (reference) | Python/Cairo        | `weasyprint` on PATH                   |

Each adapter (`src/adapters/`) detects availability (binary on PATH / npm package /
docker HTTP) and **skips gracefully** if absent.

## Install

The harness itself only needs the monorepo install:

```bash
pnpm install            # from repo root; this package is globbed via benches/*
```

The competitor engines are **optional** — install only the ones you want to measure.

### Node engines (npm)

```bash
cd benches/competitive
pnpm add puppeteer playwright @react-pdf/renderer pdfkit jspdf jspdf-autotable react
npx playwright install chromium      # Playwright needs its own browser binary
```

### Typst

- macOS: `brew install typst`
- Linux: download the release binary from the Typst GitHub releases, put it on PATH.
- Cargo: `cargo install typst-cli`

### wkhtmltopdf (legacy reference)

- macOS: `brew install --cask wkhtmltopdf`
- Debian/Ubuntu: `apt-get install wkhtmltopdf` (or the upstream `.deb` for the patched
  Qt build).

### WeasyPrint (Python reference)

- macOS: `brew install weasyprint` (or `pipx install weasyprint`)
- Debian/Ubuntu: `apt-get install weasyprint` (pulls Cairo/Pango/GDK-Pixbuf).

### Gotenberg (docker)

```bash
docker run --rm -p 3000:3000 gotenberg/gotenberg:8
# override the URL with GOTENBERG_URL if not on localhost:3000
```

### PNG equivalence (poppler)

`pdftoppm` rasterizes PDFs for the pixel diff. Without it, similarity is simply not
scored (the rest of the harness still runs).

- macOS: `brew install poppler`
- Debian/Ubuntu: `apt-get install poppler-utils`

## Run

```bash
cd benches/competitive

pnpm bench:smoke       # quick: 5 runs, 1 warmup, all workloads
pnpm bench:full        # real: 100 runs, 10 warmup (spec §10.2 ≥100 runs)

# custom:
node --experimental-strip-types src/cli.ts \
  --runs 100 --warmup 10 \
  --workloads invoice,report-1k,legal \
  --reference pdfkit
```

### Output

- `results/results.json` — raw artifact (gitignored via root `/benches/**/results/`).
- `RESULTS.md` — regenerated markdown table, **never hand-edited** (AC-10.12). Records
  hardware/OS + per-engine versions (AC-10.11).

`--reference <id>` selects the engine whose render is the PNG-equivalence baseline
(default `pdfkit`, an always-available pure-JS engine). Once Phase 10 lands, the
WeasyPrint or Puppeteer render is the more honest fidelity baseline.

## Deferred work

- **`TODO(phase14)` in `src/adapters/turbopdf.ts`** — wire the real turbo-pdf adapter
  once Phase 10 (N-API) exposes `compile()` / `Program.render()`. Cold = include
  compile; warm = reuse the cached `Program` (AC-10.4 amortization). Until then the
  adapter reports itself unavailable so the matrix still runs end-to-end.
- **`cargo xtask bench-competitive`** (AC-10.8) — a thin Rust wrapper that shells into
  this harness (`pnpm bench:full`) and copies the artifact into CI. Deferred: there is
  no `xtask` crate yet and Phase 14's brief is to **not** touch the root workspace
  `Cargo.toml`. Equivalent today: `pnpm --filter @turbo-html2pdf/bench-competitive bench`.
- **CI artifact + fixed-runner regeneration** (AC-10.12) — add once the xtask exists.

## Claim-gating (AC-10.13)

Every public performance claim ("Nx faster than Puppeteer") MUST cite a specific
`(workload id, machine)` from `RESULTS.md` and link the reproducing command. No
freestanding multipliers. The metrics table is keyed by engine + workload id precisely
so claims map 1:1 to a workload.
