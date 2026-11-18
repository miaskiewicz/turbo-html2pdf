//! Text showing (§7, AC-7.2). A `TextLine` fragment becomes one text object: we
//! select the line's subset font and show its 2-byte `Identity-H` glyph codes.
//! Horizontal lines share a positioned text array; its adjustments account for
//! the embedded font advances so the galley still determines every glyph's spot.
//! Lines with vertical offsets retain independent absolute glyph matrices.

use pdf_writer::{Content, Name, Str};

use crate::layout::fragment::{Fragment, FragmentContent, PositionedGlyph};
use crate::layout::value::Rgba;
use crate::text::FontFace;

use super::color::set_fill;
use super::fonts::FontStore;
use super::unit::{flip_y, px_to_pt};

/// Paint a `TextLine` fragment as a PDF text object. No-op for any other
/// fragment content (boxes/directives are handled elsewhere). `cmyk` selects the
/// device colour space for the fill (see [`set_fill`]).
pub fn paint_text(
    content: &mut Content,
    frag: &Fragment,
    fonts: &FontStore,
    page_height_pt: f32,
    cmyk: bool,
) {
    if let FragmentContent::TextLine {
        glyphs,
        face,
        font_size,
        color,
        upright,
        vertical,
        ..
    } = &frag.content
    {
        let line = Line {
            frag,
            glyphs,
            face,
            font_size: *font_size,
            color: *color,
            horizontal: !upright && !vertical,
        };
        show_line(content, &line, fonts, page_height_pt, cmyk);
    }
}

/// One text line resolved from its fragment, ready to show.
struct Line<'a> {
    frag: &'a Fragment,
    glyphs: &'a [PositionedGlyph],
    face: &'a FontFace,
    font_size: f32,
    color: Rgba,
    horizontal: bool,
}

fn show_line(
    content: &mut Content,
    line: &Line,
    fonts: &FontStore,
    page_height_pt: f32,
    cmyk: bool,
) {
    let face_index = fonts.index_of(line.face);
    let resource = FontStore::resource_name(face_index);
    content.begin_text();
    content.set_font(Name(resource.as_bytes()), px_to_pt(line.font_size));
    set_fill(content, line.color, cmyk);
    show_glyphs(content, line, fonts, face_index, page_height_pt);
    content.end_text();
}

fn show_glyphs(
    content: &mut Content,
    line: &Line,
    fonts: &FontStore,
    face_index: usize,
    page_height_pt: f32,
) {
    if can_batch(line) {
        show_batch(content, line, fonts, face_index, page_height_pt);
    } else {
        for glyph in line.glyphs {
            show_glyph(content, line, glyph, fonts, face_index, page_height_pt);
        }
    }
}

fn can_batch(line: &Line) -> bool {
    let Some(first) = line.glyphs.first() else {
        return false;
    };
    line.horizontal
        && valid_size(line.font_size)
        && same_baseline(line, first)
        && accurate_adjustments(line)
}

fn valid_size(size: f32) -> bool {
    size.is_finite() && size > 0.0
}

fn same_baseline(line: &Line, first: &PositionedGlyph) -> bool {
    line.frag.x.is_finite()
        && line.frag.y.is_finite()
        && line
            .glyphs
            .iter()
            .all(|g| g.y == first.y && g.x.is_finite() && g.y.is_finite())
}

// Admission bounds the entire line, rather than each offset independently:
// large excursions can lose low bits and shift a later, visible glyph.
const MAX_POSITION_ERROR_PT: f64 = 0.0001;

#[derive(Clone, Copy)]
struct PdfNumber {
    value: f64,
    error: f64,
}

impl PdfNumber {
    // pdf-writer writes exact i32s or shortest round-trippable f32 decimals.
    // Such a decimal is inside the input's half-ULP rounding interval.
    fn new(value: f32) -> Self {
        if value as i32 as f32 == value {
            return Self {
                value: f64::from(value as i32),
                error: 0.0,
            };
        }
        let center = f64::from(value);
        let above = f64::from(value.next_up()) - center;
        let below = center - f64::from(value.next_down());
        Self {
            value: center,
            error: above.max(below) * 0.5,
        }
    }
}

