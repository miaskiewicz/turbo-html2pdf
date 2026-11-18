# v0.8.4 release preparation

The release branch retains **0.8.4** across the Rust workspace/lockfile, Node
packages and Python project. The private root package remains 0.0.1. Generated
WASM package versions are stamped by the publishing workflow.

## Integrated changes

`turbo-dom` and `turbo-dom-layout` use published exact `=0.8.4` registry
requirements with registry checksums in `Cargo.lock`. There is no git patch or
git source in the release dependency graph. Both published crates were verified
not yanked. Their source/assets match the previously validated `688a1e3f` pin.
This includes ASCII direction and paired intrinsic text preparation, plus the
quirks document formatting-context fixes. The rejected Cow cascade experiment
is excluded. See [CHANGELOG.md](../CHANGELOG.md).

## Validation provenance

Previous source validation on 2026-11-18 for `688a1e3f` (the published crates
have byte-identical source/assets):

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

## Final release gates

Both npm/crates and Python tag guards pass after registry resolution. The final
PR CI must validate the registry dependency graph, including coverage and all
binding builds. Follow [PUBLISHING.md](../PUBLISHING.md). The matching DOM registry
releases are verified and the patch is removed. Run package checks and require
all final release CI gates, including NAPI, WASM, Python and coverage. Use the actual green release
merge commit for npm/crates/MCP and Python tags.

This preparation record is not evidence of registry publication or passing
release CI. Registry publication and tags remain separate release steps.
