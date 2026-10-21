//! Source selection follows actual placed cluster cells and native font boxes.
use super::{geometry, LayoutSnapshot, Rect, SourceClusterBox, SourceId};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextGeometryError {
    #[error("text source {0:?} is not present in this text-enabled snapshot")]
    MissingSource(SourceId),
    #[error("caret point coordinates must be finite")]
    InvalidPoint,
    #[error("caret transform is degenerate or unrepresentable")]
    UnsupportedCaretTransform,
    #[error("text selection offsets are outside the original UTF-16 source")]
    InvalidOffsets,
    #[error("selection has no placed source cluster or resolved collapsible gap")]
    UnresolvedSource,
}

/// An actual selected native text fragment, before and after its affine transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceRangeBox {
    pub fragment_index: usize,
    pub rect: Rect,
    pub transformed_bounds: Rect,
}

pub(crate) fn ranges(
    snapshot: &LayoutSnapshot,
    id: SourceId,
    start: usize,
    end: usize,
) -> Result<Vec<SourceRangeBox>, TextGeometryError> {
    let text = source_text(snapshot, id, start, end)?;
    let cells = source_cells(snapshot, id);
    if snapshot.collapsed_text_sources.contains(&id) {
        return Ok(Vec::new());
    }
    validate_source_coverage(&cells, &text, start, end)?;
    let mut boxes = selected_boxes(&cells, start, end);
    append_placed_gaps(&mut boxes, &cells, &text, start, end);
    append_empty_boundaries(&mut boxes, &cells, &text, start, end)?;
    append_native_wrap_boundary(snapshot, &mut boxes, &cells, &text, start, end);
    coalesce_native_runs(snapshot, &mut boxes);
    encompass_native_edges(snapshot, &cells, &mut boxes);
    transform_boxes(snapshot, &mut boxes);
    Ok(boxes)
}

fn source_text(
    snapshot: &LayoutSnapshot,
    id: SourceId,
    start: usize,
    end: usize,
) -> Result<Vec<u16>, TextGeometryError> {
    let source = snapshot
        .text_sources()
        .iter()
        .find(|source| source.source_id == id)
        .ok_or(TextGeometryError::MissingSource(id))?;
    let text = snapshot
        .text_original_units(id)
        .map(<[u16]>::to_vec)
        .unwrap_or_else(|| source.text.encode_utf16().collect());
    if start > end || end > text.len() {
        return Err(TextGeometryError::InvalidOffsets);
    }
    Ok(text)
}

fn source_cells(snapshot: &LayoutSnapshot, id: SourceId) -> Vec<SourceClusterBox> {
    let mut cells = snapshot.text_cluster_boxes_for_source(id);
    cells.sort_by_key(|cell| (cell.utf16_start, cell.utf16_end));
    cells
}

fn validate_source_coverage(
    cells: &[SourceClusterBox],
    text: &[u16],
    start: usize,
    end: usize,
) -> Result<(), TextGeometryError> {
    let mut covered = start;
    for cell in cells
        .iter()
        .filter(|cell| cell.utf16_end > start && cell.utf16_start < end)
    {
        let next = cell.utf16_start.min(end).max(covered);
        require_collapsible(&text[covered..next])?;
        covered = covered.max(cell.utf16_end.min(end));
    }
    require_collapsible(&text[covered..end])
}

fn require_collapsible(text: &[u16]) -> Result<(), TextGeometryError> {
    if text.iter().all(|unit| is_collapsible(*unit)) {
        Ok(())
    } else {
        Err(TextGeometryError::UnresolvedSource)
    }
}

fn selected_boxes(cells: &[SourceClusterBox], start: usize, end: usize) -> Vec<SourceRangeBox> {
    let selected = selected_cells(cells, start, end);
    let has_advance = selected.iter().any(|cell| cell.rect.width > 0.0);
    let mut boxes = Vec::new();
    for cell in selected
        .into_iter()
        .filter(|cell| !has_advance || cell.rect.width > 0.0)
    {
        append_box(&mut boxes, cell.fragment_index, cell.rect);
    }
    boxes
}

