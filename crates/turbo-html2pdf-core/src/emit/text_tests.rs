use super::*;
use crate::layout::fragment::NodeId;

fn fragment(positions: &[(f32, f32)], font_size: f32) -> Fragment {
    let face = turbo_dom_layout::test_support::evolventa();
    let shaped = face.shape("CAB");
    let glyphs = positions
        .iter()
        .zip(shaped)
        .map(|(&(x, y), glyph)| PositionedGlyph {
            glyph_id: glyph.glyph_id,
            x,
            y,
            text_start: 0,
            text_end: 0,
        })
        .collect();
    Fragment::new(
        NodeId(1),
        20.25,
        30.5,
        200.0,
        20.0,
        FragmentContent::TextLine {
            glyphs,
            face,
            text: String::new(),
            font_size,
            color: Rgba::new(20, 40, 60, 255),
            upright: false,
            vertical: false,
        },
    )
}

fn line(fragment: &Fragment) -> Line<'_> {
    let FragmentContent::TextLine {
        glyphs,
        face,
        font_size,
        color,
        upright,
        vertical,
        ..
    } = &fragment.content
    else {
        panic!("text fixture");
    };
    Line {
        frag: fragment,
        glyphs,
        face,
        font_size: *font_size,
        color: *color,
        horizontal: !upright && !vertical,
    }
}

fn paint(fragment: &Fragment) -> String {
    let line = line(fragment);
    let mut fonts = FontStore::default();
    let gids: Vec<u16> = line.glyphs.iter().map(|g| g.glyph_id).collect();
    fonts.record_glyphs(line.face, &gids);
    let mut content = Content::new();
    paint_text(&mut content, fragment, &fonts, 600.0, false);
    String::from_utf8(content.finish().to_vec()).unwrap()
}

fn adjustments(content: &str) -> Vec<f64> {
    let array = content.lines().find(|line| line.ends_with(" TJ")).unwrap();
    array
        .split(')')
        .filter_map(|part| part.split_once('('))
        .filter_map(|(prefix, _)| prefix.trim().parse().ok())
        .collect()
}

#[test]
fn positioned_array_preserves_backward_and_fractional_glyph_positions() {
    let fragment = fragment(&[(7.25, 12.5), (-4.125, 12.5), (23.375, 12.5)], 13.25);
    let line = line(&fragment);
    let content = paint(&fragment);
    assert_eq!(content.matches(" Tm").count(), 1);
    assert_eq!(content.matches(" TJ").count(), 1);
    assert_eq!(content.matches(" Tj").count(), 0);
    let displacements = adjustments(&content);
    assert_eq!(displacements.len(), 2);
    let mut position = f64::from(px_to_pt(line.frag.x + line.glyphs[0].x));
    for (glyphs, displacement) in line.glyphs.windows(2).zip(displacements) {
        let width = f32::from(line.face.glyph_advance(glyphs[0].glyph_id))
            * (1000.0 / f32::from(line.face.units_per_em()));
        position +=
            (f64::from(width) - displacement) * f64::from(px_to_pt(line.font_size)) / 1000.0;
        assert!((position - f64::from(px_to_pt(line.frag.x + glyphs[1].x))).abs() < 0.00001);
    }
}

#[test]
fn vertical_offsets_keep_absolute_glyph_matrices() {
    let fragment = fragment(&[(0.0, 12.0), (3.5, 10.0), (8.0, 12.0)], 12.0);
    let content = paint(&fragment);
    assert_eq!(content.matches(" Tm").count(), 3);
    assert_eq!(content.matches(" Tj").count(), 3);
    assert_eq!(content.matches(" TJ").count(), 0);
    assert!(content.contains("1 0 0 1 15.1875 568.125 Tm"));
    assert!(content.contains("1 0 0 1 17.8125 569.625 Tm"));
    assert!(content.contains("1 0 0 1 21.1875 568.125 Tm"));
}

