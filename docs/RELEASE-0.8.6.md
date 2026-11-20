# v0.8.6 release preparation

The release branch sets **0.8.6** across the Rust workspace/lockfile, Node
packages and Python project. The private root package remains 0.0.1. Generated
WASM package versions are stamped by the publishing workflow.

## Integrated changes

`turbo-dom` and `turbo-dom-layout` use published exact `=0.8.6` registry
requirements with registry checksums in `Cargo.lock`. The pre-publish
`[patch.crates-io]` git rev was dropped once the DOM crates landed on crates.io,
and re-locking changed only those two entries. No git source remains in the
dependency graph.

turbo-dom 0.8.6 fixes the 0.8.5 grid baseline regression (turbo-dom #537): a
grid row with an `fr` track beside an `auto` track and `align-items: baseline`
now stays on the solved column width, matching Chrome 156. The downstream payslip
that rendered on two pages in 0.8.5 is back to one page.

`crates/turbo-html2pdf-core/tests/grid_baseline.rs` guards the regression: the
test fails on turbo-dom 0.8.5 (row is 22px instead of 11px; a twenty-row payslip
is 5 pages instead of 1) and passes on 0.8.6.

## Validation provenance

Local validation on 2026-11-20 against the registry crates:

- `release-guard.sh` passes for `v0.8.6` and `pyv0.8.6`; `cargo metadata --locked`
  resolves with no git sources.
- **528 workspace tests pass**, zero failures/ignores, including the new grid
  baseline regression test.
- Formatting, strict all-targets Clippy (`-D warnings`), the cyclomatic-complexity
  gate (≤ 5) and the packaged-notices check pass.
- Coverage (tarpaulin) at 100%.
- `cargo package -p turbo-html2pdf-core --locked` verifies against the registry
  crates.

The historical ABBA comparison remains tied to its measured `8266b30a` source and
0.8.4 label; 0.8.6 was not rebenchmarked.

## Final release gates

Follow [PUBLISHING.md](../PUBLISHING.md). The final PR CI must validate the
registry dependency graph, including coverage and all binding builds (NAPI, WASM,
Python). Tag the actual green release merge commit with `v0.8.6` (npm, crates.io,
MCP) and `pyv0.8.6` (PyPI).

This preparation record is not evidence of registry publication or passing
release CI. Registry publication and tags remain separate release steps.
