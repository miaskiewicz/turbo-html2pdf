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
    let cells = snapshot.text_cluster_boxes_for_source(id);
    let mut best = None;
    for (index, cell) in cells.iter().enumerate() {
        let fragment = &snapshot.fragments()[cell.fragment_index];
        let matrix = snapshot.native_hit_transforms[fragment.index];
        let local = local_point(point, matrix, snapshot.html_layout_scale)?;
        let distance = vertical_distance(local.y, cell.rect) * matrix[2].hypot(matrix[3]) as f32;
        let mut edge = nearest_edge(local.x, cell);
        edge.0 *= matrix[0].hypot(matrix[1]) as f32;
        consider(
            &mut best,
            distance,
            fragment.rect.y,
            cell.fragment_index,
            edge,
            index,
        );
    }
    best.map(|(_, _, _, (_, offset), index)| {
        super::text_caret_edges::canonicalize(snapshot, &cells, index, offset)
    })
    .ok_or(TextGeometryError::UnresolvedSource)
}

fn validate_point(point: Point) -> Result<(), TextGeometryError> {
    if point.x.is_finite() && point.y.is_finite() {
        Ok(())
    } else {
        Err(TextGeometryError::InvalidPoint)
    }
}

fn local_point(
    point: Point,
    matrix: [f64; 6],
    scale: Option<f32>,
) -> Result<Point, TextGeometryError> {
    let [a, b, c, d, e, f] = matrix;
    let determinant = a * d - b * c;
    if determinant == 0.0 || !determinant.is_finite() {
        return Err(TextGeometryError::UnsupportedCaretTransform);
    }
    let (x, y) = (f64::from(point.x) - e, f64::from(point.y) - f);
    let local = Point {
        x: ((d * x - c * y) / determinant) as f32,
        y: ((a * y - b * x) / determinant) as f32,
    };
    validate_point(local)?;
    let local = physical_hit_point(local, scale);
    validate_point(local)?;
    Ok(local)
}

// A transformed HitTestLocation preserves the original PointF through inverse
// projection, then floors the resulting local PhysicalOffset.
fn physical_hit_point(point: Point, scale: Option<f32>) -> Point {
    match scale {
        Some(scale) => {
            let units = 64.0 * scale;
            Point {
                x: (point.x * units).floor() / units,
                y: (point.y * units).floor() / units,
            }
        }
        None => point,
    }
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

type Candidate = (f32, f32, usize, (f32, usize), usize);

fn consider(
    best: &mut Option<Candidate>,
    vertical: f32,
    line_top: f32,
    fragment: usize,
    edge: (f32, usize),
    index: usize,
) {
    let candidate = (vertical, line_top, fragment, edge, index);
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