fn append_empty_boundaries(
    boxes: &mut Vec<SourceRangeBox>,
    cells: &[SourceClusterBox],
    text: &[u16],
    start: usize,
    end: usize,
) -> Result<(), TextGeometryError> {
    if boxes.is_empty() {
        for (cell, at_start) in boundary_cells(cells, text, start, end)? {
            let x = boundary_x(cell, at_start);
            append_box(
                boxes,
                cell.fragment_index,
                Rect {
                    x,
                    width: 0.0,
                    ..cell.rect
                },
            );
        }
    }
    Ok(())
}

// A separator-only selection retains both actual line affinities at a soft
// wrap. Positive selections continue to discard zero-advance cells globally.
fn append_native_wrap_boundary(
    snapshot: &LayoutSnapshot,
    boxes: &mut Vec<SourceRangeBox>,
    cells: &[SourceClusterBox],
    text: &[u16],
    start: usize,
    end: usize,
) {
    if !native_zero_selection(snapshot, boxes, text, start, end) {
        return;
    }
    let Some((before, after)) = wrap_boundary_cells(cells, end) else {
        return;
    };
    if different_line_origins(snapshot, before, after) {
        append_box(
            boxes,
            after.fragment_index,
            Rect {
                x: boundary_x(after, true),
                width: 0.0,
                ..after.rect
            },
        );
    }
}
fn native_zero_selection(
    snapshot: &LayoutSnapshot,
    boxes: &[SourceRangeBox],
    text: &[u16],
    start: usize,
    end: usize,
) -> bool {
    snapshot.html_layout_scale.is_some()
        && start < end
        && is_collapsible(text[end - 1])
        && boxes.iter().all(|box_| box_.rect.width == 0.0)
}
fn wrap_boundary_cells(
    cells: &[SourceClusterBox],
    end: usize,
) -> Option<(&SourceClusterBox, &SourceClusterBox)> {
    let before = cells
        .iter()
        .rev()
        .find(|cell| cell.utf16_end == end && cell.rect.width == 0.0)?;
    let after = cells.iter().find(|cell| cell.utf16_start == end)?;
    Some((before, after))
}
fn different_line_origins(
    snapshot: &LayoutSnapshot,
    before: &SourceClusterBox,
    after: &SourceClusterBox,
) -> bool {
    let before = &snapshot.fragments[before.fragment_index];
    let after = &snapshot.fragments[after.fragment_index];
    before.parent == after.parent && before.rect.y != after.rect.y
}

// Chrome's source-scoped FragmentItem selection encompasses nonempty offsets,
// while equal offsets retain one floored LayoutUnit edge. Quantize within the
// native fragment before transforms; generic/PDF source queries keep raw units.
fn encompass_native_edges(
    snapshot: &LayoutSnapshot,
    cells: &[SourceClusterBox],
    boxes: &mut [SourceRangeBox],
) {
    let Some(scale) = snapshot.html_layout_scale else {
        return;
    };
    let units = scale * 64.0;
    for box_ in boxes {
        let origin = cells
            .iter()
            .filter(|cell| {
                cell.rect.y == box_.rect.y
                    && cell.rect.height == box_.rect.height
                    && same_projection(snapshot, cell.fragment_index, box_.fragment_index)
            })
            .map(|cell| cell.rect.x)
            .reduce(f32::min)
            .unwrap_or(snapshot.fragments()[box_.fragment_index].rect.x);
        let left = box_.rect.x - origin;
        let right = left + box_.rect.width;
        let begin = (left * units).floor() / units;
        let end = if left == right {
            begin
        } else {
            (right * units).ceil() / units
        };
        box_.rect.x = origin + begin;
        box_.rect.width = end - begin;
    }
}

fn same_projection(snapshot: &LayoutSnapshot, a: usize, b: usize) -> bool {
    snapshot.native_transforms[a] == snapshot.native_transforms[b]
        && super::range_projection::compatible(snapshot, a, b)
}

