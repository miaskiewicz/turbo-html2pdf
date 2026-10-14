# Live-tree layout snapshots

`turbo-html2pdf-core::layout_snapshot` adds an immutable native layout and source
provenance API alongside the existing PDF entry points. It takes the caller's
actual borrowed `Node` forest (or an already styled `StyledNode` forest), builds
one box tree, and retains every native galley fragment. There is no HTML
serialization/reparse, lazy-image rewrite, subtree removal, or injected identity
attribute.

`SourceId(u64)` comes from the caller's node identity registry. Each
`SourceIdentity` supplies that ID and the node's current child-index path in this
snapshot; paths count **all** nodes, including text. A moved node keeps its ID
and gets a new path. IDs and paths must be unique and paths must identify actual
nodes. Partial identity maps are allowed. The engine's generated `NodeId` and
fragment index are scoped to this layout and are not stable DOM identities.
The internal address lookup exists only while building the same borrowed styled
tree; addresses never escape as source IDs.

```rust
use turbo_html2pdf_core::layout_snapshot::{
    snapshot_live_tree, SnapshotInputs, SourceId, SourceIdentity,
};
use turbo_html2pdf_core::{
    build_cascade_with_width, parse_html, set_media_viewport_height,
    Diagnostics, FontRegistry, ImageCtx, NoImages,
};

let nodes = parse_html("<div style='height:40px'>hello</div>");
let identities = [
    SourceIdentity { id: SourceId(42), path: vec![0] },
    SourceIdentity { id: SourceId(43), path: vec![0, 0] },
];
let fonts = FontRegistry::new();
let images = ImageCtx { resolver: &NoImages, body_height: None };
set_media_viewport_height(600.0);
let cascade = build_cascade_with_width("", "", Default::default(), 800.0);
let inputs = SnapshotInputs { width: 800.0, height: 600.0, fonts: &fonts, images: &images };
let snapshot = snapshot_live_tree(&nodes, &identities, &cascade, &inputs,
    &mut Diagnostics::default()).unwrap();
assert!(!snapshot.source(SourceId(42)).unwrap().fragment_indices.is_empty());
```

For a live DOM integration, construct `Node` directly through its public fields;
include the real root elements and existing child order. The HTML parser example
above is only a standalone convenience. Supply live styles (including UA rules,
CSSOM changes, presentational state, form state and pseudo-class state) through
the cascade or already styled entry point. Resolve media conditions using the
same viewport before building the cascade. The snapshot sets that viewport for
layout unit resolution, but cannot retroactively reevaluate a previously built
cascade. Fonts and loaded image bytes come from the caller's public registry and
resolver; the snapshot does not execute scripts or fetch resources.

## Geometry and fragment ownership

Live absolute descendants use the actual padding-box containing block, including
percentage sizes, insets and measured auto-height parents. Plain fixed descendants
resolve against the viewport. Fixed descendants under transformed ancestors need
a further containing-block implementation and are explicitly rejected.

The live path drops `display:none` boxes, preserves `visibility:hidden`,
`opacity:0` and clipped boxes in layout, and resolves definite percentage heights
against the continuous-media viewport/ancestor. It sizes resolved raster images
using intrinsic dimensions, CSS dimensions, box sizing and min/max constraints,
without the PDF path's 100%-width / 60%-page-height caps. An image has a real border
box and a separate content image fragment with the same source identity.

`LayoutSnapshot` owns an immutable galley plus an index of **all** fragments in
native subtree paint order. Each indexed fragment carries its native layout ID,
parent index, optional source ID, source ancestors, category, pre-transform
rectangle, composed affine matrix, transformed quadrilateral and bounding box.
Border fragments also carry `UsedBoxMetrics`, recorded from the actual resolved
style during layout: border, padding, margin, content dimensions, display and
position. Local metrics follow the final fragment translation instead of keeping
stale absolute origins after relative/flex placement. `content_rect` and
`padding_rect` expose those actual boxes without claiming scrollbar extents.
Anonymous live boxes inherit text properties without repeating parent frames or
transforms. The Chrome controls independently compare client border/inner sizes
and untransformed offset sizes for the supported font-free boxes.

Ancestor and child transforms are composed around each box's actual absolute
transform origin. Image/background/mask/text fragments are retained, so multiple
fragment records for one source are expected. Anonymous wrappers and generated
input label text do not acquire fictional source identities; their layout parent
still contributes source ancestry.

