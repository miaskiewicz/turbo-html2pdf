//! Caret hits select real cluster edges; they never subdivide glyph advances.
use super::{LayoutSnapshot, Point, Rect, SourceClusterBox, SourceId, TextGeometryError};

pub(crate) fn offset(
    snapshot: &LayoutSnapshot,
    id: SourceId,
    point: Point,
) -> Result<usize, TextGeometryError> {
    validate_point(point)?;
    if !snapshot
        .text_sources()
        .iter()
        .any(|source| source.source_id == id)
    {
        return Err(TextGeometryError::MissingSource(id));
    }
    let mut best = None;
    for cell in snapshot.text_cluster_boxes_for_source(id).iter() {
        let fragment = &snapshot.fragments()[cell.fragment_index];
        let matrix = fragment.world_transform;
        let local = local_point(point, matrix)?;
        let distance = vertical_distance(local.y, cell.rect) * matrix[3].abs();
        let edge = nearest_edge(local.x, cell);
        consider(
            &mut best,
            distance,
            fragment.rect.y,
            cell.fragment_index,
            edge,
        );
    }
    best.map(|(_, _, _, (_, offset))| offset)
        .ok_or(TextGeometryError::UnresolvedSource)
}

fn validate_point(point: Point) -> Result<(), TextGeometryError> {
    if point.x.is_finite() && point.y.is_finite() {
        Ok(())
    } else {
        Err(TextGeometryError::InvalidPoint)
    }
}

fn local_point(point: Point, matrix: [f32; 6]) -> Result<Point, TextGeometryError> {
    if unsupported_transform(matrix) {
        return Err(TextGeometryError::UnsupportedCaretTransform);
    }
    let local = Point {
        x: (point.x - matrix[4]) / matrix[0],
        y: (point.y - matrix[5]) / matrix[3],
    };
    validate_point(local)?;
    Ok(local)
}

fn unsupported_transform(matrix: [f32; 6]) -> bool {
    matrix[1] != 0.0 || matrix[2] != 0.0 || matrix[0] == 0.0 || matrix[3] == 0.0
}

fn vertical_distance(y: f32, rect: Rect) -> f32 {
    (rect.y - y).max(y - rect.y - rect.height).max(0.0)
}

fn nearest_edge(x: f32, cell: &SourceClusterBox) -> (f32, usize) {
    let left = (x - cell.rect.x).abs();
    let right = (x - cell.rect.x - cell.rect.width).abs();
    let (start, end) = if cell.right_to_left {
        (cell.utf16_end, cell.utf16_start)
    } else {
        (cell.utf16_start, cell.utf16_end)
    };
    if left <= right {
        (left, start)
    } else {
        (right, end)
    }
}

type Candidate = (f32, f32, usize, (f32, usize));

fn consider(
    best: &mut Option<Candidate>,
    vertical: f32,
    line_top: f32,
    fragment: usize,
    edge: (f32, usize),
) {
    let candidate = (vertical, line_top, fragment, edge);
    if best.is_none_or(|old| preference(candidate) < preference(old)) {
        *best = Some(candidate);
    }
}

fn preference(candidate: Candidate) -> (f32, f32, f32, usize) {
    // Overlapping font boxes hit the first actual line; equal gaps between font
    // boxes select the following line. Runs on that line compete by real edges.
    let line = if candidate.0 == 0.0 {
        candidate.1
    } else {
        -candidate.1
    };
    (candidate.0, line, candidate.3 .0, candidate.2)
}