fn coalesce_native_runs(snapshot: &LayoutSnapshot, boxes: &mut Vec<SourceRangeBox>) {
    if snapshot.html_layout_scale.is_none() {
        return;
    }
    let mut merged = Vec::<SourceRangeBox>::new();
    for box_ in boxes.drain(..) {
        let contiguous = merged.last_mut().filter(|previous| {
            previous.rect.y == box_.rect.y
                && previous.rect.height == box_.rect.height
                && (previous.rect.x + previous.rect.width - box_.rect.x).abs() < 0.0001
                && same_projection(snapshot, previous.fragment_index, box_.fragment_index)
        });
        match contiguous {
            Some(previous) => previous.rect.width = box_.rect.x + box_.rect.width - previous.rect.x,
            None => merged.push(box_),
        }
    }
    *boxes = merged;
}

fn transform_boxes(snapshot: &LayoutSnapshot, boxes: &mut [SourceRangeBox]) {
    boxes.sort_by_key(|box_| box_.fragment_index);
    for box_ in boxes {
        let matrix = snapshot.native_transforms[box_.fragment_index];
        box_.transformed_bounds = if snapshot.html_layout_scale.is_some() {
            super::range_projection::project(snapshot, box_.fragment_index, box_.rect)
        } else {
            geometry::bounds(
                geometry::rect_quad(box_.rect).map(|point| geometry::transform(point, matrix)),
            )
        };
    }
}

fn selected_cells(cells: &[SourceClusterBox], start: usize, end: usize) -> Vec<&SourceClusterBox> {
    cells
        .iter()
        .filter(|cell| {
            if start == end {
                cell.utf16_start < start && start < cell.utf16_end
            } else {
                cell.utf16_start < end && start < cell.utf16_end
            }
        })
        .collect()
}

fn boundary_cells<'a>(
    cells: &'a [SourceClusterBox],
    text: &[u16],
    start: usize,
    end: usize,
) -> Result<Vec<(&'a SourceClusterBox, bool)>, TextGeometryError> {
    let first = cells.first().ok_or(TextGeometryError::UnresolvedSource)?;
    let last = cells.last().ok_or(TextGeometryError::UnresolvedSource)?;
    let before = cells.iter().rev().find(|cell| cell.utf16_end <= start);
    let after = cells.iter().find(|cell| cell.utf16_start >= end);
    let gap_start = before.map_or(0, |cell| cell.utf16_end);
    let gap_end = after.map_or(text.len(), |cell| cell.utf16_start);
    require_collapsible(&text[gap_start..gap_end])?;
    boundary_edges(before, after, first, last, start, end)
}

fn boundary_edges<'a>(
    before: Option<&'a SourceClusterBox>,
    after: Option<&'a SourceClusterBox>,
    first: &'a SourceClusterBox,
    last: &'a SourceClusterBox,
    start: usize,
    end: usize,
) -> Result<Vec<(&'a SourceClusterBox, bool)>, TextGeometryError> {
    match before {
        // With no selected cell, absent preceding cells identify the real first
        // edge. Both neighbours cannot be absent when the cell list is nonempty.
        None => Ok(vec![(first, true)]),
        Some(a) => match after {
            Some(b) => gap_edges(a, b, start, end),
            None => Ok(vec![(last, false)]),
        },
    }
}

fn gap_edges<'a>(
    before: &'a SourceClusterBox,
    after: &'a SourceClusterBox,
    start: usize,
    end: usize,
) -> Result<Vec<(&'a SourceClusterBox, bool)>, TextGeometryError> {
    if start == end {
        return Ok(vec![collapsed_edge(before, after, start)]);
    }
    if before.fragment_index == after.fragment_index {
        // A selection starting at the preceding edge already appended the
        // actual placed pen gap. Only subsequent zero-advance spaces reach here.
        return Ok(vec![(after, true)]);
    }
    Ok(vec![(before, false), (after, true)])
}

fn collapsed_edge<'a>(
    before: &'a SourceClusterBox,
    after: &'a SourceClusterBox,
    offset: usize,
) -> (&'a SourceClusterBox, bool) {
    if offset == after.utf16_start {
        return (after, true);
    }
    if offset > before.utf16_end && before.fragment_index == after.fragment_index {
        return (after, true);
    }
    (before, false)
}

