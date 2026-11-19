# v0.8.5 release preparation

The release branch sets **0.8.5** across the Rust workspace/lockfile, Node
packages and Python project. The private root package remains 0.0.1. Generated
WASM package versions are stamped by the publishing workflow.

## Integrated changes

`turbo-dom` and `turbo-dom-layout` use published exact `=0.8.5` registry
requirements with registry checksums in `Cargo.lock`. Re-locking changed only
those two entries. No git patch or git source was used at any point in this
release. turbo-dom 0.8.5 only adds public Rust API items relative to 0.8.4 (see
turbo-dom's `tools/release/public-api-0.8.5.txt`), so no source changes were
needed here. Its layout fixes (flex row minimums, auto-fit gutters, signed column
margins, percentage item frames, functional border colors, ratio-sized overlays,
typed `calc()`, mixed flex lengths) ship through the dependency. See
[CHANGELOG.md](../CHANGELOG.md).

turbo-dom-raster's `avif` feature now decodes through `turbo-dom-rav1d` 1.1.0.
turbo-html2pdf does not depend on turbo-dom-raster, so neither crate appears in its
dependency graph.

## Validation provenance

Local validation on 2026-11-19 against the registry crates:

- `release-guard.sh` passes for `v0.8.5` and `pyv0.8.5`; `cargo metadata --locked`
  resolves with no git sources.
- **526 workspace tests pass**, zero failures/ignores. No pinned PDF/pagination
  expectation changed against the 0.8.5 layout crate.
- Formatting, strict all-targets Clippy (`-D warnings`), the cyclomatic-complexity
  gate (≤ 5) and the packaged-notices check pass.
- `cargo package -p turbo-html2pdf-core --locked` verifies against the registry
  crates.

The historical ABBA comparison remains tied to its measured `8266b30a` source and
0.8.4 label; 0.8.5 was not rebenchmarked.

## Final release gates

Follow [PUBLISHING.md](../PUBLISHING.md). The final PR CI must validate the
registry dependency graph, including coverage and all binding builds (NAPI, WASM,
Python). Tag the actual green release merge commit with `v0.8.5` (npm, crates.io,
MCP) and `pyv0.8.5` (PyPI).

This preparation record is not evidence of registry publication or passing
release CI. Registry publication and tags remain separate release steps.
