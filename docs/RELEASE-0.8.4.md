# v0.8.4 candidate preparation

The release branch retains **0.8.4** across the Rust workspace/lockfile, Node
packages and Python project. The private root package remains 0.0.1. Generated
WASM package versions are stamped by the publishing workflow.

## Integrated changes

`turbo-dom` and `turbo-dom-layout` require `=0.8.4` and temporarily resolve from
`688a1e3f4197c4fdf5ec55f3ab1c98da642e4948`. This includes the validated ASCII
direction shortcut and shared preparation for paired intrinsic text widths.
The current pin adds quirks viewport and paragraph-margin behavior; it is not
source-identical to the historical benchmark pin. The ABBA report retains its
measured `8266b30a` revision. The separate Cow cascade experiment is excluded
because it did not improve latency reliably. See [CHANGELOG.md](../CHANGELOG.md).

## Validation provenance

Fresh validation on 2026-11-18 for `688a1e3f`:

- **526 workspace tests pass**, zero failures/ignores.
- **29 native binding tests pass**, zero skips, using the optimized addon.
- Strict all-targets Clippy, formatting and the release build pass.
- All **16 native PDFs** (four workloads, tagged/plain, cold/warm) are byte-identical
  to the saved `8266b30a` outputs. Full-page text and page counts are validated.
- Dependency metadata and packaged notices pass. The disposable Cargo build cache
  is removed after validation; the native addon and validation receipts remain.

The historical ABBA comparison remains tied to its measured `8266b30a` source.
The new layout/cascade source was tested for correctness, not rebenchmarked.
This record does not claim that the latest producer CI or release gates passed.

## Before publishing

Follow [PUBLISHING.md](../PUBLISHING.md). The matching turbo-dom 0.8.4 registry
releases must exist before removing the temporary git patch and re-locking from
crates.io. Then run both tag guards, package checks and the final release CI
gates, including NAPI, WASM, Python and coverage. Use the actual green release
merge commit for npm/crates/MCP and Python tags.

This preparation record is not evidence of registry publication or passing
release CI. Registry publication and tags remain separate release steps.
