# turbo-html2pdf — contributor & release notes

Native HTML/CSS + Jinja → PDF engine (Rust core) shipped to npm and PyPI via
N-API / WebAssembly / PyO3 bindings.

## Release runbook

Releases are **tag-driven**. Two independent tag prefixes:

| Tag      | Workflow             | Publishes to                                                                 |
| -------- | -------------------- | ---------------------------------------------------------------------------- |
| `vX.Y.Z`  | `.github/workflows/release.yml`    | **npm**: `turbo-html2pdf`, `-react`, `-template`, `-wasm`, `-wasm-fonts`, `-svg`; **GitHub Release**: `turbo-html2pdf-mcp` per-platform binary archives (`publish-mcp`) |
| `vX.Y.Z`  | `.github/workflows/release-crates.yml` | **crates.io**: `turbo-html2pdf-core`, then `turbo-html2pdf-mcp` (ordered — core first). Same `v*` tag as npm |
| `pyvX.Y.Z`| `.github/workflows/release-py.yml` | **PyPI**: `turbo-html2pdf` (maturin abi3 wheels + sdist)                  |

Required GitHub repo secrets: **`NPM_TOKEN`** (npm token with public-package publish permission), **`PYPI_TOKEN`** (PyPI API token), and **`CARGO_REGISTRY_TOKEN`**
(crates.io API token). If `PYPI_TOKEN` is unset the PyPI publish self-skips; if
`CARGO_REGISTRY_TOKEN` is unset the crates.io publish self-skips (verifies the
package, uploads nothing).

The `turbo-html2pdf-mcp` crate depends on `turbo-html2pdf-core` with an explicit
`version = "0.3"` (^0.3) alongside its `path` — crates.io requires a version on
the dep. It only needs bumping on a **minor** (0.x) change, not every patch;
keep it in sync with the workspace major.minor when you cross a minor boundary.

### 1. Bump the version — EVERY place below (they are NOT auto-synced)

Set the same `X.Y.Z` in all of these before tagging:

- `Cargo.toml` → `[workspace.package] version` (crate metadata for all 5 crates,
  incl. `turbo-html2pdf-mcp`, which inherits the workspace version)
- `crates/turbo-pdf-napi/package.json` → `version`
- `packages/react/package.json` → `version`
- `packages/template/package.json` → `version`
- `crates/turbo-pdf-py/pyproject.toml` → `version`

Auto-stamped from the git tag at publish — **do NOT bump manually**:

- `turbo-html2pdf-wasm` and `turbo-html2pdf-wasm-fonts` — `release.yml` rewrites
  `package.json` `name` + `version` from `GITHUB_REF_NAME` in the publish job.

Cosmetic version strings to refresh (not functional, but keep in sync):

- `crates/turbo-pdf-napi/README.md` — the `## Status` line (`vX.Y.Z`). **This
  README ships in the npm tarball**, so a stale version is publicly visible.
- `benches/competitive/src/adapters/turbopdf.ts` — the `version:` in `detect()`
- Keep historical `benches/competitive/RESULTS.md` versions tied to the measured
  release; update them only when rerunning the benchmark.

`Cargo.lock` regenerates on the next `cargo build` — commit the churn.

Sweep for stragglers before tagging:
`grep -rn "OLD.VERSION" --include="*.json" --include="*.toml" --include="*.md" --include="*.ts" --include="*.rs" . | grep -vE "node_modules|/target/|Cargo.lock|pnpm-lock"`
(Historical mentions in `docs/deploy-plan.md` and the `release*.yml` "first tag"
comments describe past releases — leave those.)

### 2. Pre-tag gate (all must pass locally; CI re-runs them)

```
RUSTFLAGS="-D warnings" cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --manifest-path tools/cc-check/Cargo.toml -- --max 5 crates   # cc < 6
cargo tarpaulin                                                          # 100% gate
node scripts/package-notices.mjs --check                                # packaged notices
```

