# Publishing turbo-html2pdf

The Rust workspace, public Node packages and Python project release in lockstep.
The private root `turbo-pdf-monorepo` package remains 0.0.1; it is not published.
The workflows are the executable procedure:

| Tag | Workflow | Published artifacts |
| --- | --- | --- |
| `vX.Y.Z` | `.github/workflows/release.yml` | npm `turbo-html2pdf`, `-react`, `-template`, `-wasm`, `-wasm-fonts`, `-svg`; GitHub MCP archives |
| `vX.Y.Z` | `.github/workflows/release-crates.yml` | crates.io `turbo-html2pdf-core`, then `turbo-html2pdf-mcp` |
| `pyvX.Y.Z` | `.github/workflows/release-py.yml` | PyPI `turbo-html2pdf`: four platform wheels and an sdist |

`turbo-html2pdf-core` depends on `turbo-dom-layout` and `turbo-dom` (turbo-dom
workspace), so their matching releases must be on crates.io before
`release-crates.yml` publishes core (it waits, bounded, for the locked versions on the
crates.io index: `scripts/wait-crates-index.sh`). Until then the root `Cargo.toml`
carries a pre-publish `[patch.crates-io]` (git revs of the turbo-dom consolidation
branch); the [pre-tag steps](#pre-tag-drop-the-pre-publish-patch) drop it. All three tag
workflows run a tag-only `release-guard` job (`scripts/release-guard.sh`) that fails
while a `[patch]` section or a git source in `Cargo.lock` remains, or when the
workspace, Cargo.lock, napi/react/template `package.json`, `pyproject.toml` or mcp's
core pin disagree with the tag.

The npm packages `turbo-html2pdf` and `turbo-html2pdf-svg` ship
`assets/fonts/Go-Regular.ttf`, which is not tracked (no vendored font copy): the
publish jobs run `node scripts/package-notices.mjs --pack` (fails on a stale tracked
notice, then writes the font from turbo-dom-layout's `assets/fonts`) and assert the
packed file list contains it. Run the same command before `npm pack` locally.

Both annotated tags must identify the same actual release **merge commit**.
Pushing them publishes immutable registry versions; do this only with release
authorization. Merging an ordinary PR does not publish a version.

## Prepare one release after all prerequisite PRs

1. Read the actual registry versions, open PR heads and CI. Preserve stacked
   branches with ordinary merges; retarget their bases to main as ancestors land.
   Do not squash away required changes or tag individual prerequisite merges.
2. Update the workspace version, five package entries in `Cargo.lock`, Node
   packages under `crates/turbo-pdf-napi` and `packages/{react,template}`, and the
   Python `crates/turbo-pdf-py/pyproject.toml` version. Keep README and changelog
   current. Generated WASM package versions are stamped from the release tag.
   Leave unrelated third-party dependency versions and the private root alone.
3. Run the repository gates on the final production candidate. Existing exact
   source receipts may be reused only with an explicit byte comparison; do not
   relabel their source commits or claim a later source was executed.

   ```sh
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   cargo run --manifest-path tools/cc-check/Cargo.toml -- --max 5 crates
   pnpm lint
   pnpm format:check
   node scripts/package-notices.mjs --check
   ```

   CI additionally enforces 100% Rust coverage and builds/tests the actual NAPI,
   WASM and Python bindings. Require all three CI jobs green on the final release
   PR head. Keep feature-specific and genuine Chrome regression receipts for
   changed native layout/font behavior. Commit through the normal pre-commit
   hook; do not bypass its gates.
4. Inspect `cargo package -p turbo-html2pdf-core --list` and package verification.
   The crates release workflow runs `cargo package` before uploading core. MCP
   can resolve its registry core dependency only after core is published. Review
   npm package contents; native binaries and WASM outputs are assembled by the
   release matrix, not assumed present in a source-only checkout.
5. Merge every prerequisite PR, then the single release PR using merge commits.
   Resolve the final merge SHA from GitHub; verify its two parents and exact tree
   against the green release head. Recheck all open PRs and registry versions.

## Pre-tag: drop the pre-publish patch

For 0.6.0 (and any release made while `[patch.crates-io]` is present), on the release
PR head, after turbo-dom's `v0.6.0` publish finished:

```sh
# 1. turbo-dom-layout 0.6.0 and turbo-dom 0.6.0 must be on crates.io.
for c in turbo-dom turbo-dom-layout; do
  curl -fsSL "https://index.crates.io/tu/rb/$c" | grep -q '"vers":"0.6.0"' && echo "$c 0.6.0 ok"
done
# 2. Delete the `[patch.crates-io]` section (and its comment) from the root Cargo.toml.
# 3. Re-lock ONLY those two crates from crates.io (no unrelated lockfile churn).
cargo update -p turbo-dom-layout -p turbo-dom
git diff --stat Cargo.lock                    # only the two entries (source + checksum)
! grep -n 'source = "git+' Cargo.lock         # no git sources left
# 4. Guards and gates against the registry crates.
sh scripts/release-guard.sh v0.6.0 && sh scripts/release-guard.sh pyv0.6.0
cargo test --workspace
cargo package -p turbo-html2pdf-core --locked # full verify now that deps are published
node scripts/package-notices.mjs --check      # notices built from the registry crate
node scripts/package-notices.mjs --pack && (cd crates/turbo-pdf-napi && npm pack --dry-run)
# 5. Commit ("release: drop the turbo-dom pre-publish patch"), push, wait for green CI.
```

Then merge with a merge commit and tag that merge SHA (next section):
`git tag -a v0.6.0 "$RELEASE_SHA" -m "Release v0.6.0"` and
`git tag -a pyv0.6.0 "$RELEASE_SHA" -m "Python release v0.6.0"`, then
`git push origin v0.6.0 pyv0.6.0`.

## Tag, monitor and verify

After authorization and the green release merge, create annotated `vX.Y.Z` and
`pyvX.Y.Z` tags at that merge SHA, then push both. Never tag the PR branch tip.
The npm workflow requires real musl binaries and a successful Rust quality gate.
The crates workflow packages core, then publishes core before MCP. PyPI builds
wheels and sdist independently. Repository secrets `NPM_TOKEN`,
`CARGO_REGISTRY_TOKEN` and `PYPI_TOKEN` supply publication credentials; never put
credentials in files or logs. The crates/PyPI workflows can skip upload when a
secret is absent, so green build jobs alone are not registry publication proof.

Monitor all three tag workflows. Verify all six npm `latest` versions and packed
metadata/binaries, both crates.io versions and registry core dependency, and all
PyPI wheel/sdist versions. Verify the GitHub MCP archives too. Preserve failed
jobs and actual upload/index-wait outcomes; registry versions cannot be replaced.
If one registry fails after another succeeded, repair the concrete workflow
failure and retry only the missing publication, keeping the same release source.
