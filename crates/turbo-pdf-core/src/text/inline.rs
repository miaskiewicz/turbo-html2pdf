//! Inline/text layout (§5.2): shape with the face, find break opportunities via
//! `unicode-linebreak`, and greedily wrap into lines with computed metrics and
//! alignment. Justified spacing distribution, hyphenation, bidi/RTL, and
//! per-glyph font fallback within a line are deferred to later refinements.

use unicode_linebreak::{linebreaks, BreakOpportunity};

use super::font::FontFace;

/// CSS `white-space` handling (subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WhiteSpace {
    /// Collapse runs of whitespace and wrap at break opportunities.
    #[default]
    Normal,
    /// Preserve whitespace and line breaks; do not wrap.
    Pre,
    /// Collapse whitespace but never wrap.
    NoWrap,
}

/// CSS `text-align` (subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Right,
    Center,
    Justify,
}

/// The text properties needed to lay out a run.
#[derive(Debug, Clone, Copy)]
pub struct TextStyle {
    pub font_size: f32,
    pub line_height: Option<f32>,
    pub letter_spacing: f32,
    pub align: Align,
    pub white_space: WhiteSpace,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            font_size: 16.0,
            line_height: None,
            letter_spacing: 0.0,
            align: Align::Left,
            white_space: WhiteSpace::Normal,
        }
    }
}

/// One laid-out line.
#[derive(Debug, Clone, PartialEq)]
pub struct LineBox {
    pub text: String,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub height: f32,
    pub x_offset: f32,
}

fn collapse_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn align_offset(align: Align, width: f32, max_width: f32) -> f32 {
    match align {
        Align::Left | Align::Justify => 0.0,
        Align::Right => (max_width - width).max(0.0),
        Align::Center => ((max_width - width) / 2.0).max(0.0),
    }
}

fn make_line(text: String, face: &FontFace, style: &TextStyle, max_width: f32) -> LineBox {
    let trimmed = text.trim_end().to_string();
    let width = face.measure(&trimmed, style.font_size, style.letter_spacing);
    let height = style
        .line_height
        .unwrap_or_else(|| face.line_height_px(style.font_size));
    let x_offset = align_offset(style.align, width, max_width);
    LineBox {
        text: trimmed,
        width,
        ascent: face.ascent_px(style.font_size),
        descent: face.descent_px(style.font_size),
        height,
        x_offset,
    }
}

fn segments(text: &str) -> Vec<(&str, bool)> {
    let mut out = Vec::new();
    let mut prev = 0;
    for (idx, op) in linebreaks(text) {
        out.push((&text[prev..idx], op == BreakOpportunity::Mandatory));
        prev = idx;
    }
    out
}

fn flush(
    cur: &mut String,
    cur_w: &mut f32,
    face: &FontFace,
    style: &TextStyle,
    max_width: f32,
    lines: &mut Vec<LineBox>,
) {
    let line = std::mem::take(cur);
    *cur_w = 0.0;
    if !line.trim().is_empty() {
        lines.push(make_line(line, face, style, max_width));
    }
}

struct WrapState {
    cur: String,
    cur_w: f32,
}

fn append_segment(
    seg: &str,
    mandatory: bool,
    face: &FontFace,
    style: &TextStyle,
    max_width: f32,
    st: &mut WrapState,
    lines: &mut Vec<LineBox>,
) {
    let seg_w = face.measure(seg, style.font_size, style.letter_spacing);
    if !st.cur.is_empty() && st.cur_w + seg_w > max_width {
        flush(&mut st.cur, &mut st.cur_w, face, style, max_width, lines);
    }
    st.cur.push_str(seg);
    st.cur_w += seg_w;
    if mandatory {
        flush(&mut st.cur, &mut st.cur_w, face, style, max_width, lines);
    }
}

fn wrap_normal(text: &str, face: &FontFace, style: &TextStyle, max_width: f32) -> Vec<LineBox> {
    let collapsed = collapse_ws(text);
    let mut lines = Vec::new();
    let mut st = WrapState {
        cur: String::new(),
        cur_w: 0.0,
    };
    for (seg, mandatory) in segments(&collapsed) {
        append_segment(seg, mandatory, face, style, max_width, &mut st, &mut lines);
    }
    flush(
        &mut st.cur,
        &mut st.cur_w,
        face,
        style,
        max_width,
        &mut lines,
    );
    lines
}

fn layout_pre(text: &str, face: &FontFace, style: &TextStyle, max_width: f32) -> Vec<LineBox> {
    text.split('\n')
        .map(|line| make_line(line.to_string(), face, style, max_width))
        .collect()
}

fn layout_nowrap(text: &str, face: &FontFace, style: &TextStyle, max_width: f32) -> Vec<LineBox> {
    let collapsed = collapse_ws(text);
    if collapsed.is_empty() {
        return Vec::new();
    }
    vec![make_line(collapsed, face, style, max_width)]
}

/// Lay out `text` with `face`/`style` into lines fitting `max_width` pixels.
pub fn layout_text(text: &str, face: &FontFace, style: &TextStyle, max_width: f32) -> Vec<LineBox> {
    match style.white_space {
        WhiteSpace::Pre => layout_pre(text, face, style, max_width),
        WhiteSpace::NoWrap => layout_nowrap(text, face, style, max_width),
        WhiteSpace::Normal => wrap_normal(text, face, style, max_width),
    }
}
