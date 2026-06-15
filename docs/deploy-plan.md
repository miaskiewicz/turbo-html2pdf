# Deploy plan — publishing `@turbo-pdf/*` to npm

> **Status: ACTIVE (Phase 16).** The release workflow now runs in CI. The napi
> crate it builds (`crates/turbo-pdf-napi`) **exists** (landed Phase 10), so the
> former `.github/workflows/release.yml.draft` has been promoted to a live
> `.github/workflows/release.yml`. It triggers on a `v*` tag (first release:
> `v0.1.0`) and is gated so nothing publishes off an ordinary push.

## 1. What we are publishing

| npm package           | Source                       | Kind                    | Depends on        |
| --------------------- | ---------------------------- | ----------------------- | ----------------- |
| `@turbo-pdf/napi`     | `crates/turbo-pdf-napi`      | native (napi prebuilds) | —                 |
| `@turbo-pdf/react`    | `packages/react`             | pure TS (tsup → dist)   | `@turbo-pdf/napi` |
| `@turbo-pdf/template` | `packages/template`          | pure TS (tsup → dist)   | `@turbo-pdf/napi` |

`@turbo-pdf/napi` is the only package with a native component: it is the napi-rs
front-end crate wrapping `turbo-pdf-core`, and the npm package *is* that crate
directory (its `package.json` carries the `napi` config; `index.js` is the
runtime loader). `react` and `template` are the existing pnpm workspace packages
under `packages/*`; they already build with `tsup` and emit `dist/` (see their
`package.json` `files: ["dist"]` + `build: "tsup"`). They consume the engine
through `@turbo-pdf/napi`.

## 2. The proven sibling pattern (what we copied)

Both sibling repos were inspected. **Neither has a dedicated `*release*`
workflow** — their publish job lives inside `ci.yml`, gated on a version tag. We
lift the same job shape into a standalone `release.yml` (so we never touch the
live `ci.yml`). The two reference files:

- `/Users/grzegorzmiaskiewicz/github-flux/turbo-dom/.github/workflows/ci.yml`
  — **the napi analog and the model for `@turbo-pdf/napi`.**
- `/Users/grzegorzmiaskiewicz/github-flux/turbo-test/.github/workflows/ci.yml`
  — a native-**binary** (CLI) variant; same shape, but ships a compiled `bin`
  rather than `*.node` addons. Corroborates the tag-gate + artifact + assemble +
  `npm publish` flow.

### 2.1 The chosen shape: single bundled package, not per-platform sub-packages

turbo-dom's `package.json` declares a napi config but **ships every platform
`*.node` binary inside the one `turbo-dom` package** — its loader picks the
matching local `*.node` at runtime. The publish job bundles all binaries into
the package root and runs a single `npm publish`. From
`turbo-dom/.github/workflows/ci.yml`, the `publish` job comment states it:

> "Single bundled package: all platform `*.node` binaries ship inside
> `turbo-dom` (index.js loads the matching local binary). One package to
> publish — no per-platform sub-packages to create."

We adopt the **single bundled package** strategy for `@turbo-pdf/napi`. It
avoids minting and authenticating five separate `@turbo-pdf/napi-<platform>`
packages. Our `index.js` loader resolves
`turbo-pdf-napi.<platform>.node` (the suffix `napi build --platform` emits,
e.g. `turbo-pdf-napi.darwin-arm64.node`, `turbo-pdf-napi.linux-x64-gnu.node`,
with musl-vs-glibc detection on Linux), and falls back to an unsuffixed
`turbo-pdf-napi.node` / the raw cargo cdylib for local dev.

### 2.2 The build matrix — turbo-dom's matrix MINUS mac-intel

turbo-dom's `build` job matrix, **minus `x86_64-apple-darwin` (mac-intel),
which is intentionally excluded** (slow, unwanted). The remaining five legs:

```
matrix.include:
  - { os: ubuntu-latest,  target: x86_64-unknown-linux-gnu }
  - { os: ubuntu-latest,  target: x86_64-unknown-linux-musl }
  - { os: ubuntu-latest,  target: aarch64-unknown-linux-gnu }
  - { os: macos-latest,   target: aarch64-apple-darwin }   # apple silicon only
  - { os: windows-latest, target: x86_64-pc-windows-msvc }
```