`SnapshotSource.fragment_indices` selects that node's actual fragments;
`descendant_fragment_indices` includes provenance through mapped ancestors,
including inline elements whose text was flattened. A mapped hidden source or
flattened inline can legitimately have no own border-box fragment. Text source
IDs reach every produced text-line fragment, and `galley()` retains actual
positioned glyphs and font faces. `TextLineContainer` rectangles describe the
native line container: they are **not** DOM Range rectangles or tight glyph
bounds. No source identity is reconstructed by assuming allocation order from a
second box tree.

## Validation and current boundaries

The public API rejects duplicate source IDs, duplicate paths, missing paths,
and nonfinite/nonpositive viewport dimensions. `UnsupportedFixedContainingBlock`
rejects a fixed descendant of a valid transformed ancestor before returning any
snapshot, through unmapped/static ancestors and identity transforms alike.
Transform siblings, `none`, invalid transform declarations and excluded subtrees
do not trigger this admission error. Both entry points share this validation. Eighteen authored offline contracts
cover identity after moves, path validation, partial maps, generated and anonymous
ownership, all text lines, transforms, hidden flow, definite heights, replaced
sizes, and unchanged legacy PDF filtering/caps. A further shared fixture compares
52 authored graphs against the independently installed Chrome
`155.0.8059.79` / V8 `15.5.35.23` numeric oracle, recorded on macOS ARM64. The
control compares 50 supported graphs numerically and verifies explicit rejection
of two transformed-ancestor fixed-position graphs. The geometry tolerance is 0.001 CSS pixels (affine Float32 rounding); font-free
control graphs avoid attributing font differences to box geometry.

This seam exposes the real native engine; it does not turn its CSS subset into
Blink. Remaining work before a complete browser geometry API includes:

- Fragmented inline border boxes and original text byte/UTF-16 offsets, whitespace
  mapping, bidi/ligature clusters, caret and selection rectangles. Existing
  positioned glyphs have no retained original-offset mapping; none is invented.
- Full overflow/clip/scrollbar regions, scroll offsets, sticky behavior, hit
  testing, transforms that establish fixed containing blocks, frame coordinate
  propagation, and browser stacking contexts beyond the engine's paint subset.
- Remaining CSS layout and intrinsic sizing cases, including full control UA
  layout, missing/unloaded replaced resources, flex/grid percentage-height and
  stretch details, table/border collapse, modern writing modes and shadow trees.
- Browser font loading/fallback epochs and raster glyph behavior. Bundled PDF
  fonts are not assumed to be the browser's fonts.

The snapshot should be invalidated by the caller's tree/document version,
stylesheet/CSSOM state, viewport/frame geometry, image/font readiness and actual
form/pseudo state. Stable node IDs alone are not a sufficient layout cache key.
No fake caret, client/scroll dimensions, clipping rectangles or browser identity
values are returned to fill these gaps.

## Repeating the independent Chrome control

The fixture is `crates/turbo-html2pdf-core/tests/fixtures/layout-snapshot/cases.json`.
It supplies explicit external IDs and authored tree/style definitions; the Chrome
collector retains actual node references rather than injecting ID attributes.

```sh
node scripts/layout-snapshot-chrome-control.mjs \
  --runner /absolute/turbo-surf/harness/detection/chrome-cdp-trace.mjs \
  --out /absolute/turbo-html2pdf/crates/turbo-html2pdf-core/tests/fixtures/layout-snapshot/chrome155.json
cargo test -p turbo-html2pdf-core --test layout_snapshot
```

Chrome is independent development tooling, never a production dependency. The
runner serves a local authored page in a fresh Chrome profile and records actual
browser/V8 versions, viewport, bounding rectangles and all client rectangles.
For authored loopback controls only, the optional `disableExtensions:true`
setting adds `--disable-extensions` and records its effective flag. Normal browser
captures keep their default extension behavior. The control still rejects any
page/probe/target capture error; failed captures are retained for diagnosis. The
standalone PDF, N-API, WASM and Python APIs retain their existing behavior.
This source checkpoint does not release a new crate version; consumers need a
separately reviewed additive dependency/version checkpoint before using it.

Source records expose actual `excluded_from_layout` provenance, including unmapped
`display:none` ancestors. Zero fragments alone cannot distinguish exclusion from
a visible empty/flattened inline, so embedders must not invent a zero rectangle
from fragment absence.
