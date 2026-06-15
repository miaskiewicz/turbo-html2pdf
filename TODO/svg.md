# TODO: `svg` feature — vector image support

**Status:** deferred in Phase 15. Feature flag not yet declared.

## What it is (plain)
Let `<img src>` / `background-image` point at **SVG** (vector graphics: logos,
charts, icons that stay crisp at any zoom). Today only raster PNG/JPEG works
(Phase 9b); those pixelate when scaled up. SVG stays sharp.

## Why deferred
`resvg`/`usvg` (the rasterizer) pulls a **large transitive dependency tree** that
needs an MSRV-1.82 + determinism audit before adding it — shipping it unpinned
would risk the build/coverage/determinism gates.

## Where to start
- Source hook: `crates/turbo-pdf-core/src/image.rs` (top-of-file
  `TODO(phase15b, feature "svg")`). The decode path is the single integration
  point — current raster decode lives here.
- The Phase 9b image XObject path (`src/emit/image.rs` `ImageStore`) is reused
  unchanged: SVG just produces an RGBA buffer that feeds the *existing* pipeline.

## What's needed
1. Add `resvg`/`usvg` (+ `tiny-skia`) as `optional = true` deps tied to
   `svg = ["dep:resvg", ...]`. Pin exact versions; audit the tree for MSRV 1.82
   and for any non-determinism (fonts, time, threads).
2. Behind `#[cfg(feature = "svg")]`: detect `image/svg+xml` bytes, rasterize via
   `resvg` at a chosen target DPI into RGBA, then hand off to the existing
   `ImageStore` (RGB body + alpha → SMask) — **no new XObject code**.
3. Determinism: pin the resvg/usvg font handling (no system font lookup) so
   output is byte-identical for identical inputs (matches the engine's rule).
4. Add `svg = [...]` to `[features]`.

## Acceptance
- `--features svg`: an `<img>` referencing SVG bytes embeds a rasterized XObject;
  byte-deterministic; `qpdf --check` clean.
- Default build unaffected (resvg not pulled in); per-feature test + clippy green;
  `--all-features` builds; tarpaulin 100% on default.

## Rough effort
Medium, mostly **dependency audit + determinism pinning**. The code is small
(decode → reuse 9b). Risk is the dep tree, not the logic.
