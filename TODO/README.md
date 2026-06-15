# TODO — deferred feature gaps

Four optional, `#[cfg(feature)]` capability gates were scoped in Phase 15 but
**deferred** (the two that landed: `endnotes`, `print-color`). Each gets its own
doc below: what it is, why it was deferred, the exact source hook to start from,
what's needed, and acceptance criteria.

| Gap | One-liner | Effort | Blocker |
|---|---|---|---|
| [`xref`](xref.md) | clickable internal links / cross-references | Medium | carry anchor/href through layout→pagination |
| [`svg`](svg.md) | vector (SVG) image support | Medium | audit + pin the `resvg` dep tree |
| [`pdf-a`](pdf-a.md) | "keep-forever" archival PDF/A-2b | Med-High | ICC asset + XMP + veraPDF (not installed) |
| [`pdf-ua`](pdf-ua.md) | accessible/tagged PDF for screen readers | High | StructTree + marked-content through the painter |

All four are off-by-default add-ons; the default build is unaffected. `pdf-a` and
`pdf-ua` share XMP/metadata work and are best done together.
