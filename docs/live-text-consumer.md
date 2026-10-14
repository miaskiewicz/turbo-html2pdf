# Native text geometry consumer handoff

This additive seam is on draft PR20 (`codex/live-text-provenance`). The published
`turbo-html2pdf-core 0.3.5` provides box snapshots; these text APIs require a
separately reviewed and published dependency update. This document does not
change release versions or authorize a tag.

Build the actual borrowed source forest and `SourceIdentity` paths, including
all text nodes. Load the actual font programs into `FontRegistry`; caller aliases
cannot substitute browser-installed font metrics. Capture with
`snapshot_html_live_tree_with_text` or `snapshot_html_styled_tree_with_text` to use
the HTML font policy. The generic text APIs keep the PDF font/line policy.

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

`SourceRangeBox` exposes `fragment_index`, the actual local `rect`, and
`transformed_bounds`. Offsets address the original source's UTF-16 encoding,
including collapsed whitespace. Genuine combining/shaping clusters retain their
whole native cell; no internal glyph advance is interpolated. Collapsed gaps use
native pen positions and real line edges. Captures are bounded to 16 MiB/one
million Unicode scalars of original text and one million glyph records. Source
queries filter native runs before materializing cells for the chosen source.

Caret input uses document coordinates before the embedding DOM's scroll/frame
projection. The caller must select the visible text source through genuine hit
testing. Rotated, skewed or degenerate caret transforms return
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
crate records real Chrome155, the exact bundled Go font hash, final collector,
probe and runner source hashes, and all offsets/queries. Twelve controlled-font
graphs certify 144 Range client-rectangle lists and 89 caret hits, including
negative leading. The thirteenth emoji-fallback graph is retained but excluded
from numerical certification. Native regressions separately cover real fallback
run competition, combining-cluster edges, source ownership, hidden/missing data,
invalid inputs and unchanged PDF behavior.
