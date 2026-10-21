//! Advance/font boxes from real glyph clusters, not line-container rectangles.
use super::{geometry, LayoutSnapshot, Point, Rect, SnapshotTextRun, SourceId};
use std::collections::BTreeMap;
use std::sync::Arc;

/// One actual shaper cluster's native advance/font box and composed transform.
/// It may span several UTF-16 units; this is not an interpolated caret position.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceClusterBox {
    pub fragment_index: usize,
    pub source_id: Option<SourceId>,
    pub source_ancestors: Arc<[SourceId]>,
    pub right_to_left: bool,
    /// Actual CSS-collapse provenance; a rendered first space can still advance.
    pub collapsed_whitespace: bool,
    pub utf16_start: usize,
    pub utf16_end: usize,
    pub rect: Rect,
    pub quad: [Point; 4],
    pub transformed_bounds: Rect,
}

pub(crate) fn cluster_boxes(snapshot: &LayoutSnapshot) -> Vec<SourceClusterBox> {
    collect_boxes(snapshot, snapshot.text_runs().iter())
}

pub(crate) fn source_cluster_boxes(
    snapshot: &LayoutSnapshot,
    source: SourceId,
) -> Vec<SourceClusterBox> {
    collect_boxes(
        snapshot,
        snapshot
            .text_runs()
            .iter()
            .filter(|run| run.source_id == Some(source)),
    )
}

fn collect_boxes<'a>(
    snapshot: &LayoutSnapshot,
    runs: impl Iterator<Item = &'a SnapshotTextRun>,
) -> Vec<SourceClusterBox> {
    let mut boxes = Vec::new();
    for run in runs {
        let fragment = &snapshot.fragments()[run.fragment_index];
        let ancestry: Arc<[SourceId]> = run.source_ancestors.clone().into();
        let mut groups = BTreeMap::<(usize, usize), (f32, f32, bool, bool)>::new();
        for glyph in &run.glyphs {
            let source = glyph.source;
            let begin = source.pen_x.min(source.pen_x + source.advance);
            let end = source.pen_x.max(source.pen_x + source.advance);
            let edges = groups
                .entry((source.utf16_start, source.utf16_end))
                .or_insert((
                    begin,
                    end,
                    source.right_to_left,
                    source.collapsed_whitespace,
                ));
            edges.0 = edges.0.min(begin);
            edges.1 = edges.1.max(end);
            edges.3 &= source.collapsed_whitespace;
        }
        for ((start, end), (left, right, right_to_left, collapsed_whitespace)) in groups {
            let font = run.font_box;
            let rect = Rect {
                x: fragment.rect.x + left,
                y: fragment.rect.y + font.baseline - font.ascent,
                width: right - left,
                height: font.ascent + font.descent,
            };
            let quad = geometry::rect_quad(rect).map(|point| {
                geometry::transform(point, snapshot.native_transforms[fragment.index])
            });
            boxes.push(SourceClusterBox {
                right_to_left,
                collapsed_whitespace,
                fragment_index: fragment.index,
                source_id: run.source_id,
                source_ancestors: Arc::clone(&ancestry),
                utf16_start: start,
                utf16_end: end,
                rect,
                quad,
                transformed_bounds: geometry::bounds(quad),
            });
        }
    }
    boxes
}