fn is_collapsible(unit: u16) -> bool {
    matches!(unit, 0x09 | 0x0a | 0x0c | 0x0d | 0x20)
}

fn boundary_x(cell: &SourceClusterBox, at_start: bool) -> f32 {
    if at_start == cell.right_to_left {
        cell.rect.x + cell.rect.width
    } else {
        cell.rect.x
    }
}

fn append_box(boxes: &mut Vec<SourceRangeBox>, index: usize, rect: Rect) {
    if let Some(last) = boxes.iter_mut().find(|last| last.fragment_index == index) {
        let right = (last.rect.x + last.rect.width).max(rect.x + rect.width);
        let bottom = (last.rect.y + last.rect.height).max(rect.y + rect.height);
        last.rect.x = last.rect.x.min(rect.x);
        last.rect.y = last.rect.y.min(rect.y);
        last.rect.width = right - last.rect.x;
        last.rect.height = bottom - last.rect.y;
    } else {
        boxes.push(SourceRangeBox {
            fragment_index: index,
            rect,
            transformed_bounds: rect,
        });
    }
}

// The layout's actual pen gap accounts for one collapsed space; subsequent
// original whitespace units have no advance. No glyph width is interpolated.
fn append_placed_gaps(
    boxes: &mut Vec<SourceRangeBox>,
    cells: &[SourceClusterBox],
    text: &[u16],
    start: usize,
    end: usize,
) {
    for pair in cells.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if !selected_gap(a, b, text, start, end) {
            continue;
        }
        let x1 = boundary_x(a, false);
        let x2 = boundary_x(b, true);
        let rect = Rect {
            x: x1.min(x2),
            width: (x2 - x1).abs(),
            ..a.rect
        };
        append_box(boxes, a.fragment_index, rect);
    }
}

fn selected_gap(
    a: &SourceClusterBox,
    b: &SourceClusterBox,
    text: &[u16],
    start: usize,
    end: usize,
) -> bool {
    a.fragment_index == b.fragment_index
        && a.utf16_end < b.utf16_start
        && start <= a.utf16_end
        && a.utf16_end < end
        && text[a.utf16_end..b.utf16_start]
            .iter()
            .all(|unit| is_collapsible(*unit))
}

#[cfg(test)]
mod native_capture_loss_contracts {
    #[test]
    fn missing_actual_source_glyphs_fail_closed_instead_of_publishing_partial_ranges() {
        use super::*;
        use crate::layout_snapshot::{
            snapshot_html_live_tree_with_text, SnapshotInputs, SourceIdentity,
        };
        use crate::{build_cascade, Diagnostics, FontRegistry, ImageCtx, NoImages, Node};
        let nodes = [Node::Text("ab".into())];
        let fonts = FontRegistry::new();
        let images = ImageCtx {
            resolver: &NoImages,
            body_height: None,
        };
        let mut native = snapshot_html_live_tree_with_text(
            &nodes,
            &[SourceIdentity {
                id: SourceId(100),
                path: vec![0],
            }],
            &build_cascade("", "", Default::default()),
            &SnapshotInputs {
                width: 200.0,
                height: 80.0,
                fonts: &fonts,
                images: &images,
            },
            &mut Diagnostics::default(),
        )
        .unwrap();
        assert_eq!(
            native.text_range_boxes(SourceId(100), 0, 2).unwrap().len(),
            1
        );
        // Remove an actual captured glyph to model an incomplete provider result;
        // the remaining font/glyph/source positions are the real native output.
        native.text_runs[0]
            .glyphs
            .retain(|glyph| glyph.source.utf16_end <= 1);
        assert_eq!(
            native.text_range_boxes(SourceId(100), 0, 2),
            Err(TextGeometryError::UnresolvedSource)
        );
        assert_eq!(
            native.text_range_boxes(SourceId(100), 1, 1),
            Err(TextGeometryError::UnresolvedSource)
        );
        assert_eq!(
            native.text_range_boxes(SourceId(100), 0, 1).unwrap().len(),
            1
        );
    }
}
