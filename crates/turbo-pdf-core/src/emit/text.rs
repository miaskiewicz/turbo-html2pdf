//! Text showing (§7, AC-7.2). A `TextLine` fragment becomes one text object: we
//! select the line's subset font, then place each shaped glyph at its own
//! position with a text matrix and show its 2-byte `Identity-H` code (the
//! subset-remapped glyph id). Per-glyph placement means we never depend on the
//! embedded advances matching the galley's shaped positions — the layout already
//! decided every glyph's spot.

use pdf_writer::{Content, Name, Str};

use crate::layout::fragment::{Fragment, FragmentContent, PositionedGlyph};
use crate::layout::value::Rgba;
use crate::text::FontFace;

use super::color::device_rgb;
use super::fonts::FontStore;
use super::unit::{flip_y, px_to_pt};

/// Paint a `TextLine` fragment as a PDF text object. No-op for any other
/// fragment content (boxes/directives are handled elsewhere).
pub fn paint_text(content: &mut Content, frag: &Fragment, fonts: &FontStore, page_height_pt: f32) {
    if let FragmentContent::TextLine {
        glyphs,
        face,
        font_size,
        color,
    } = &frag.content
    {
        let line = Line {
            frag,
            glyphs,
            face,
            font_size: *font_size,
            color: *color,
        };
        show_line(content, &line, fonts, page_height_pt);
    }
}

/// One text line resolved from its fragment, ready to show.
struct Line<'a> {
    frag: &'a Fragment,
    glyphs: &'a [PositionedGlyph],
    face: &'a FontFace,
    font_size: f32,
    color: Rgba,
}

fn show_line(content: &mut Content, line: &Line, fonts: &FontStore, page_height_pt: f32) {
    let face_index = fonts.index_of(line.face);
    let resource = FontStore::resource_name(face_index);
    let rgb = device_rgb(line.color);
    content.begin_text();
    content.set_font(Name(resource.as_bytes()), px_to_pt(line.font_size));
    content.set_fill_rgb(rgb.r, rgb.g, rgb.b);
    for glyph in line.glyphs {
        show_glyph(content, line, glyph, fonts, face_index, page_height_pt);
    }
    content.end_text();
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
    let x = px_to_pt(line.frag.x + glyph.x);
    let y = flip_y(line.frag.y + glyph.y, page_height_pt);
    content.set_text_matrix([1.0, 0.0, 0.0, 1.0, x, y]);
    let code = fonts.remap(face_index, glyph.glyph_id);
    content.show(Str(&code.to_be_bytes()));
}