fn accurate_adjustments(line: &Line) -> bool {
    let mut position = PdfNumber::new(glyph_x(line, &line.glyphs[0]));
    let size = PdfNumber::new(px_to_pt(line.font_size));
    line.glyphs
        .windows(2)
        .all(|pair| accurate_step(line, pair, size, &mut position))
}

fn accurate_step(
    line: &Line,
    pair: &[PositionedGlyph],
    size: PdfNumber,
    position: &mut PdfNumber,
) -> bool {
    let width = PdfNumber::new(embedded_advance(line, &pair[0]));
    let adjustment = PdfNumber::new(glyph_adjustment(line, &pair[0], &pair[1]));
    advance_position(position, width, adjustment, size);
    let target = PdfNumber::new(glyph_x(line, &pair[1]));
    (position.value - target.value).abs() + position.error + target.error <= MAX_POSITION_ERROR_PT
}

fn advance_position(
    position: &mut PdfNumber,
    width: PdfNumber,
    offset: PdfNumber,
    size: PdfNumber,
) {
    let difference = width.value - offset.value;
    let step = difference * size.value / 1000.0;
    position.error += ((width.error + offset.error) * (size.value.abs() + size.error)
        + difference.abs() * size.error)
        / 1000.0;
    // Also cover the arithmetic used to reconstruct PDF's real-number advance.
    position.error += (position.value.abs() + step.abs()) * f64::EPSILON * 16.0;
    position.value += step;
}

fn glyph_x(line: &Line, glyph: &PositionedGlyph) -> f32 {
    px_to_pt(line.frag.x + glyph.x)
}

fn embedded_advance(line: &Line, glyph: &PositionedGlyph) -> f32 {
    let scale = 1000.0 / f32::from(line.face.units_per_em());
    f32::from(line.face.glyph_advance(glyph.glyph_id)) * scale
}

fn show_batch(
    content: &mut Content,
    line: &Line,
    fonts: &FontStore,
    face_index: usize,
    page_height_pt: f32,
) {
    let first = &line.glyphs[0];
    place_glyph(content, line, first, page_height_pt);
    let mut positioned = content.show_positioned();
    let mut items = positioned.items();
    items.show(Str(&fonts.remap(face_index, first.glyph_id).to_be_bytes()));
    for pair in line.glyphs.windows(2) {
        items.adjust(glyph_adjustment(line, &pair[0], &pair[1]));
        items.show(Str(&fonts
            .remap(face_index, pair[1].glyph_id)
            .to_be_bytes()));
    }
}

/// TJ subtracts thousandths of text space after the font's embedded advance.
/// Include the actual advance even when shaping moves backwards (bidi/kerning).
/// This private painter uses fresh page content with default Tc/Tw/Tz; changing
/// that text state must also update the advance calculation.
fn glyph_adjustment(line: &Line, previous: &PositionedGlyph, next: &PositionedGlyph) -> f32 {
    let advance = embedded_advance(line, previous);
    let previous_x = glyph_x(line, previous);
    let next_x = glyph_x(line, next);
    advance - (next_x - previous_x) * 1000.0 / px_to_pt(line.font_size)
}

/// Place and show a single glyph at its galley position.
fn show_glyph(
    content: &mut Content,
    line: &Line,
    glyph: &PositionedGlyph,
    fonts: &FontStore,
    face_index: usize,
    page_height_pt: f32,
) {
    place_glyph(content, line, glyph, page_height_pt);
    let code = fonts.remap(face_index, glyph.glyph_id);
    content.show(Str(&code.to_be_bytes()));
}

fn place_glyph(content: &mut Content, line: &Line, glyph: &PositionedGlyph, page_height_pt: f32) {
    let x = px_to_pt(line.frag.x + glyph.x);
    let y = flip_y(line.frag.y + glyph.y, page_height_pt);
    content.set_text_matrix([1.0, 0.0, 0.0, 1.0, x, y]);
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
