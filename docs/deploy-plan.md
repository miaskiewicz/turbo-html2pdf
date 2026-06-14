# Deploy plan — publishing `@turbo-pdf/*` to npm

> **Status: DRAFT (Phase 16 groundwork).** Nothing here runs in CI yet. The
> native `@turbo-pdf/core` package depends on a napi crate
> (`crates/turbo-pdf-napi`) that is **planned for Phase 10 but does not exist
> yet**. The companion workflow ships as
> `.github/workflows/release.yml.draft` — the `.draft` suffix means GitHub will
> **not** execute it. Rename to `release.yml` only after Phase 10 lands.

## 1. What we are publishing

| npm package        | Source                          | Kind                  | Depends on            |
| ------------------ | ------------------------------- | --------------------- | --------------------- |
| `@turbo-pdf/core`  | `crates/turbo-pdf-napi` (Phase 10) | native (napi prebuilds) | —                  |
| `@turbo-pdf/react` | `packages/react`                | pure TS (tsup → dist) | `@turbo-pdf/core`     |
| `@turbo-pdf/template` | `packages/template`          | pure TS (tsup → dist) | `@turbo-pdf/core`     |

`@turbo-pdf/core` is the only package with a native component. `react` and
`template` are the existing pnpm workspace packages under `packages/*`; they
already build with `tsup` and emit `dist/` (see their `package.json` `files:
["dist"]` + `build: "tsup"`). They consume the engine through `@turbo-pdf/core`.

## 2. The proven sibling pattern (what we copy)

Both sibling repos were inspected. **Neither has a dedicated `*release*`
workflow** — the publish job lives inside `ci.yml`, gated on a version tag. The
two relevant files:

- `/Users/grzegorzmiaskiewicz/github-flux/turbo-dom/.github/workflows/ci.yml`
  — **this is the napi analog and the model for `@turbo-pdf/core`.**
- `/Users/grzegorzmiaskiewicz/github-flux/turbo-test/.github/workflows/ci.yml`
  — a native-**binary** (CLI) variant; same shape, but ships a compiled `bin`
  rather than `*.node` addons. Useful corroboration of the tag-gate + artifact
  + assemble + `npm publish` flow.

### 2.1 The chosen shape: single bundled package, not per-platform sub-packages

turbo-dom's `package.json` declares a napi config but **ships every platform
`*.node` binary inside the one `turbo-dom` package** — its NAPI-RS-generated
`index.js` loader picks the matching local `*.node` at runtime (and falls back
to a per-platform `@scope/...` sub-package only if the local file is absent).
The publish job bundles all binaries into the package root and runs a single
`npm publish`. From `turbo-dom/.github/workflows/ci.yml`, the `publish` job
comment states it directly:

> "Single bundled package: all platform `*.node` binaries ship inside
> `turbo-dom` (index.js loads the matching local binary). One package to
> publish — no per-platform sub-packages to create."

We adopt the **single bundled package** strategy for `@turbo-pdf/core`. It
avoids minting and authenticating five separate `@turbo-pdf/core-<platform>`
packages.

### 2.2 The build matrix (verbatim from turbo-dom)

`turbo-dom/.github/workflows/ci.yml`, `build` job:

```
matrix.include:
  - { os: ubuntu-latest,  target: x86_64-unknown-linux-gnu }
  - { os: ubuntu-latest,  target: x86_64-unknown-linux-musl }
  - { os: ubuntu-latest,  target: aarch64-unknown-linux-gnu }
  - { os: macos-latest,   target: aarch64-apple-darwin }
  - { os: windows-latest, target: x86_64-pc-windows-msvc }
```

Per-leg steps we reproduce:

- `actions/checkout@v4`, `actions/setup-node@v4` (node 22).
- `dtolnay/rust-toolchain@stable` with `targets: ${{ matrix.target }}`.
- musl leg: `apt-get install -y musl-tools`.
- aarch64-gnu leg: `apt-get install -y gcc-aarch64-linux-gnu` + export
  `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER` / `CC_aarch64_unknown_linux_gnu`.
- `Swatinem/rust-cache@v2` keyed on the target.
- `npm install --ignore-scripts` then
  `npx napi build --platform --release --target ${{ matrix.target }}`.
- `actions/upload-artifact@v4` of `*.node`, `if-no-files-found: error`.

### 2.3 The publish job (from turbo-dom, distilled)

- Triggers only on a tag: `if: startsWith(github.ref, 'refs/tags/v')`.
- `needs: [build]` (turbo-dom also `needs` its `test` job; we gate on our Rust
  test/clippy/coverage equivalents — see §5).
- `actions/setup-node@v4` with `registry-url: https://registry.npmjs.org`.
- `actions/download-artifact@v4` → `find artifacts -name '*.node' -exec cp {} . \;`
  to drop every binary into the package root.
- `npm publish --access public` with
  `env: NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}`.

### 2.4 NPM_TOKEN

Both siblings document the same requirement, e.g. turbo-test's `publish` job:

> "Requires repo secret `NPM_TOKEN` (a Classic Automation token allowed to
> publish the package)."

We need one repo secret `NPM_TOKEN`, a Classic **Automation** token on the npm
account that owns the `@turbo-pdf` scope, with publish rights to the scope.
`registry-url` on `setup-node` wires it through `NODE_AUTH_TOKEN`.

### 2.5 napi `package.json` config (to add in Phase 10)

turbo-dom's `package.json` carries the `napi` block that drives NAPI-RS:

