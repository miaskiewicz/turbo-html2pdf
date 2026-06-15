# TODO: `xref` feature — internal links & cross-references (AC-3.25)

**Status:** deferred in Phase 15. Feature flag not yet declared.

## What it is (plain)
Clickable navigation *inside* the PDF. `<t:anchor name="ch2">` marks a spot;
`<a href="#ch2">see Chapter 2</a>` becomes a clickable link that jumps there.
Also the foundation for `page()`/`ref()`-style "see page N" cross-references.

## Why deferred
Positioned fragments do **not** carry the anchor `name` / link `href` through
layout → pagination, so at emit time the engine doesn't know *where on which page*
an anchor landed, nor which rectangle a link should cover.

## Where to start
- Source hook: `crates/turbo-pdf-core/src/emit/document.rs` (top-of-file
  `TODO(phase15b, feature "xref", AC-3.25)`).
- `crates/turbo-pdf-core/src/node.rs` — `TKind::Anchor` already exists.

## What's needed
1. Thread an optional `anchor: Option<String>` (from `<t:anchor name>`) and
   `link_href: Option<String>` (from `<a href>`) onto the box/fragment in
   `boxgen.rs` → `layout/fragment.rs` so it survives layout + pagination.
2. Two-pass emit behind `#[cfg(feature = "xref")]`:
   - Pass 1: walk all pages, record each anchor's page index + position → a
     `Dests` name tree (PDF named destinations).
   - Pass 2: for each `<a href="#name">` fragment, write a Link annotation
     (rectangle = the fragment's box) with a `GoTo` action to that dest.
3. Add `xref = []` to `[features]` in `crates/turbo-pdf-core/Cargo.toml`.

## Acceptance
- `--features xref`: a doc with an anchor + a link to it produces a Link
  annotation whose GoTo target is the anchor's page; `qpdf --check` clean.
- Default build byte-identical; `cargo tarpaulin` stays 100% (gated code excluded
  like `perf`/`endnotes`); per-feature test + clippy green; `--all-features` builds.

## Rough effort
Medium. The plumbing (carry name/href through layout) is the bulk; the PDF
annotation/dest writing is small (`pdf-writer` supports both).
