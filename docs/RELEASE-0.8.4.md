# v0.8.4 candidate preparation

The release branch retains **0.8.4** across the Rust workspace/lockfile, Node
packages and Python project. The private root package remains 0.0.1. Generated
WASM package versions are stamped by the publishing workflow.

## Integrated changes

`turbo-dom` and `turbo-dom-layout` require `=0.8.4` and temporarily resolve from
`4238c125d922749cfc5ea3040ddc250447a45a70`. This includes the validated ASCII
direction shortcut and shared preparation for paired intrinsic text widths.
Both consumed crate trees are byte-identical to the benchmarked `8266b30a`
revision. The ABBA report retains that measured revision. The validation below
identifies which revisions executed the broader checks; the current pin also
passes its own workspace compile check. The separate Cow
cascade experiment is excluded because measured latency did not improve reliably. See [CHANGELOG.md](../CHANGELOG.md) for the full release.

## Validation recorded on 2026-11-13

- Workspace checked at `ed985b90`: **526 tests pass**, zero failures/ignores.
  Both consumed crate trees are verified unchanged at the current pin.
- Native addon checked at `dac92864`: **29 binding tests pass**, zero skips.
  The current pin passes a workspace compile check with identical consumed sources.
- Strict all-targets Clippy and formatting pass.
- Upstream combined DOM layout suite: **1,198 tests pass**, with strict gates.
- All 16 native preflight PDFs remain byte-identical to the frozen baseline
  from before the two new layout optimizations.
- Three complete ABBA cycles compare the final native addon with jsPDF, with
  all-page content validation and recorded CPU-load snapshots. The
  [report and raw samples](../benches/competitive/ABBA-0.8.4.md) retain cold/warm
  results and background-load limitations.

## Before publishing

Follow [PUBLISHING.md](../PUBLISHING.md). The matching turbo-dom 0.8.4 registry
releases must exist before removing the temporary git patch and re-locking from
crates.io. Then run both tag guards, package checks and the final release CI
gates, including NAPI, WASM, Python and coverage. Use the actual green release
merge commit for npm/crates/MCP and Python tags.

This preparation record is not evidence of registry publication or passing
release CI. Registry publication and tags remain separate release steps.
