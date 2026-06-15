# TODO: `pdf-ua` feature — tagged / accessible PDF (AC-11.1)

**Status:** deferred in Phase 15. Feature flag not yet declared. **Heaviest of the
four.**

## What it is (plain)
The **blind-friendly PDF** ("Universal Accessibility"). The file is *tagged* so a
screen reader knows "this is a heading", "this is a list", "this is a table cell",
"this image means <alt text>", and **in what order to read it**. Required by
accessibility law (e.g. ADA / Section 508 / EN 301 549).

## Why deferred
It needs a whole **structure tree** woven through the page painter: every piece of
content must be wrapped in marked-content (`BDC`/`EMC`) operators and linked into a
`StructTreeRoot`. That is a cross-cutting change to the emitter that wouldn't reach
100% coverage as a small slice.

## Where to start
- Source hook: `crates/turbo-pdf-core/src/emit/document.rs`
  (`TODO(phase15b, feature "pdf-ua", AC-11.1)`).
- Semantic info already exists upstream: the node tree (`node.rs`) and styled tree
  know the original HTML tags (`h1`, `ul`, `li`, `table`, `td`, `img alt=...`) —
  carry that role down to the fragment so the emitter can tag it.

## What's needed
Behind `#[cfg(feature = "pdf-ua")]`:
1. Carry a **structure role** (Document/H1../P/L/LI/Table/TR/TD/Figure) +
   `alt` text from the styled tree → fragment (`layout/fragment.rs`).
2. In `emit/page.rs`, wrap each fragment's painting in `/Tag <</MCID n>> BDC … EMC`
   marked-content, allocating MCIDs per page.
3. Build a **`StructTreeRoot`** + `StructElem` tree in the catalog, with
   `/ParentTree` mapping MCIDs back to struct elements; set reading order by
   document order.
4. `<img alt>` → `/Alt` on its `Figure` struct elem; mark artifacts (watermark,
   running header/footer decoration) as `/Artifact` so readers skip them.
5. Set `/MarkInfo <</Marked true>>`, `/Lang`, the `ViewerPreferences` DisplayDocTitle,
   and (usually pair with **pdf-a**) the document title in XMP.
6. Add `pdf-ua = [...]` to `[features]`.

## Acceptance
- Validate with **veraPDF (`--flavour ua1`)** and/or **PAC** if available
  (gate the test on `which verapdf`): a `--features pdf-ua` document passes.
- `qpdf --check` clean; byte-deterministic; default build unaffected; per-feature
  test + clippy green; `--all-features` builds; tarpaulin 100% on default.

## Rough effort
High. The marked-content + StructTreeRoot plumbing touches the painter broadly and
must hit 100% coverage. Best done **with `pdf-a`** (they share XMP/metadata work).
