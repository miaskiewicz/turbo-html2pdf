# turbo-html2pdf

A native (Rust) HTML/CSS-to-PDF engine with a Jinja-compatible templating DSL,
fronted by a thin TS/N-API layer. A template is compiled once to a reusable
`Program`, then executed against data to emit PDF bytes. No browser, no DOM at
the hot path, threadable concurrency.

Two principles shape everything:

1. **The core is platform/app-agnostic.** Its only input contract is parseable
   template HTML/CSS containing the required fields, plus a `t:` DSL for
   paged-media constructs. Authoring frontends (React, Vue, plain strings) are
   separate, optional layers that all emit the same template HTML.
2. **Pagination is automatic by default.** Content defines a flow; the engine
   paginates it. Page masters, named pages, and forced breaks are *overrides*,
   never a precondition for producing a PDF.

See [`docs/spec.md`](docs/spec.md) for the full implementation specification.

## Status

Built in phases per the spec's build order. Track progress in the issue list /
task board. Current focus: the Rust core (templating → layout → pagination →
PDF emit), then N-API, React frontend, WASM, and the competitive benchmark suite.

## Repository layout

```
crates/turbo-pdf-core   # the engine (templating, layout, pagination, PDF emit)
crates/turbo-pdf-napi   # Node N-API binding              (later phase)
packages/react          # @turbo-html2pdf/react authoring frontend (later phase)
tools/cc-check          # Rust cyclomatic-complexity gate (cc < 6)
scripts/cc-check.js     # TS/JS cyclomatic-complexity gate (cc < 6)
benches/                # criterion + competitive benchmark harness (later phase)
```

## Engineering gates

- **Tests:** 100% line coverage (`cargo tarpaulin --fail-under 100`).
- **Complexity:** every function has cyclomatic complexity ≤ 5
  (`cargo run -p cc-check` for Rust, `scripts/cc-check.js` for TS/JS).
- **Rust:** `cargo fmt` + `clippy -D warnings`.
- **JS/TS:** `oxlint` (lint), `biome` (format), `tsgo` (typecheck).
- A pre-commit hook (`.githooks/pre-commit`) runs the relevant gates on staged
  files. Enable it with `git config core.hooksPath .githooks`.

## License

MIT — see [LICENSE](LICENSE).