turbo-dom itself never lists `x86_64-apple-darwin` in its matrix either, so this
matrix is turbo-dom's verbatim. The one place mac-intel could sneak in is the
napi `triples.defaults: true`, whose default set *does* include
`x86_64-apple-darwin` — so our `crates/turbo-pdf-napi/package.json` sets
`triples.defaults: false` and lists exactly the five chosen targets, keeping
mac-intel out of every code path.

Per-leg steps we reproduce (this repo is a **pnpm 9** workspace, so node deps go
through `pnpm` instead of turbo-dom's `npm`):

- `actions/checkout@v4`, `pnpm/action-setup@v4` (v9), `actions/setup-node@v4`
  (node 22, `cache: pnpm`).
- `dtolnay/rust-toolchain@stable` with `targets: ${{ matrix.target }}`.
- musl leg: `apt-get install -y musl-tools`.
- aarch64-gnu leg: `apt-get install -y gcc-aarch64-linux-gnu` + export
  `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER` / `CC_aarch64_unknown_linux_gnu`.
- `Swatinem/rust-cache@v2` keyed on the target.
- `pnpm install --frozen-lockfile --ignore-scripts` then, from
  `crates/turbo-pdf-napi`, `pnpm exec napi build --platform --release --target
  ${{ matrix.target }}`.
- `actions/upload-artifact@v4` of `crates/turbo-pdf-napi/*.node`,
  `if-no-files-found: error`.

### 2.3 The publish jobs (from turbo-dom, distilled)

`publish-napi`:

- Triggers only on a tag: `if: startsWith(github.ref, 'refs/tags/v')`.
- `needs: [build-napi, gate]` (turbo-dom's publish `needs: [build, test]`; our
  `gate` is the Rust fmt/clippy/test equivalent — see §5).
- `actions/setup-node@v4` with `registry-url: https://registry.npmjs.org`.
- `actions/download-artifact@v4` → `find artifacts -name '*.node' -exec cp {}
  crates/turbo-pdf-napi/ \;` to drop every binary into the package root.
- `npm publish --access public --ignore-scripts` (working-directory
  `crates/turbo-pdf-napi`) with `env: NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}`.
  `--ignore-scripts` because the binaries are already built.

`publish-frontends` (after `publish-napi`, also tag-gated): a `pkg:
[react, template]` matrix, `pnpm install`, `pnpm --filter @turbo-pdf/<pkg>
build`, `pnpm --filter @turbo-pdf/<pkg> publish --access public --no-git-checks`.

### 2.4 NPM_TOKEN

Both siblings document the same requirement, e.g. turbo-test's `publish` job:

> "Requires repo secret `NPM_TOKEN` (a Classic Automation token allowed to
> publish the package)."

We need one repo secret `NPM_TOKEN`, a Classic **Automation** token on the npm
account that owns the `@turbo-pdf` scope, with publish rights to the scope
(**already configured in GitHub**). `registry-url` on `setup-node` wires it
through `NODE_AUTH_TOKEN`.

### 2.5 napi `package.json` config (now present)

`crates/turbo-pdf-napi/package.json` carries the `napi` block that drives
NAPI-RS, the matrix targets (mac-intel excluded), `publishConfig.access:
public`, and `@napi-rs/cli` in `devDependencies`:

