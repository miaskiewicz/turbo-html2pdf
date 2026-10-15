use super::sources::SourceRecord;
use super::{GeometryKind, SnapshotFragment};
use crate::{Fragment, FragmentContent, NodeId, Transform2D};
use std::collections::HashMap;

const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

pub(crate) type CollectedGeometry = (Vec<SnapshotFragment>, Vec<[f64; 6]>, Vec<[f64; 6]>);

struct Capture<'a> {
    sources: &'a HashMap<NodeId, SourceRecord>,
    metrics: &'a HashMap<NodeId, super::UsedBoxMetrics>,
    transforms: &'a HashMap<NodeId, crate::layout::value::NativeTransform>,
    scale: Option<f32>,
}

#[derive(Clone, Copy)]
struct Inherited {
    layout: [f64; 6],
    hit: [f64; 6],
    paint_scope: [f32; 2],
}

pub(crate) fn collect(
    galley: &Fragment,
    sources: &HashMap<NodeId, SourceRecord>,
    metrics: &HashMap<NodeId, super::UsedBoxMetrics>,
    transforms: &HashMap<NodeId, crate::layout::value::NativeTransform>,
    scale: Option<f32>,
) -> CollectedGeometry {
    let mut fragments = (Vec::new(), Vec::new(), Vec::new());
    let capture = Capture {
        sources,
        metrics,
        transforms,
        scale,
    };
    append(
        galley,
        None,
        Inherited {
            layout: IDENTITY,
            hit: IDENTITY,
            paint_scope: [0.0, 0.0],
        },
        &capture,
        &mut fragments,
    );
    fragments
}

fn append(
    fragment: &Fragment,
    parent: Option<usize>,
    inherited: Inherited,
    capture: &Capture<'_>,
    out: &mut CollectedGeometry,
) {
    let source = capture.sources.get(&fragment.node_id);
    let own = capture
        .transforms
        .get(&fragment.node_id)
        .filter(|_| matches!(fragment.content, FragmentContent::Box { .. }))
        .map_or_else(
            || own_matrix(fragment),
            |transform| about_native_origin(*transform, fragment.x, fragment.y),
        );
    let matrix = multiply(inherited.layout, own);
    let (paint, paint_scope) =
        super::hit_geometry::own_matrix(fragment, own, inherited.paint_scope, capture.scale);
    let hit = multiply(inherited.hit, paint);
    let rect = Rect {
        x: fragment.x,
        y: fragment.y,
        width: fragment.width,
        height: fragment.height,
    };
    let quad = rect_quad(rect).map(|point| transform(point, matrix));
    let index = out.0.len();
    out.1.push(matrix);
    out.2.push(hit);
    out.0.push(SnapshotFragment {
        index,
        parent,
        layout_node_id: fragment.node_id,
        source_id: source.and_then(|s| s.id),
        source_ancestors: source.map_or_else(
            || inherited_sources(parent, &out.0),
            |s| s.ancestors.clone(),
        ),
        kind: kind(&fragment.content),
        used_box: used_box(fragment, capture.metrics),
        rect,
        world_transform: matrix.map(|value| value as f32),
        quad,
        transformed_bounds: bounds(quad),
    });
    let inherited = Inherited {
        layout: matrix,
        hit,
        paint_scope,
    };
    for i in fragment.paint_order() {
        append(&fragment.children[i], Some(index), inherited, capture, out);
    }
}

fn kind(content: &FragmentContent) -> GeometryKind {
    match content {
        FragmentContent::Box { .. } => GeometryKind::BorderBox,
        FragmentContent::Image(_) => GeometryKind::ImagePaint,
        FragmentContent::TextLine { .. } => GeometryKind::TextLineContainer,
        FragmentContent::Directive(_) => GeometryKind::Directive,
    }
}

fn own_matrix(fragment: &Fragment) -> [f64; 6] {
    match &fragment.content {
        FragmentContent::Box {
            transform: Some(matrix),
            ..
        } => about_origin(*matrix, fragment.x, fragment.y),
        _ => IDENTITY,
    }
}

fn about_native_origin(
    transform: crate::layout::value::NativeTransform,
    x: f32,
    y: f32,
) -> [f64; 6] {
    let mut matrix = transform.matrix;
    let ox = f64::from(x) + transform.origin[0];
    let oy = f64::from(y) + transform.origin[1];
    matrix[4] += ox - matrix[0] * ox - matrix[2] * oy;
    matrix[5] += oy - matrix[1] * ox - matrix[3] * oy;
    matrix
}

fn about_origin(transform: Transform2D, x: f32, y: f32) -> [f64; 6] {
    let mut matrix = transform.matrix.map(f64::from);
    let ox = f64::from(x) + f64::from(transform.origin_x);
    let oy = f64::from(y) + f64::from(transform.origin_y);
    matrix[4] += ox - matrix[0] * ox - matrix[2] * oy;
    matrix[5] += oy - matrix[1] * ox - matrix[3] * oy;
    matrix
}

fn multiply(a: [f64; 6], b: [f64; 6]) -> [f64; 6] {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

pub(crate) fn transform(point: Point, matrix: [f64; 6]) -> Point {
    let (x, y) = (f64::from(point.x), f64::from(point.y));
    Point {
        x: (matrix[0] * x + matrix[2] * y + matrix[4]) as f32,
        y: (matrix[1] * x + matrix[3] * y + matrix[5]) as f32,
    }
}

pub(crate) fn rect_quad(rect: Rect) -> [Point; 4] {
    [
        Point {
            x: rect.x,
            y: rect.y,
        },
        Point {
            x: rect.x + rect.width,
            y: rect.y,
        },
        Point {
            x: rect.x + rect.width,
            y: rect.y + rect.height,
        },
        Point {
            x: rect.x,
            y: rect.y + rect.height,
        },
    ]
}

pub(crate) fn bounds(quad: [Point; 4]) -> Rect {
    let x = quad.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let y = quad.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let right = quad.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
    let bottom = quad.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
    Rect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    }
}

fn inherited_sources(
    parent: Option<usize>,
    fragments: &[SnapshotFragment],
) -> Vec<super::SourceId> {
    let Some(parent) = parent else {
        return Vec::new();
    };
    let fragment = &fragments[parent];
    let mut ancestors = fragment.source_ancestors.clone();
    ancestors.extend(fragment.source_id);
    ancestors
}

fn used_box(
    fragment: &Fragment,
    metrics: &HashMap<NodeId, super::UsedBoxMetrics>,
) -> Option<super::UsedBoxMetrics> {
    if matches!(fragment.content, FragmentContent::Box { .. }) {
        metrics.get(&fragment.node_id).copied()
    } else {
        None
    }
}
