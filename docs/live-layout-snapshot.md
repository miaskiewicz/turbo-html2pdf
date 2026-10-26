# Live-tree layout snapshots

`turbo-html2pdf-core::layout_snapshot` adds an immutable native layout and source
provenance API alongside the existing PDF entry points.

> The layout engine, this API and its tests/Chrome control fixtures live in the
> [`turbo-dom-layout`](https://github.com/miaskiewicz/turbo-dom/tree/main/crates/turbo-dom-layout)
> crate (re-exported as `turbo_html2pdf_core::layout_snapshot`). Relative `tests/…`
> and `src/…` paths below are relative to that crate. It takes the caller's
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

Top-level HTML elements in the live forest retain document-root formatting
boundaries: body margins stay inside the root and first/last margins contribute
to its box. Nested HTML elements retain ordinary element flow. Root client sizes
are a CSSOM View viewport special case; the snapshot reports actual box metrics
and supplies its viewport separately.

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
do not trigger this admission error. Both entry points share this validation. Twenty authored offline contracts
cover identity after moves, path validation, partial maps, generated and anonymous
ownership, all text lines, transforms, hidden flow, definite heights, replaced
sizes, and unchanged legacy PDF filtering/caps. A further shared fixture compares
69 authored graphs against the independently installed Chrome
`155.0.8059.79` / V8 `15.5.35.23` numeric oracle, recorded on macOS ARM64. The
control compares 67 supported graphs numerically and verifies explicit rejection
of two transformed-ancestor fixed-position graphs. The geometry tolerance is 0.001 CSS pixels (affine Float32 rounding); font-free
control graphs avoid attributing font differences to box geometry.

This seam exposes the real native engine; it does not turn its CSS subset into
Blink. Remaining work before a complete browser geometry API includes:

- Broader collapsed-whitespace mapping, bidi/ligature
  caret positions and complete selection rectangles. The v0.3.8 optional
  text path below retains actual original byte/UTF-16 cluster provenance; the
  published box-only path does not invent this information.
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

The fixture is turbo-dom's `crates/turbo-dom-layout/tests/fixtures/layout-snapshot/cases.json`
(the control script finds the turbo-dom-layout checkout through `cargo metadata`).
It supplies explicit external IDs and authored tree/style definitions; the Chrome
collector retains actual node references rather than injecting ID attributes.

```sh
node scripts/layout-snapshot-chrome-control.mjs \
  --runner /absolute/turbo-surf/harness/detection/chrome-cdp-trace.mjs \
  --out /absolute/turbo-dom/crates/turbo-dom-layout/tests/fixtures/layout-snapshot/chrome155.json
cargo test -p turbo-dom-layout --test layout_snapshot   # in the turbo-dom workspace
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
a visible empty/flattened inline in the box-only path, so embedders must not invent a zero rectangle
from fragment absence.


## Optional native text-source provenance (v0.3.8)

`snapshot_live_tree_with_text` and `snapshot_styled_tree_with_text` return the same
immutable snapshot with `text_runs()`. Each record identifies an actual native
text fragment, the caller's source ID/ancestors and its positioned glyphs. Every
glyph carries the real shaper byte cluster converted to original input UTF-16
ranges, pen position and advance. Fragment origin and composed transform remain
in `fragments()[run.fragment_index]`. These are actual shaping records, not equally
spaced positions derived from a line rectangle.

`layout::inline::layout_paragraph_in_with_sources` exposes the native paragraph
and parallel per-line/per-run/per-glyph source data without changing the existing
public paragraph/glyph structs. The text-enabled route shapes clusters in the
primary face before choosing a fallback: normalization can produce a valid glyph
for a base-plus-mark sequence even when that individual mark has no cmap entry.
The existing PDF/box-only itemization and entry points retain their behavior.

The separate `snapshot_html_live_tree_with_text`,
`snapshot_html_styled_tree_with_text` and
`layout_html_paragraph_in_with_sources` entry points select continuous HTML
font metrics. Ascent/descent are rounded independently from the loaded font
tables; macOS legacy-family adjustments follow the pinned Blink155 algorithm,
using the font program's real family rather than a caller alias. The generic
text entry points preserve their fractional/PDF metrics. `SourceFontBox` retains
the actual run baseline and metrics. `text_cluster_boxes()` combines genuine
shaper advances with these font boxes and the fragment's composed transform.
Cluster cells share immutable native ancestry rather than copying a deep
ancestor vector for every glyph. Each result keeps source ownership and UTF-16
bounds; it is not an interpolated
caret position or a glyph ink rectangle.

`text_sources()` preserves each mapped text node's actual native UTF-8 input
before whitespace collapse, together with its stable source ID. This provides
original whitespace/UTF-16 ownership for subsequent Range algorithms without
reading a guest getter or reconstructing text from rendered glyphs. Cluster
records also retain the native shaper's selected direction; this does not imply
complete CSS bidi itemization.

The native capture refuses more than 16 MiB or one million Unicode scalars of
input text before source copying/layout capture, layout-ID overflow or more than
one million captured
glyph records. `TextSnapshotError` wraps the existing viewport/source validation
errors and reports that additional capacity bound. Original box-only snapshots
retain their error type and allocate no glyph provenance. Text metadata holds
native values and font/source IDs, with no script references or guest attributes.
Shared loaded-font memoization is separately bounded to 128 entries and 8 MiB
of logical run-text/glyph payload per face; oversized actual shaping results are
returned without persistent caching. Eviction leaves real shaping output unchanged.

The authored `scripts/text-source-chrome-control.mjs` serves the exact bundled
Go-Regular.ttf and captures Range rectangles and caret queries in independent
Chrome. Its saved font hash is
`197d9f3703b4c00af609178876a8d73e396f64fe438b2c871778566632374be3`.
Twelve controlled-face graphs check exact native cluster x/y/width/height,
including combining text, collapsed leading whitespace, wrapping, transforms
and four font sizes. NBSP/em-space retain their actual shaped advances, and
NBSP remains unbroken when its word overflows. A thirteenth graph preserves the uncontrolled installed emoji
fallback result; its
numeric metrics are not certified. The capture requires zero errors and records
localhost-only extension flags. Native tests separately verify astral UTF-16
ranges, source ownership and repeated wrapped runs.

`text_range_boxes(source, start, end)` selects one original text node using the
actual placed cells and pen gaps. It returns the native fragment index, local
rectangle and transformed bounds for each selected line. Partial combining
clusters retain their whole native cell; selecting collapsed spaces uses the
measured gap between adjacent placed cells or a genuine zero-width line edge.
`text_caret_offset_from_point(source, point)` chooses a real cluster edge after
inverting a finite affine fragment transform. The embedding caller must first
identify the visible text source through its actual DOM hit-test policy. This
method does not choose a DOM node, invent internal ligature offsets or perform
browser viewport clipping. Rotation and skew are supported for finite invertible
affine transforms; degenerate transforms remain
explicitly unsupported. Missing/unplaced sources and invalid input have distinct
`TextGeometryError` variants.

The same twelve controlled-font graphs now compare every valid UTF-16 start/end
pair's complete client-rectangle list and every recorded caret hit. This includes
partial combining-cluster selection, collapsed spaces, wrap edges, overlapping
negative-leading lines and translated/scaled fragments. The source-scoped producer
filters actual native runs before producing cells, avoiding unrelated glyph
geometry allocations. On overlapping font boxes the earlier real line wins;
between equally distant boxes the following line wins. Separate fallback runs
on a chosen native line compete by their genuine edge distances. The public HTML
text policy permits negative leading; generic/PDF policy remains unchanged.
Offline contracts also cover invalid/missing/hidden sources,
nonfinite coordinates and unsupported caret transforms. The original emoji
fallback graph remains recorded but is explicitly outside numerical certification.

Full cross-node DOM Range assembly, hit-test selection, bidi/CJK itemization,
broader inline formatting, CSS whitespace modes/Unicode line breaking and complete
font-fallback behavior remain unfinished. The controlled-font comparisons do not
certify installed fallback fonts, platform-specific hinted metrics or complete
CSSOM View behavior.

## Native inline boundaries and border fragments (v0.3.8)

The native text-enabled HTML entry point preserves ordinary inline start/end
boundaries before word breaking. Horizontal margins, borders and padding advance
the real line pen. Per-line fragments retain the inline's own physical font box
and first/last decoration slices; a transparent ancestor references actual child
regions without painting the child's decorations a second time. Empty contexts
retain placed zero rectangles and explicit collapsed-text provenance. Ordinary
inline CSS transforms are ignored, as in Chrome; relative offsets move the own
and descendant visual fragments without changing normal-flow advances.

`UsedBoxMetrics.inline_fragment` identifies ordinary non-replaced inline geometry;
an inline image or control retains its own client metrics. Native per-inline
leading is quantized in physical LayoutUnits, parent struts participate in the
line ascent/descent union, and nested super/sub shifts use the parent's physical
font size. Shifted atomic children retain their own vertical-align policy.
The existing generic/PDF boundary flattening and leading policies are unchanged.

`tests/native_inline_borders.rs` compares 43 original-source graphs against fresh
Chrome155 controls, including actual container heights and every border fragment.
Every host exercises the same source graphs with source-role, ordinary/replaced
metric and relative-offset invariants. Exact installed-font numerical comparison
is certified on macOS; Linux glyph geometry remains unmeasured against this oracle.
The saved `tests/fixtures/inline-borders/probe.js` is the independently executed
probe; its SHA-256 is checked against the recorded control. Regenerate the browser
control with turbo-surf's reusable `harness/detection/authored-dom-chrome.mjs`
collector, using that probe and the `chrome155.json` output path.

Ordinary static block descendants split inline flow into before/block/after
segments. Boundary-only continuations keep their zero client rectangles without
adding line height or stopping margin collapse. Interior horizontal decorations
are sliced; the lifted block uses its own width, margins and style. Its real
fragment is additionally associated with the original inline source, while the
fragment's own source ID and ancestor chain remain unchanged. The generic/PDF
flattening path remains intact. Auto native inline-block widths use the same
physical LayoutUnit admission as explicit lengths.

`tests/native_block_inline.rs` retains six independently measured Chrome
155.0.8059.85 graphs and the reusable `fixtures/block-inline-probe.js`, including
nested links, empty continuations, multiple blocks, sliced borders and the
following button. The fixture constructs real live document roots and supplies
the caller's measured browser button styles; PDF parsing intentionally returns
body flow and the generic PDF user-agent stylesheet is a separate policy.

Native admission still rejects positioned/floating split contexts, ordinary
inline top/bottom/middle alignment, absolute/fixed/sticky positioning, RTL and vertical writing modes with
specific `SnapshotError` variants. Atomic and inline-block top/bottom alignment
remain available. These limits describe the supported native subset, rather than
certifying the full CSS inline formatting model or complete browser layout.

For macOS native HTML, generic serif selects physical Times before Times New
Roman; the ordinary PDF system-font loader retains its prior preference. Physical
legacy ascent policy reads decoded platform family records when a font has no
Unicode family name, so an author alias cannot enable or suppress that policy.
The source-pinned `tests/fixtures/native-serif/probe.js` and Chrome155 control
cover fifteen font/size combinations: serif, Times, Times New Roman, Helvetica
and Courier at 13, 16 and 24 CSS pixels. Tests compare real physical font metrics,
text advances, source Range rectangles and normal container line heights; a
separate actual-font identity check preserves the existing PDF selection.

## Original-scope matched declarations and unboxed elements (v0.3.8)

An embedder with its own DOM and shadow trees can call
`style::resolve_matched_style(parent, declarations)` after matching selectors in
their actual scopes. Each `MatchedDeclaration` carries its CSS origin, shadow
depth, inline status, specificity and source order. Normal declarations prefer
outer encapsulation contexts; important declarations prefer inner contexts.
Origin precedence applies first, with the usual important-origin reversal.
The caller supplies UA declarations and inline declarations explicitly;
`presentational_declarations` exposes the ordinary engine's attribute hints.
`user_agent_css` and `user_agent_stylesheet` expose the same UA rules for an
embedder's original-scope matcher.
Custom properties, inherited values, quad shorthands and font sizes share the
existing style resolution. This API does not match selectors or provide cascade
layers, animations or complete CSS parsing on the caller's behalf.

`display:contents` now suppresses only the element's box and promotes its actual
children into the parent's formatting context. The source tree retains the
unboxed ancestor and its inherited text style, without applying its padding,
margin or transform to promoted boxes. Its own fragment list remains empty;
descendant provenance remains available. Replaced HTML elements and controls
listed in CSS Display's unusual-element rules suppress their subtree instead.
Flex/grid text becomes genuine anonymous items, grouped across unboxed and
display-none boundaries while retaining original text identities and styles.
Whitespace-only items are omitted; NBSP remains real content. Native HTML
intrinsic sizing and flex/grid placement retain fractional advances and the same
whitespace policy as final glyph layout. Legacy PDF rounding remains separate.
Full anonymous table row/cell synthesis is still unsupported.

The authored fourteen-case control is independently captured with the reusable
collector's `--fixture` argument; its source hashes and actual Chrome/V8 versions
are retained in `tests/fixtures/layout-snapshot/contents-chrome155.json` and
`contents-text-chrome155.json`; the latter loads the same bundled Go font bytes
in both engines and records their hash.

## Explicit outer SVG viewports (v0.3.8)

`snapshot_html_live_tree_with_svg_viewports` and its already-styled counterpart
accept the complete original forest, source identities, explicit outer-SVG
`SourceId` declarations and a physical `layout_scale`. The embedding caller
supplies identities from its actual SVG namespace. A supplied declaration must
identify an SVG element; duplicate/nested declarations and invalid scale fail
before layout. Qualified SVG names retain their original source identity.

This route builds an actual atomic replaced viewport with float intrinsic size
and native CSS width/height, constraints, padding/borders and transforms. It reads
case-sensitive native attributes, resolves percentage geometry against the real
containing block, and distinguishes CSS `auto` from intrinsic attribute sizes.
Intrinsic width/height ratio precedes a viewBox-only ratio. Missing natural axes
use the real default 300×150 sizing; viewBox-only sizing uses the containing
width. There is no fabricated raster image, injected attribute or rewritten tag.

Every original source/path remains in the snapshot. SVG shape, nested SVG and
foreignObject descendants have `unsupported_svg_source_geometry(id) == true`;
they have no manufactured fragment or zero rectangle. The supported outer SVG
has its genuine border fragment and used content metrics. Display-none exclusion
remains independently recorded through actual source ancestry. The normal
box-only/PDF and generic text APIs retain their previous policy.

Before allocating viewport fragments, native replaced sizing converts used
content dimensions through the pinned Blink155 six-bit LayoutUnit model at the
caller's measured physical layout scale. That scale is separate from JavaScript
`devicePixelRatio`; the oracle measures it with independent ordinary HTML input
sizes. The current profile's 32.51→32.5078125 and 32.504→32.5 observations uniquely
match scale 2 among the tested 1/2/4/8 candidates. An explicit DPR 1 comparison
retains the same observations as DPR 2. No returned rectangle or affine quad is
postprocessed to match a hash. Source attribution/license is in
`src/layout/svg-layout-provenance.json` and the packaged notices.

`scripts/svg-viewport-chrome-control.mjs` retains actual Chrome/V8, source hashes,
DPR, scale-calibration inputs/results and full parent/viewport/shape measurements.
Forty-nine independent graphs verify the complete supported parent and viewport
rectangles/client dimensions, including fractional sizes, ratio precedence,
case sensitivity, invalid CSS versus computed-variable invalidation, malformed/
negative/mixed-calc attributes, strict signed/exponent/decimal viewBox grammar,
CSS overrides/auto, percentages, padding/borders, transforms, flex controls,
hidden flow and margins. Shape measurements remain recorded and explicitly
outside this viewport producer. Native contracts also verify declarations,
nested-source admission, preserved identities and bounded/configurable scale.

The native engine's remaining general CSS subset and float layout policy still
apply outside this scoped used-size conversion. This does not supply SVG shape
layout/painting, foreignObject layout, nested viewport transforms, full CSS zoom,
all flex/grid/table sizing or browser hit-test stacking. Embedders must retain
those limits and use real loaded resources/fonts; this seam does not certify a
complete mixed SVG/browser implementation.