`release.yml`'s `gate` job runs only fmt/clippy/test — so a red `cc-check` or
coverage gate does NOT block a publish. Verify them locally anyway; a tag is
public and hard to walk back.

### 3. Merge, tag the merge commit, and push

Commit the version bump and fixes on the release PR. Run the local gates and
wait for every CI job to pass on that exact head before merging with a merge
commit. Verify the merge tree matches the tested head and tag that merge SHA,
not the branch head. Both tags below publish; push only when release is authorized.

```sh
git fetch origin main
# Set RELEASE_SHA to the verified merge commit from the merged PR.
git tag -a vX.Y.Z "$RELEASE_SHA" -m "Release vX.Y.Z"   # npm + crates + MCP
git tag -a pyvX.Y.Z "$RELEASE_SHA" -m "Python release vX.Y.Z" # PyPI
git push origin vX.Y.Z pyvX.Y.Z
```

Watch: `gh run list`. The npm `build-napi` matrix (5 platforms) takes ~10-15 min.
Verify after: `npm view turbo-html2pdf@X.Y.Z version` and the others;
`npm view turbo-html2pdf@X.Y.Z dist.tarball`.

## Feature gates (turbo-html2pdf-core)

Default build = `["bundled-fonts"]` only. Everything else is opt-in and kept out
of the default-coverage surface (gated-only modules are listed in
`tarpaulin.toml` `exclude-files`; gated branches in shared files are relocated
into gated-only sibling modules so the tarpaulin cfg-mapping quirk can't flag
them — see `layout/boxgen_ua.rs`, `layout/boxgen_xref.rs`).

- `bundled-fonts` (default): embeds Inter/Roboto, Liberation Serif/PT Serif,
  Fira Code/IBM Plex Mono so docs render with no caller fonts (~6 MB).
- `endnotes`, `xref`, `print-color`, `pdf-a`, `pdf-ua`, `append`, `encrypt`, `svg`.
- `pdf-a`/`pdf-ua`/`cmyk` are also **per-render** runtime toggles on
  `EmitOptions` (default off → byte-identical plain RGB/untagged output).

Bindings (napi/py) compile the full feature set EXCEPT `svg`. The browser `-wasm`
crate pins core `default-features = false` (no fonts) + the capability features;
`-wasm-fonts` adds `bundled-fonts`. The `.wasm` build needs
`RUSTFLAGS='--cfg getrandom_backend="wasm_js"'` (encrypt/append pull getrandom 0.3).

`crates/turbo-html2pdf-mcp` is a native **MCP server** (stdio JSON-RPC 2.0, no
SDK) over the same core pipeline — a `bin` (`turbo-html2pdf-mcp`) over a testable
protocol `lib`. It compiles the same full feature set EXCEPT `svg` and exposes
`render` / `append_pdf` / `check_template` (the union of the napi + py surface).
It is a Rust binary, NOT an npm/PyPI package. The `publish-mcp` job ships
per-platform GitHub Release archives; `release-crates.yml` also publishes the
crate for `cargo install turbo-html2pdf-mcp`. Local builds use
`cargo build -p turbo-html2pdf-mcp --release`. It is excluded from the coverage
gate (`tarpaulin.toml`) like the other bindings; its surface is unit-tested
in-crate. It inherits the workspace version, so it needs no manual version bump.

## Known gaps

- **`turbo-html2pdf-svg` IS published.** The napi crate carries the
  `svg = ["turbo-html2pdf-core/svg"]` passthrough feature
  (`crates/turbo-pdf-napi/Cargo.toml`), and `release.yml` builds it on every
  `v*` tag via the `build-napi-svg` / `build-napi-svg-musl` jobs and ships it
  under the `turbo-html2pdf-svg` npm name from `publish-svg`. (Earlier revisions
  of this file said it was unpublished — that is no longer true.) The MCP crate
  also carries an `svg` passthrough; the standard binary omits it, build it with
  `cargo build -p turbo-html2pdf-mcp --release --features svg` for vector `<img>`.
