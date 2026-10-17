//! Text selection maps a local FloatQuad through each flat transform boundary.
//! Unlike element geometry, ordinary ancestor offsets after a transform are
//! added to that float quad rather than fused into its double matrix. Chromium
//! 155.0.8059.80 TransformState::{ApplyTransform,FlattenWithTransform} defines
//! these boundaries. LayoutBox Element quads share this path; LayoutInline
//! Element quads instead force one accumulated transform and are kept separate.
use super::{geometry, GeometryKind, LayoutSnapshot, Point, Rect, SnapshotFragment};
use crate::layout::value::{NativeTransform, Position};
use crate::NodeId;
use std::collections::HashMap;

pub(super) struct Projection {
    container: Option<usize>,
    local: Option<[f64; 6]>,
    transformed: bool,
}
#[derive(Clone, Copy, Default)]
struct Scope {
    absolute: Option<usize>,
    fixed: Option<usize>,
}
impl Scope {
    fn container(self, fragment: &SnapshotFragment) -> Option<usize> {
        match positioned(fragment) {
            Position::Fixed => self.fixed,
            Position::Absolute => self.absolute,
            _ => fragment.parent,
        }
    }
    fn descend(mut self, index: usize, fragment: &SnapshotFragment, transformed: bool) -> Self {
        if positioned(fragment) != Position::Static || transformed {
            self.absolute = Some(index);
        }
        if transformed {
            self.fixed = Some(index);
        }
        self
    }
}
pub(super) fn collect(
    fragments: &[SnapshotFragment],
    transforms: &HashMap<NodeId, NativeTransform>,
    native: bool,
) -> Vec<Projection> {
    if !native {
        return Vec::new();
    }
    let mut projections: Vec<Projection> = Vec::with_capacity(fragments.len());
    let mut scopes: Vec<Scope> = Vec::with_capacity(fragments.len());
    for fragment in fragments {
        let scope = fragment
            .parent
            .map_or_else(Scope::default, |parent| scopes[parent]);
        let container = scope.container(fragment);
        let local = transforms
            .get(&fragment.layout_node_id)
            .filter(|_| fragment.kind == GeometryKind::BorderBox)
            .map(local_matrix);
        let transformed =
            local.is_some() || container.is_some_and(|parent| projections[parent].transformed);
        scopes.push(scope.descend(fragment.index, fragment, local.is_some()));
        projections.push(Projection {
            container,
            local,
            transformed,
        });
    }
    projections
}
fn positioned(fragment: &SnapshotFragment) -> Position {
    fragment
        .used_box
        .map_or(Position::Static, |metrics| metrics.position)
}
pub(super) fn compatible(snapshot: &LayoutSnapshot, a: usize, b: usize) -> bool {
    let a = &snapshot.range_projection[a];
    let b = &snapshot.range_projection[b];
    a.container == b.container && a.local == b.local
}
fn local_matrix(transform: &NativeTransform) -> [f64; 6] {
    let mut matrix = transform.matrix;
    let [x, y] = transform.origin;
    matrix[4] += x - matrix[0] * x - matrix[2] * y;
    matrix[5] += y - matrix[1] * x - matrix[3] * y;
    matrix
}
fn origin(snapshot: &LayoutSnapshot, index: Option<usize>) -> Point {
    index.map_or(Point { x: 0.0, y: 0.0 }, |index| {
        let rect = snapshot.fragments[index].rect;
        Point {
            x: rect.x,
            y: rect.y,
        }
    })
}
fn move_quad(quad: &mut [Point; 4], offset: Point) {
    for point in quad {
        point.x += offset.x;
        point.y += offset.y;
    }
}

fn integer_translation(matrix: [f64; 6]) -> Option<Point> {
    (matrix[..4] == [1.0, 0.0, 0.0, 1.0] && matrix[4].fract() == 0.0 && matrix[5].fract() == 0.0)
        .then_some(Point {
            x: matrix[4] as f32,
            y: matrix[5] as f32,
        })
}
fn apply_transform(quad: &mut [Point; 4], pending: &mut Point, matrix: [f64; 6]) {
    if let Some(offset) = integer_translation(matrix) {
        pending.x += offset.x;
        pending.y += offset.y;
    } else {
        move_quad(quad, *pending);
        *pending = Point { x: 0.0, y: 0.0 };
        *quad = quad.map(|point| geometry::transform(point, matrix));
    }
}

pub(super) fn project(snapshot: &LayoutSnapshot, index: usize, rect: Rect) -> Rect {
    geometry::bounds(quad(snapshot, index, rect))
}
fn quad(snapshot: &LayoutSnapshot, index: usize, rect: Rect) -> [Point; 4] {
    if !snapshot.range_projection[index].transformed {
        return geometry::rect_quad(rect);
    }
    let start = origin(snapshot, Some(index));
    let mut quad = geometry::rect_quad(rect);
    move_quad(
        &mut quad,
        Point {
            x: -start.x,
            y: -start.y,
        },
    );
    let mut cursor = Some(index);
    let mut pending = Point { x: 0.0, y: 0.0 };
    while let Some(current) = cursor {
        let projection = &snapshot.range_projection[current];
        let from = origin(snapshot, Some(current));
        let to = origin(snapshot, projection.container);
        let offset = Point {
            x: from.x - to.x,
            y: from.y - to.y,
        };
        if let Some(mut matrix) = projection.local {
            matrix[4] += f64::from(offset.x);
            matrix[5] += f64::from(offset.y);
            apply_transform(&mut quad, &mut pending, matrix);
        } else {
            pending.x += offset.x;
            pending.y += offset.y;
        }
        cursor = projection.container;
    }
    move_quad(&mut quad, pending);
    quad
}

pub(super) fn place_boxes(snapshot: &mut LayoutSnapshot) {
    if snapshot.html_layout_scale.is_none() {
        return;
    }
    for index in 0..snapshot.fragments.len() {
        let fragment = &snapshot.fragments[index];
        if block_border(fragment) {
            let projected = quad(snapshot, index, fragment.rect);
            let fragment = &mut snapshot.fragments[index];
            fragment.quad = projected;
            fragment.transformed_bounds = geometry::bounds(projected);
        }
    }
}
fn block_border(fragment: &SnapshotFragment) -> bool {
    fragment.kind == GeometryKind::BorderBox
        && fragment
            .used_box
            .is_some_and(|metrics| !metrics.inline_fragment)
}