```json
"publishConfig": { "access": "public" },
"napi": {
  "name": "turbo-pdf-napi",
  "triples": {
    "defaults": false,
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

`files` lists `*.node` + `index.js` / `index.d.ts` + the crate sources. The Rust
side already has `crate-type = ["cdylib", "rlib"]` and napi deps in
`crates/turbo-pdf-napi/Cargo.toml`.

## 3. Prerequisites — all met

1. ✅ **`crates/turbo-pdf-napi` exists** (Phase 10). napi-rs front-end crate
   wrapping `turbo-pdf-core`, `crate-type = ["cdylib", "rlib"]`. `cargo build -p
   turbo-pdf-napi` succeeds.
2. ✅ **`@turbo-pdf/napi` package manifest** — the crate dir doubles as the npm
   package (turbo-dom layout). Carries the `napi` config from §2.5,
   `"name": "@turbo-pdf/napi"`, `"version": "0.1.0"`, `publishConfig`.
   Root `package.json` `workspaces` already lists `"crates/turbo-pdf-napi"`.
3. ⚙️ **`NPM_TOKEN` repo secret** — configured in GitHub (§2.4).
4. ⚙️ **`@turbo-pdf` scope** on npm, owned by the token's account.
5. ✅ **`@turbo-pdf/react` / `@turbo-pdf/template`** publish via `pnpm publish`,
   which rewrites any `workspace:` protocol dependency to a real version range.

## 4. Version strategy

- **First public tag: `v0.1.0`.** All three packages moved from their `0.0.1`
  placeholders to `0.1.0` and publish in lockstep.
- **Lockstep, tag-driven.** A single git tag `vX.Y.Z` triggers the whole
  release, matching both siblings (which gate on `refs/tags/v*`).
- The three packages share one version number so a `react`/`template` release
  always pins the `napi` engine it was tested against.
- Bump all three `package.json` versions before each new tag (npm rejects
  re-publishing an existing version).

## 5. Publish order (the live `release.yml` job graph)

The native package must be on the registry before the TS packages that depend on
it resolve. Within the tagged release:

1. **`build-napi`** — the five-leg matrix produces every `*.node` (parallel).
2. **`gate`** — Rust fmt/clippy/test. We do **not** edit the live `ci.yml`; the
   gate re-runs the minimal Rust check inline (mirrors turbo-dom's publish
   `needs: [build, test]` intent).
3. **`publish-napi`** — download artifacts, bundle `*.node` into the package
   root, `npm publish --access public --ignore-scripts`. Gated on the tag.
4. **`publish-frontends`** — `needs: [publish-napi]`; `tsup` build then `pnpm
   publish` for `react` and `template`. Gated on the tag.

Order 3 → 4 is the load-bearing constraint. `react` and `template` are
order-independent of each other (separate matrix legs).

## 6. How this maps onto the siblings (citation summary)

| This workflow | Sibling source |
| ------------- | -------------- |
| napi build matrix (5 targets = turbo-dom minus mac-intel, cross-toolchain steps, rust-cache) | `turbo-dom/.github/workflows/ci.yml` `build` job |
| `napi build --platform --release --target …` | `turbo-dom/.github/workflows/ci.yml` + `turbo-dom/package.json` scripts |
| single bundled package, loader picks `*.node` | `turbo-dom/index.js` (NAPI-RS loader) + `turbo-dom/package.json` (`files`, `napi`) |
| tag-gated publish `if: startsWith(github.ref,'refs/tags/v')` | both siblings' `publish` job |
| download-artifact → copy `*.node` to package root → `npm publish --access public` | `turbo-dom/.github/workflows/ci.yml` `publish` job |
| `NPM_TOKEN` Classic Automation token → `NODE_AUTH_TOKEN` | both siblings' `publish` job comments |
| napi `Cargo.toml` `crate-type = ["cdylib","rlib"]` + napi deps | `turbo-dom/Cargo.toml` |

The CLI-binary variant in `turbo-test/.github/workflows/ci.yml` (staging a `bin/`
instead of `*.node`, `continue-on-error` for an optional musl leg, a startup
smoke test) is not adopted as-is — our engine is a library addon, not a CLI — but
its `build → upload-artifact → download → assemble → publish` skeleton is
identical and confirms the pattern.

## 7. Local dry run (optional, no publish)

```bash
# Build the native addon for the host platform:
cd crates/turbo-pdf-napi && pnpm exec napi build --platform --release
# Inspect the tarball that would be published (does not publish):
npm pack
tar -tzf turbo-pdf-napi-*.tgz | head -40
```

## 8. Validation status

- `release.yml` YAML parses cleanly (validated with a YAML parser).
- `actionlint` was **not** run — it is not installed in this environment
  (`which actionlint` → not found, and the sandbox cannot install it). Run
  `actionlint .github/workflows/release.yml` before the first tag if available.
- `cargo build -p turbo-pdf-napi` succeeds (the crate the workflow builds).
