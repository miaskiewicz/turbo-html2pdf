//! Collapsed whitespace maps to a real following glyph's same physical edge.
use super::{LayoutSnapshot, SourceClusterBox};
use std::collections::HashMap;

pub(crate) fn canonicalize(
    snapshot: &LayoutSnapshot,
    cells: &[SourceClusterBox],
    index: usize,
    offset: usize,
) -> usize {
    if snapshot.html_layout_scale.is_none() {
        return offset;
    }
    let selected = &cells[index];
    let x = logical_edge(selected, offset == selected.utf16_start);
    let matching = cells.iter().filter(|cell| {
        logical_edge(cell, true) == x && same_line_projection(snapshot, selected, cell)
    });
    let (ends, zeros) = edge_offsets(matching);
    connected_suffix(ends, &zeros, offset)
}

fn connected_suffix(ends: Vec<usize>, zeros: &HashMap<usize, usize>, offset: usize) -> usize {
    ends.into_iter()
        .filter(|end| (suffix_start(zeros, *end)..=*end).contains(&offset))
        .max()
        .unwrap_or(offset)
}

fn edge_offsets<'a>(
    cells: impl Iterator<Item = &'a SourceClusterBox>,
) -> (Vec<usize>, HashMap<usize, usize>) {
    let mut ends = Vec::new();
    let mut zeros = HashMap::new();
    for cell in cells {
        if cell.rect.width > 0.0 {
            ends.push(cell.utf16_start);
        } else if cell.collapsed_whitespace {
            zeros.insert(cell.utf16_end, cell.utf16_start);
        }
    }
    (ends, zeros)
}
fn suffix_start(zeros: &HashMap<usize, usize>, end: usize) -> usize {
    let mut start = end;
    // Actual CharInfo whitespace intervals strictly decrease the source offset;
    // the suffix must terminate at a real glyph start on this same physical edge.
    while let Some(previous) = zeros.get(&start) {
        start = *previous;
    }
    start
}
fn same_line_projection(
    snapshot: &LayoutSnapshot,
    a: &SourceClusterBox,
    b: &SourceClusterBox,
) -> bool {
    let fa = &snapshot.fragments[a.fragment_index];
    let fb = &snapshot.fragments[b.fragment_index];
    fa.parent == fb.parent
        && fa.rect.y == fb.rect.y
        && snapshot.native_hit_transforms[fa.index] == snapshot.native_hit_transforms[fb.index]
}
fn logical_edge(cell: &SourceClusterBox, start: bool) -> f32 {
    if start == cell.right_to_left {
        cell.rect.x + cell.rect.width
    } else {
        cell.rect.x
    }
}