#[test]
fn zero_size_and_empty_lines_retain_legacy_behavior() {
    let zero = paint(&fragment(&[(0.0, 12.0)], 0.0));
    assert_eq!(zero.matches(" Tm").count(), 1);
    assert_eq!(zero.matches(" Tj").count(), 1);
    let empty = paint(&fragment(&[], 12.0));
    assert_eq!(empty.matches(" Tm").count(), 0);
    assert_eq!(empty.matches(" TJ").count(), 0);
}

#[test]
fn vertical_writing_and_nonfinite_inputs_do_not_batch() {
    let mut fragment = fragment(&[(0.0, 12.0)], 12.0);
    if let FragmentContent::TextLine { vertical, .. } = &mut fragment.content {
        *vertical = true;
    }
    assert!(!can_batch(&line(&fragment)));
    assert_eq!(paint(&fragment).matches(" Tj").count(), 1);
    assert!(!can_batch(&line(&self::fragment(&[(0.0, 12.0)], f32::NAN))));
    assert!(!can_batch(&line(&self::fragment(
        &[(f32::INFINITY, 12.0)],
        12.0
    ))));
}

#[test]
fn overflowing_adjustments_keep_finite_absolute_matrices() {
    let fragment = fragment(&[(-f32::MAX, 12.0), (f32::MAX, 12.0)], 12.0);
    let content = paint(&fragment);
    assert!(!can_batch(&line(&fragment)));
    assert_eq!(content.matches(" Tm").count(), 2);
    assert_eq!(content.matches(" Tj").count(), 2);
    assert_eq!(content.matches(" TJ").count(), 0);
}

#[test]
fn tiny_fonts_and_nonfinite_baselines_keep_absolute_placement() {
    let tiny = fragment(&[(0.0, 12.0), (1.0, 12.0)], f32::MIN_POSITIVE);
    assert!(!can_batch(&line(&tiny)));
    for y in [f32::NAN, f32::INFINITY] {
        assert!(!can_batch(&line(&fragment(&[(0.0, y)], 12.0))));
    }
    let mut shifted = fragment(&[(0.0, 12.0)], 12.0);
    shifted.x = f32::NAN;
    assert!(!can_batch(&line(&shifted)));
    shifted.x = 0.0;
    shifted.y = f32::INFINITY;
    assert!(!can_batch(&line(&shifted)));
}

#[test]
fn finite_excursions_that_lose_visible_position_precision_do_not_batch() {
    let fragment = fragment(&[(0.0, 12.0), (100_000_000.0, 12.0), (10.0, 12.0)], 13.25);
    let line = line(&fragment);
    assert!(line
        .glyphs
        .windows(2)
        .all(|p| glyph_adjustment(&line, &p[0], &p[1]).is_finite()));
    assert!(!can_batch(&line));
    let content = paint(&fragment);
    assert_eq!(content.matches(" Tm").count(), 3);
    assert_eq!(content.matches(" TJ").count(), 0);
    assert!(content.contains("1 0 0 1 22.6875 568.125 Tm"));
}

#[test]
fn cumulative_rounding_is_bounded_across_long_lines() {
    let mut fragment = fragment(&[(0.0, 12.0)], 13.25);
    if let FragmentContent::TextLine { glyphs, .. } = &mut fragment.content {
        let glyph_id = glyphs[0].glyph_id;
        *glyphs = (0..10_000)
            .map(|i| PositionedGlyph {
                glyph_id,
                x: i as f32 * 0.123_456_7,
                y: 12.0,
                text_start: 0,
                text_end: 0,
            })
            .collect();
    }
    assert!(!can_batch(&line(&fragment)));
}

#[test]
fn pdf_number_bounds_include_the_writers_integer_fast_path() {
    let exact = PdfNumber::new(12.0);
    assert_eq!(exact.value, 12.0);
    assert_eq!(exact.error, 0.0);
    let saturated = PdfNumber::new(i32::MAX as f32);
    assert_eq!(saturated.value, f64::from(i32::MAX));
    assert_eq!(saturated.error, 0.0);
}
