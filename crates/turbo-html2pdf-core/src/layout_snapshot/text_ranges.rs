//! Source selection follows actual placed cluster cells and native font boxes.
use super::{geometry, LayoutSnapshot, Rect, SourceClusterBox, SourceId};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextGeometryError {
    #[error("text source {0:?} is not present in this text-enabled snapshot")]
    MissingSource(SourceId),
    #[error("caret point coordinates must be finite")]
    InvalidPoint,
    #[error("caret transform is rotated, skewed or degenerate")]
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
    let mut boxes = selected_boxes(&cells, start, end);
    append_placed_gaps(&mut boxes, &cells, &text, start, end);
    append_empty_boundaries(&mut boxes, &cells, &text, start, end)?;
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
    let text: Vec<_> = source.text.encode_utf16().collect();
    if start > end || end > text.len() {
        return Err(TextGeometryError::InvalidOffsets);
    }
    Ok(text)
}

fn source_cells(snapshot: &LayoutSnapshot, id: SourceId) -> Vec<SourceClusterBox> {
    snapshot.text_cluster_boxes_for_source(id)
}

fn selected_boxes(cells: &[SourceClusterBox], start: usize, end: usize) -> Vec<SourceRangeBox> {
    let selected = selected_cells(cells, start, end);
    let mut boxes = Vec::new();
    for cell in selected {
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

fn transform_boxes(snapshot: &LayoutSnapshot, boxes: &mut [SourceRangeBox]) {
    boxes.sort_by_key(|box_| box_.fragment_index);
    for box_ in boxes {
        let matrix = snapshot.fragments()[box_.fragment_index].world_transform;
        box_.transformed_bounds = geometry::bounds(
            geometry::rect_quad(box_.rect).map(|point| geometry::transform(point, matrix)),
        );
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
    if !text[gap_start..gap_end]
        .iter()
        .all(|unit| is_collapsible(*unit))
    {
        return Err(TextGeometryError::UnresolvedSource);
    }
    match (before, after) {
        (Some(a), Some(b)) => gap_edges(a, b, start, end),
        (Some(_), None) => Ok(vec![(last, false)]),
        (None, Some(_)) => Ok(vec![(first, true)]),
        (None, None) => Err(TextGeometryError::UnresolvedSource),
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
        let edge = if start > before.utf16_end {
            (after, true)
        } else {
            (before, false)
        };
        return Ok(vec![edge]);
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