```json
"napi": {
  "name": "turbo-pdf-core",
  "triples": {
    "defaults": true,
    "additional": [
      "x86_64-unknown-linux-gnu",
      "x86_64-unknown-linux-musl",
      "aarch64-unknown-linux-gnu",
      "aarch64-apple-darwin",
      "x86_64-pc-windows-msvc"
    ]
  }
}
```

…plus `files` listing `*.node` + the generated `index.js` / `index.d.ts`, a
`prepublishOnly: "napi build --platform --release"` rebuild guard, and
`@napi-rs/cli` in `devDependencies`. The Rust side needs
`crate-type = ["cdylib", "rlib"]` and napi deps gated behind a feature, exactly
as in `turbo-dom/Cargo.toml`:

```
napi        = { version = "2", default-features = false, features = ["napi6"], optional = true }
napi-derive = { version = "2", optional = true }
napi-build  = { version = "2", optional = true }
```

## 3. Prerequisites — what must exist before this plan activates

1. **`crates/turbo-pdf-napi` (Phase 10).** A napi-rs front-end crate wrapping
   `turbo-pdf-core`, `crate-type = ["cdylib", "rlib"]`, napi deps feature-gated.
   It currently does **not** exist — `crates/` holds only `turbo-pdf-core`, and
   the workspace `Cargo.toml` `members` lists only `crates/turbo-pdf-core`.
2. **`@turbo-pdf/core` package manifest.** Either the napi crate dir doubles as
   the npm package (like turbo-dom, where the Rust crate root *is* the package)
   or a thin `packages/core` wrapper. Must carry the `napi` config from §2.5 and
   `"name": "@turbo-pdf/core"`. NAPI-RS generates the `index.js` loader.
   - Note: root `package.json` `workspaces` already pre-lists
     `"crates/turbo-pdf-napi"`, so the npm-package-in-crate layout is the
     intended one. (pnpm itself globs `packages/*` + `benches/*` via
     `pnpm-workspace.yaml`; the `workspaces` array is the npm-compat hint.)
3. **`NPM_TOKEN` repo secret** (§2.4).
4. **`@turbo-pdf` scope** created on npm with the token's account as owner.
5. **`@turbo-pdf/react` / `@turbo-pdf/template`** must declare a real
   (non-`workspace:`) dependency range on `@turbo-pdf/core` at publish time, or
   have the `workspace:` protocol rewritten on publish (pnpm publish does this).

## 4. Version strategy

- **First public tag: `v0.1.0`.** All three packages move from their current
  `0.0.1` placeholders to `0.1.0` together and are published in lockstep. The
  workspace Rust `version` (`Cargo.toml` `[workspace.package] version = "0.0.1"`)
  bumps to match.
- **Lockstep, tag-driven.** A single git tag `vX.Y.Z` triggers the whole
  release. This matches both siblings, which gate on `refs/tags/v*`
  (turbo-dom is at `0.2.0`, turbo-test references `v0.2.3` in its smoke-test
  comment — both already past their `0.1.0`).
- The three packages share one version number so a `react`/`template` release
  always pins the `core` it was tested against.

## 5. Publish order

The native package must exist on the registry before the TS packages that
depend on it resolve. Within the tagged release:

1. **Build matrix** produces all five `*.node` binaries (parallel legs).
2. **Gate** on the existing Rust quality jobs (fmt/clippy/test/coverage from
   `ci.yml`) + a napi smoke load. turbo-dom's publish `needs: [build, test]`;
   we mirror that intent. (We do **not** edit the live `ci.yml`; the draft
   re-runs the minimal gate inline.)
3. **Publish `@turbo-pdf/core`** — download artifacts, bundle `*.node` into the
   package root, `npm publish --access public`.
4. **Publish `@turbo-pdf/react`** and **`@turbo-pdf/template`** — `tsup` build,
   then `npm publish --access public`. These have no native step; they only
   need `core` already on the registry so consumers can install the set.

Order 3 → 4 is the load-bearing constraint. `react` and `template` are
order-independent of each other.

## 6. How this maps onto the siblings (citation summary)

| This plan | Sibling source |
| --------- | -------------- |
| napi build matrix (5 targets, cross-toolchain steps, rust-cache) | `turbo-dom/.github/workflows/ci.yml` `build` job |
| `npx napi build --platform --release --target …` | `turbo-dom/.github/workflows/ci.yml` + `turbo-dom/package.json` scripts |
| single bundled package, loader picks `*.node` | `turbo-dom/index.js` (NAPI-RS loader) + `turbo-dom/package.json` (`files`, `napi`) |
| tag-gated publish `if: startsWith(github.ref,'refs/tags/v')` | both siblings' `publish` job |
| download-artifact → copy `*.node` to root → `npm publish --access public` | `turbo-dom/.github/workflows/ci.yml` `publish` job |
| `NPM_TOKEN` Classic Automation token → `NODE_AUTH_TOKEN` | both siblings' `publish` job comments |
| napi `Cargo.toml` feature-gating + `crate-type` | `turbo-dom/Cargo.toml` |

The CLI-binary variant in
`turbo-test/.github/workflows/ci.yml` (staging a `bin/` instead of `*.node`,
`continue-on-error` for the optional musl leg, a startup smoke test) is not
adopted as-is — our core is a library addon, not a CLI — but its
`build → upload-artifact → download → assemble → publish` skeleton is identical
and confirms the pattern.
