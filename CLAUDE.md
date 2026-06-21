# turbo-html2pdf — contributor & release notes

Native HTML/CSS + Jinja → PDF engine (Rust core) shipped to npm and PyPI via
N-API / WebAssembly / PyO3 bindings.

## Release runbook

Releases are **tag-driven**. Two independent tag prefixes:

| Tag      | Workflow             | Publishes to                                                                 |
| -------- | -------------------- | ---------------------------------------------------------------------------- |
| `vX.Y.Z`  | `.github/workflows/release.yml`    | **npm**: `turbo-html2pdf`, `-react`, `-template`, `-wasm`, `-wasm-fonts`, `-svg`; **GitHub Release**: `turbo-html2pdf-mcp` per-platform binary archives (`publish-mcp`) |
| `pyvX.Y.Z`| `.github/workflows/release-py.yml` | **PyPI**: `turbo-html2pdf` (maturin abi3 wheels + sdist)                  |

Required GitHub repo secrets: **`NPM_TOKEN`** (npm automation token, public
publish) and **`PYPI_TOKEN`** (PyPI API token). If `PYPI_TOKEN` is unset the
PyPI publish step self-skips (builds wheels, uploads nothing).

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
- `benches/competitive/RESULTS.md` — the version column (benchmark snapshot)

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
```

`release.yml`'s `gate` job runs only fmt/clippy/test — so a red `cc-check` or
coverage gate does NOT block a publish. Verify them locally anyway; a tag is
public and hard to walk back.

### 3. Tag + push

```
git push origin main          # push commits first (CI runs on main)
git tag -a vX.Y.Z   -m "..."  # npm  → release.yml
git tag -a pyvX.Y.Z -m "..."  # PyPI → release-py.yml (needs PYPI_TOKEN)
git push origin vX.Y.Z pyvX.Y.Z
```

Watch: `gh run list`. The npm `build-napi` matrix (5 platforms) takes ~10-15 min.
Verify after: `npm view turbo-html2pdf@X.Y.Z version` and the others;
`npm view turbo-html2pdf@X.Y.Z dist.tarball`.

## Feature gates (turbo-pdf-core)

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
It is a Rust binary, NOT an npm/PyPI package: there is no publish job; consumers
`cargo build -p turbo-html2pdf-mcp --release`. It is excluded from the coverage
gate (`tarpaulin.toml`) like the other bindings; its surface is unit-tested
in-crate. It inherits the workspace version, so it needs no manual version bump.

## Known gaps

- **`turbo-html2pdf-svg` IS published.** The napi crate carries the
  `svg = ["turbo-pdf-core/svg"]` passthrough feature
  (`crates/turbo-pdf-napi/Cargo.toml`), and `release.yml` builds it on every
  `v*` tag via the `build-napi-svg` / `build-napi-svg-musl` jobs and ships it
  under the `turbo-html2pdf-svg` npm name from `publish-svg`. (Earlier revisions
  of this file said it was unpublished — that is no longer true.) The MCP crate
  also carries an `svg` passthrough; the standard binary omits it, build it with
  `cargo build -p turbo-html2pdf-mcp --release --features svg` for vector `<img>`.
