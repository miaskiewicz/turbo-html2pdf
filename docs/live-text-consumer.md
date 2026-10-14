# Native text geometry consumer handoff

The text/SVG provenance APIs ship in `turbo-html2pdf-core 0.3.6` from merged
PR20. The physical-scale font/leading and fractional Range corrections are
prepared separately as 0.3.7. No release tag is authorized by this document.

Build the actual borrowed source forest and `SourceIdentity` paths, including
all text nodes. Load the actual font programs into `FontRegistry`; caller aliases
cannot substitute browser-installed font metrics. Capture with
`snapshot_html_live_tree_with_text` or `snapshot_html_styled_tree_with_text` to use
the HTML font policy. The generic text APIs keep the PDF font/line policy.

Native used transforms are captured during actual box placement with double
precision CSS coefficients/composition until the final float points.
`native_world_transform(fragment_index)` exposes that matrix; the existing
`SnapshotFragment.world_transform` and public `Fragment`/PDF matrices retain their
original float type. Inner image paint fragments inherit the box matrix once.

The new queries are methods on that immutable `LayoutSnapshot`:

```rust
use turbo_html2pdf_core::layout_snapshot::{Point, SourceId};

let text_id = SourceId(100); // The embedding DOM's own stable native identity.
let rects = snapshot.text_range_boxes(text_id, 0, 2)?;
let offset = snapshot.text_caret_offset_from_point(
    text_id, Point { x: 12.0, y: 20.0 },
)?;
let cells = snapshot.text_cluster_boxes_for_source(text_id);
```

For the explicit physical-scale HTML/SVG capture path, ascent/descent are
rounded at physical font size and half-leading is floored before line placement.
Source Range edges then encompass nonempty offsets in native LayoutUnits and
floor equal offsets before affine transforms. Raw cluster records retain their
real pen advances. Generic/PDF source APIs keep the existing float policy.

`SourceRangeBox` exposes `fragment_index`, the actual local `rect`, and
`transformed_bounds`. Offsets address the original source's UTF-16 encoding,
including collapsed whitespace. Genuine combining/shaping clusters retain their
whole native cell; no internal glyph advance is interpolated. Collapsed gaps use
native pen positions and real line edges. Captures are bounded to 16 MiB/one
million Unicode scalars of original text and one million glyph records. Source
queries filter native runs before materializing cells for the chosen source.

Caret input uses document coordinates before the embedding DOM's scroll/frame
projection. The caller must select the visible text source through genuine hit
testing. Finite invertible rotation/skew/scale transforms use real native affine
inversion for caret queries. Degenerate or unrepresentable transforms return
`UnsupportedCaretTransform`. Missing sources, unplaced/hidden sources, invalid
UTF-16 bounds and nonfinite points remain distinct errors. Range transforms can
produce actual affine bounds even when that caret query is unsupported.

The embedding still owns cross-node Range assembly, DOMRect allocation and
prototype descriptors, frame/scroll/clip projection, document hit testing and
layout invalidation after tree/CSSOM/font/image/viewport changes. Native UTF-8
source inputs preserve Unicode scalar UTF-16 offsets; they do not supply a
lossless lone-surrogate DOM storage representation. Full bidi itemization,
internal ligature carets, CSS whitespace modes, fragmented inline borders and
uncontrolled fallback-font geometry remain open. Atomic-inline line metrics
retain their existing policy; the new negative-leading control covers pure text.

The independent fixture `tests/fixtures/text-source-chrome155.json` under the core
crate records real Chrome 155, the exact bundled Go font hash, final collector,
probe and runner source hashes, and all offsets/queries. Sixteen controlled-font
graphs certify 168 exact Range client-rectangle lists and 107 caret hits, including
negative leading, 13/15px physical font metrics, rotation and skew. The emoji-fallback graph is
retained but excluded from numerical certification. Native regressions separately cover real fallback
run competition, combining-cluster edges, source ownership, hidden/missing data,
invalid inputs and unchanged PDF behavior.


For mixed HTML/SVG forests, the new
`snapshot_html_live_tree_with_svg_viewports` / already-styled variant takes
`viewports: &[SourceId]` followed by `layout_scale: f32`, before the ordinary
cascade/inputs/diagnostics arguments. Declare only actual outer SVG namespace
nodes. The current independently measured profile uses scale 2; use the recorded
calibration rather than equating this value to JS DPR. Check
`unsupported_svg_source_geometry(source)` before answering any SVG descendant
geometry request. Full source paths/IDs are retained; shape/nested/foreignObject
geometry is not synthesized. Native viewport output can contribute to a genuine
HTML parent's layout while these per-source limits remain explicit. See the
SVG section in `docs/live-layout-snapshot.md` and its separate reproducible oracle.

The optional `--adversarial` collector records 31 graphs. The 30 controlled
Go-font graphs match all 391 Range lists, 273 caret hits and 1141 fractional
caret sweeps. Real used-value placement quantizes typed lengths before layout;
first-line indentation survives float clearance, including negative and percentage
indentation. Reused layout trees reset font/physical/source policies and intrinsic
caches against fresh controls. Text snapshots reject nonfinite metadata.

The native `snapshot_html_styled_tree_with_native_text` / live-tree variant accepts
`NativeTextOptions` with original `SourceTextUnits`, actual SVG identities and the
measured physical scale. Existing Node/InlineRun/Fragment variants and scalar/PDF
entry points remain unchanged. Unknown/duplicate identities, projection mismatches
and more than two million supplied UTF-16 units fail before layout. Obtain actual
retained units with `text_original_units`; scalar UTF-8 cluster byte indices still
refer to the validated scalar projection. Raw-unit shaping cache keys distinguish
lones from literal replacement characters.

On macOS the native source route consults the safe CoreText cascade and real font
coverage/metrics, including MacRoman-only installed face names. Original invalid
units remain separate through script segmentation and font selection; the selected
font shapes the UTF-16 replacement glyph. A fresh independent Chrome control retains
all 26 mixed astral/lone Range differences and twelve font/size controls. Source-line
Range boxes span adjacent fallback runs; original glyph advances remain untouched.

`native_hit_transform` separately captures paint offsets: transform paint scopes
round origins and discard residuals only on axes that cannot propagate them. Layout
and Range geometry retain `native_world_transform`. This supports measured ordinary
invertible div/img rotations, skews, nested transforms and one-axis scaling. Paint
isolation from containment, masks, opacity/compositing and animations is outside the
certified hit scope. The retained old uncontrolled astral-fallback graph remains
outside the Go numerical contract; it is not a claim of full Unicode/bidi/font parity.
