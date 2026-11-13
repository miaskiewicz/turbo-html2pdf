# AGENTS

Short reference for agents (and humans in a hurry) working on turbo-html2pdf.
Everything here is runnable through `scripts/agent/*.sh`; CLAUDE.md remains the
authoritative release runbook.

## Scripts

| Script | What it does |
| --- | --- |
| `scripts/agent/build-napi.sh` | `npm ci` + `napi build --platform --release` with scratchpad `CARGO_TARGET_DIR` and the cached `RUSTY_V8_ARCHIVE`. |
| `scripts/agent/test-all.sh [CRATE]` | fmt → clippy `-D warnings` → `cargo test` → cc-check (≤ 5) → tarpaulin (100%). |
| `scripts/agent/verapdf-corpus.sh [DIR]` | veraPDF `--flavour ua1` over every PDF in the UA corpus. Pass/fail counts; non-zero on any failure. |
| `scripts/agent/parity-compare.sh` | Run turbo-dom's `tools/parity/parity.py` against this repo's expected fixtures. |
| `scripts/agent/record-chrome-control.sh PROBE OUT` | Drive Chrome 156 headless-shell (`--use-mock-keychain --password-store=basic`, localhost only) for a `scripts/*-chrome-control.mjs` probe. |
| `scripts/agent/pin-turbo-dom.sh REV` | Pre-publish `[patch.crates-io]` pin of `turbo-dom{,-layout}` at a git rev. |
| `scripts/agent/drop-turbo-dom-pin.sh` | Remove the pin before tagging (required by `release-guard.sh`). |
| `scripts/agent/release-prep.sh X.Y.Z` | Bump every version string the runbook lists, then run both release guards. |

Each script prints `--help` with its full option list.

## Build loop

The fast inner loop for a Rust change:

```sh
scripts/agent/test-all.sh turbo-html2pdf-core   # one crate, all gates
```

For a wider change:

```sh
scripts/agent/test-all.sh                       # whole workspace
```

Scratchpad `CARGO_TARGET_DIR` lives at `/tmp/turbo-html2pdf-agent-<user>/` (override
with `SCRATCH_ROOT`). Delete it when you finish — the machine is shared with
sibling agents.

The N-API binding needs a separate build once JS/TS tests or the pre-commit hook
pull it in:

```sh
scripts/agent/build-napi.sh
```

## Fixtures & the UA corpus

- Layout / templating / paged / PDF fixtures: `crates/turbo-html2pdf-core/tests/fixtures/`
  (`invoice/`, `legal/`, `mixed/`, `report/`, `append/`, `stamp_font/`).
- Chrome-156 controls: `crates/turbo-html2pdf-core/tests/fixtures/*_chrome156.json`,
  recorded by `scripts/*-chrome-control.mjs` (drive them through
  `scripts/agent/record-chrome-control.sh`).
- PDF/UA corpus: render the tagged fixtures with the `pdf-ua` feature into
  `$SCRATCH_ROOT/ua-corpus/` (or any directory you pass to `verapdf-corpus.sh`),
  then validate with `scripts/agent/verapdf-corpus.sh`.
- turbo-dom-layout owns the shared fonts and layout fixtures — this repo does
  not copy them. `scripts/layout-crate-dir.mjs` (via `cargo metadata`) resolves
  the sibling crate's path for JS scripts, napi tests and benches.

## Adding a PDF/UA test

1. Write the failing test first. Commit it alone as `test: … (#N)`.
2. Use `assert_struct_tree` (crates/turbo-html2pdf-core/tests) to compare the
   rendered PDF's `StructTreeRoot` to the expected tree — never bytewise. Alt
   text, heading levels, list and table roles, reading order are all tree
   assertions.
3. Keep the HTML/CSS input minimal and page-sized for a predictable corpus
   entry.
4. Validate the rendered PDF: `scripts/agent/verapdf-corpus.sh` must stay green
   over the whole corpus, not just the new file.
5. Commit the fix as `fix(pdf-ua): … (#N)`.

## Gates that cannot be raised

- **Cyclomatic complexity ≤ 5** for any new or changed Rust function
  (`scripts/cc-check.js` / `tools/cc-check`). This is a hard limit — do not
  raise `--max`. Refactor instead.
- **Coverage: 100%** on the shipped crates, enforced by `tarpaulin.toml` in CI.
  Gated-only code lives in gated-only sibling modules (see `src/semantics_derive.rs`)
  so tarpaulin's cfg-mapping does not flag it.
- **No `--no-verify`.** If the pre-commit hook needs the napi build to exist,
  run `scripts/agent/build-napi.sh` first.

## Pinning turbo-dom pre-publish, and dropping the pin

The v0.8.4 candidate requires `turbo-dom` and `turbo-dom-layout` `=0.8.4`,
with a temporary git patch at `8266b30a366758a591f4d9952823576a468637a3`
until those versions are published. To update a reviewed pre-publish pin:

```sh
scripts/agent/pin-turbo-dom.sh <rev-or-sha>
```

Before tagging a release, both `release-guard.sh v*` and `release-guard.sh pyv*`
must pass — which means the pin has to be gone and `Cargo.lock` must be free of
git sources for turbo-dom:

```sh
scripts/agent/drop-turbo-dom-pin.sh
```

See CLAUDE.md "Pre-tag gate" and PUBLISHING.md "Pre-tag: drop the pre-publish
patch" for release guards and registry verification.

## Comparing against Chrome

For geometry that must match Chrome 156, use turbo-dom's parity harness:

```sh
TURBO_DOM_DIR=../turbo-dom scripts/agent/parity-compare.sh
```

The harness renders inputs with both Chrome 156 and turbo-html2pdf and diffs
the resulting artifacts. For one-off box-geometry probes used in tests, record
a fixture through `scripts/agent/record-chrome-control.sh` and assert against
it from a `*_chrome.rs` test.

## Pre-commit hook gotchas

The repo installs its hook via `git config core.hooksPath .githooks`. Known
sharp edges:

- **Rust hook runs cc-check at `CC_MAX=5`.** A refactor that nudges a function
  to 6 branches blocks the commit. Split the function or extract a helper;
  do not bypass with `--no-verify`.
- **Hook needs the napi addon on disk.** Some lanes `require()` the generated
  addon; a clean checkout will fail until you run `scripts/agent/build-napi.sh`
  once.
- **Formatter/linter use `pnpm exec` first, then `npx --no-install`.** If
  neither resolves (fresh container), run `npm ci` at the repo root before
  committing.
- **Staged `.rs` files are checked, not the working tree.** If a stray
  unformatted hunk is unstaged, the hook still passes — CI's `cargo fmt
  --check` will not.

## Shared machine hygiene

- One heavy `cargo` build at a time.
- Every script defaults `CARGO_TARGET_DIR` into the scratchpad. Delete it when
  you finish.
- `TZ=UTC LANG=C CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0` for
  reproducible timing and smaller artifacts (matches turbo-dom's AGENT-BRIEF).
- Chrome 156 and the rusty_v8 archive are cached under
  `~/.cache/turbo-consolidation/`. Never download fresh copies.
