# TODO: `pdf-a` feature — PDF/A-2b archival conformance (AC-11.2)

**Status:** deferred in Phase 15. Feature flag not yet declared.

## What it is (plain)
The **"keep-forever" PDF**. PDF/A bakes *everything* in (fonts, color definitions)
and forbids anything that could render differently later, so the file is
guaranteed to open and look the same in decades. Required by courts, governments,
and archives for long-term storage.

## Why deferred
Conformance needs (a) a vendored **sRGB ICC color profile** + `OutputIntent`,
(b) an **XMP metadata packet**, (c) no transparency, and (d) validation with
**veraPDF** — which is **not installed on PATH** here. Shipping it "PDF/A" without
the validator passing would be claiming compliance we can't prove.

## Where to start
- Source hook: `crates/turbo-pdf-core/src/emit/document.rs`
  (`TODO(phase15b, feature "pdf-a", AC-11.2)`).
- Font embedding is already done (Phase 9 subsets + embeds), which is most of the
  hard part — PDF/A mainly *forbids* leaving fonts out.

## What's needed
Behind `#[cfg(feature = "pdf-a")]`:
1. Vendor a sRGB ICC profile (e.g. `sRGB2014.icc`) into `assets/`; write it as an
   `OutputIntent` (`GTS_PDFA1`/2 subtype) in the catalog.
2. Emit an **XMP** packet (document metadata in RDF/XML) consistent with the info
   dict; declare the PDF/A conformance level (`pdfaid:part=2`, `conformance=B`).
3. Enforce the rules: device-RGB only (or via the OutputIntent), **no
   transparency / no `/ca` alpha** (the watermark fade must be disabled or use a
   knockout under this feature), all fonts embedded (already true).
4. Set the PDF version + `ID` + mark the document as tagged where required.
5. Add `pdf-a = [...]` to `[features]`.

## Acceptance
- Install **veraPDF** (`brew install verapdf` or the CLI) and gate a test on
  `which verapdf`: a `--features pdf-a` document **passes** `verapdf --flavour 2b`.
- `qpdf --check` clean; byte-deterministic; default build unaffected; per-feature
  test + clippy green; `--all-features` builds; tarpaulin 100% on default.

## Rough effort
Medium-high. The blockers are the **ICC asset + XMP packet + getting veraPDF to
pass** (lots of small conformance nits), not raw volume of code.
