use super::sources::SourceRecord;
use super::{GeometryKind, SnapshotFragment};
use crate::{Fragment, FragmentContent, NodeId, Transform2D};
use std::collections::HashMap;

const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

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

pub(crate) fn collect(
    galley: &Fragment,
    sources: &HashMap<NodeId, SourceRecord>,
    metrics: &HashMap<NodeId, super::UsedBoxMetrics>,
) -> Vec<SnapshotFragment> {
    let mut fragments = Vec::new();
    append(galley, None, IDENTITY, sources, metrics, &mut fragments);
    fragments
}

fn append(
    fragment: &Fragment,
    parent: Option<usize>,
    inherited: [f32; 6],
    sources: &HashMap<NodeId, SourceRecord>,
    metrics: &HashMap<NodeId, super::UsedBoxMetrics>,
    out: &mut Vec<SnapshotFragment>,
) {
    let source = sources.get(&fragment.node_id);
    let matrix = multiply(inherited, own_matrix(fragment));
    let rect = Rect {
        x: fragment.x,
        y: fragment.y,
        width: fragment.width,
        height: fragment.height,
    };
    let quad = rect_quad(rect).map(|point| transform(point, matrix));
    let index = out.len();
    out.push(SnapshotFragment {
        index,
        parent,
        layout_node_id: fragment.node_id,
        source_id: source.and_then(|s| s.id),
        source_ancestors: source
            .map_or_else(|| inherited_sources(parent, out), |s| s.ancestors.clone()),
        kind: kind(&fragment.content),
        used_box: used_box(fragment, metrics),
        rect,
        world_transform: matrix,
        quad,
        transformed_bounds: bounds(quad),
    });
    for i in fragment.paint_order() {
        append(
            &fragment.children[i],
            Some(index),
            matrix,
            sources,
            metrics,
            out,
        );
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

fn own_matrix(fragment: &Fragment) -> [f32; 6] {
    match &fragment.content {
        FragmentContent::Box {
            transform: Some(matrix),
            ..
        } => about_origin(*matrix, fragment.x, fragment.y),
        _ => IDENTITY,
    }
}

fn about_origin(transform: Transform2D, x: f32, y: f32) -> [f32; 6] {
    let mut matrix = transform.matrix;
    let ox = x + transform.origin_x;
    let oy = y + transform.origin_y;
    matrix[4] += ox - matrix[0] * ox - matrix[2] * oy;
    matrix[5] += oy - matrix[1] * ox - matrix[3] * oy;
    matrix
}

fn multiply(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

pub(crate) fn transform(point: Point, matrix: [f32; 6]) -> Point {
    Point {
        x: matrix[0] * point.x + matrix[2] * point.y + matrix[4],
        y: matrix[1] * point.x + matrix[3] * point.y + matrix[5],
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
