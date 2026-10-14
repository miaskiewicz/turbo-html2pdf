//! Fail before publishing text metadata if real placement overflowed native floats.
use super::{LayoutSnapshot, Rect, SnapshotFragment, SnapshotTextRun};

pub(super) fn valid(snapshot: &LayoutSnapshot) -> bool {
    snapshot.fragments.iter().all(fragment)
        && snapshot.text_runs.iter().all(run)
        && snapshot
            .native_transforms
            .iter()
            .chain(&snapshot.native_hit_transforms)
            .flatten()
            .all(|value| value.is_finite())
}
fn rect(value: Rect) -> bool {
    [
        value.x,
        value.y,
        value.width,
        value.height,
        value.x + value.width,
        value.y + value.height,
    ]
    .into_iter()
    .all(f32::is_finite)
}
fn fragment(value: &SnapshotFragment) -> bool {
    rect(value.rect)
        && rect(value.transformed_bounds)
        && value.world_transform.into_iter().all(f32::is_finite)
        && value
            .quad
            .iter()
            .all(|p| p.x.is_finite() && p.y.is_finite())
}
fn run(value: &SnapshotTextRun) -> bool {
    let font = value.font_box;
    [
        font.baseline,
        font.ascent,
        font.descent,
        font.ascent + font.descent,
    ]
    .into_iter()
    .all(f32::is_finite)
        && value.glyphs.iter().all(|glyph| {
            [
                glyph.x,
                glyph.y,
                glyph.source.pen_x,
                glyph.source.advance,
                glyph.source.pen_x + glyph.source.advance,
            ]
            .into_iter()
            .all(f32::is_finite)
        })
}
