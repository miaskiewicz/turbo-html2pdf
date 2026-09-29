//! Multi-run inline layout (§5.2, AC-5.4). Generalizes `text::layout_text` (one
//! run) to a paragraph of styled runs: it itemizes each run into shaping segments
//! by per-glyph fallback face (`.notdef` + [`LintCode::NotdefGlyph`] when no face
//! covers a char), wraps words greedily across run boundaries, baseline-aligns
//! mixed font sizes, and applies `vertical-align`.
//!
//! Deferred (per §2): justified spacing (`Align::Justify` lays out left), Knuth-
//! Liang hyphenation, bidi/RTL, and intra-word break opportunities beyond
//! whitespace (CJK). Words break at whitespace only in v1.

use crate::error::{Diagnostics, LintCode, Span};
use crate::text::{Align, FontFace, FontRegistry};

use super::fragment::{NodeId, PositionedGlyph};
use super::value::{Rgba, VAlign};

/// One styled run of text entering inline layout. `face` is the run's primary
/// face (already selected); `families`/`weight`/`italic` drive per-glyph fallback.
#[derive(Debug, Clone)]
pub struct InlineRun {
    pub node_id: NodeId,
    pub text: String,
    pub face: FontFace,
    pub families: Vec<String>,
    pub weight: u16,
    pub italic: bool,
    pub font_size: f32,
    pub line_height: Option<f32>,
    pub letter_spacing: f32,
    pub color: Rgba,
    pub valign: VAlign,
    /// `white-space:nowrap`/`pre` — the run's inter-word spaces are not line-break
    /// opportunities, so its text stays on one line (menu tabs, buttons, nowrap
    /// labels). Modeled by folding those spaces into the word rather than emitting a
    /// breakable gap.
    pub nowrap: bool,
}

/// An atomic inline box (an `inline-block` or replaced `<img>`) that flows within
/// the line as one unbreakable unit of the given border-box size. The caller lays
/// the box out itself; `id` maps a placement back to that laid fragment.
#[derive(Debug, Clone, Copy)]
pub struct InlineAtom {
    pub id: usize,
    pub width: f32,
    pub height: f32,
    /// The box's `vertical-align`, driving where it sits in the line box: `Top`
    /// pins its top to the line top, `Bottom` its bottom to the line bottom,
    /// `Middle` centres it, `Super`/`Sub` shift it off the baseline (by
    /// [`valign_shift`] of `font_size`), else it sits on the baseline (the default).
    pub valign: VAlign,
    /// The box's font size — the basis for the `Super`/`Sub` baseline shift (a
    /// browser's superscript/subscript offset scales with the font in effect).
    pub font_size: f32,
    /// The box's own margins (border box → margin box). An inline-block reserves
    /// its margin box in the line: horizontal margins add to its inline advance,
    /// and vertical margins grow the line and offset the border box within it (a
    /// baseline atom sits with its *margin* box bottom on the baseline).
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub margin_right: f32,
}

impl InlineAtom {
    /// The atom's margin-box height (border box + vertical margins) — what it
    /// contributes to the line box.
    fn outer_height(&self) -> f32 {
        self.height + self.margin_top + self.margin_bottom
    }
}

/// One inline-level piece in document order: a styled text run, an atomic box, or
/// a forced line break (`<br>`).
///
/// A `<br>` is a pure forced break: it ends the current line and continues on the
/// next, but contributes **no metrics of its own** (a browser ignores a `<br>`'s
/// own `line-height`). Each line's height instead comes from its content plus the
/// block's *strut* (see [`layout_paragraph_in`]), so the break advances by the
/// line-height in effect on the terminated line and an empty line from `<br><br>`
/// is exactly one strut tall — not a block of fixed height.
#[derive(Debug, Clone)]
pub enum Piece {
    Run(InlineRun),
    Atom(InlineAtom),
    Break,
}

/// Where an atom landed: its `id` and top-left relative to the line's top-left.
#[derive(Debug, Clone, Copy)]
pub struct PlacedAtom {
    pub id: usize,
    pub x: f32,
    pub y: f32,
}

/// A contiguous run of glyphs sharing a face/size/color, positioned relative to
/// the line's top-left.
#[derive(Debug, Clone)]
pub struct GlyphRun {
    pub node_id: NodeId,
    pub glyphs: Vec<PositionedGlyph>,
    pub face: FontFace,
    pub font_size: f32,
    pub color: Rgba,
}

/// One laid-out line: its glyph runs, atom placements, used width, top offset,
/// and box height.
#[derive(Debug, Clone)]
pub struct InlineLine {
    pub runs: Vec<GlyphRun>,
    pub atoms: Vec<PlacedAtom>,
    pub width: f32,
    pub top: f32,
    pub height: f32,
    /// Horizontal offset of the line box from the paragraph's left edge — nonzero
    /// when a `float:left` (or the free region between floats) pushes this line's
    /// start to the right. Its available width shrinks by the same floats.
    pub indent: f32,
}

/// The result of laying out a paragraph of runs into a column of `max_width`.
#[derive(Debug, Clone)]
pub struct ParagraphLayout {
    pub lines: Vec<InlineLine>,
    pub width: f32,
    pub height: f32,
}

// --------------------------------------------------------------------------
// itemization: chars -> per-glyph fallback faces
// --------------------------------------------------------------------------

struct CharInfo {
    ch: char,
    run: usize,
    face: FontFace,
}

fn resolve_face(ch: char, run: &InlineRun, reg: &FontRegistry) -> (FontFace, bool) {
    if run.face.has_glyph(ch) {
        return (run.face.clone(), false);
    }
    let families: Vec<&str> = run.families.iter().map(String::as_str).collect();
    match reg.resolve_glyph(&families, run.weight, run.italic, ch) {
        Some(face) => (face.clone(), false),
        None => (run.face.clone(), true),
    }
}

fn flatten_chars(runs: &[InlineRun], reg: &FontRegistry, diags: &mut Diagnostics) -> Vec<CharInfo> {
    let mut out = Vec::new();
    for (i, run) in runs.iter().enumerate() {
        let mut missing = false;
        for ch in run.text.chars() {
            let (face, notdef) = resolve_face(ch, run, reg);
            missing |= notdef;
            out.push(CharInfo { ch, run: i, face });
        }
        if missing {
            diags.push(
                LintCode::NotdefGlyph,
                "glyph missing from all fonts",
                Span::default(),
            );
        }
    }
    out
}

// --------------------------------------------------------------------------
// words & segments
// --------------------------------------------------------------------------

struct Seg {
    face: FontFace,
    text: String,
    font_size: f32,
    color: Rgba,
    valign: VAlign,
    letter_spacing: f32,
    line_height: Option<f32>,
    node_id: NodeId,
    width: f32,
}

struct Word {
    segs: Vec<Seg>,
    width: f32,
    space_after: f32,
    /// An atomic inline box occupying this word's slot (its `segs` are empty).
    atom: Option<InlineAtom>,
    /// A forced line break (`<br>`): a zero-width, metric-less word (empty `segs`,
    /// no `atom`) that flushes the current line once placed on it.
    break_after: bool,
}

fn seg_key(c: &CharInfo) -> (usize, String, u16, bool) {
    (
        c.run,
        c.face.family().to_string(),
        c.face.weight(),
        c.face.is_italic(),
    )
}

fn make_seg(chars: &[CharInfo], runs: &[InlineRun]) -> Seg {
    let text: String = chars.iter().map(|c| c.ch).collect();
    let run = &runs[chars[0].run];
    let face = chars[0].face.clone();
    let width = face.measure(&text, run.font_size, run.letter_spacing);
    Seg {
        face,
        text,
        font_size: run.font_size,
        color: run.color,
        valign: run.valign,
        letter_spacing: run.letter_spacing,
        line_height: run.line_height,
        node_id: run.node_id,
        width,
    }
}

fn make_word(chars: &[CharInfo], runs: &[InlineRun], space_after: f32) -> Word {
    let mut segs = Vec::new();
    let mut idx = 0;
    while idx < chars.len() {
        let start = idx;
        let key = seg_key(&chars[idx]);
        while idx < chars.len() && seg_key(&chars[idx]) == key {
            idx += 1;
        }
        segs.push(make_seg(&chars[start..idx], runs));
    }
    let width = segs.iter().map(|s| s.width).sum();
    Word {
        segs,
        width,
        space_after,
        atom: None,
        break_after: false,
    }
}

/// A zero-width forced-break word (`<br>`): no glyphs, flushes its line.
fn break_word() -> Word {
    Word {
        segs: Vec::new(),
        width: 0.0,
        space_after: 0.0,
        atom: None,
        break_after: true,
    }
}

fn space_width(c: &CharInfo, runs: &[InlineRun]) -> f32 {
    let run = &runs[c.run];
    run.face.measure(" ", run.font_size, run.letter_spacing)
}

fn build_words(chars: &[CharInfo], runs: &[InlineRun]) -> Vec<Word> {
    // A space is a line-break opportunity only when its run wraps; a `nowrap` run's
    // spaces stay inside the word so its whole text is one unbreakable unit.
    let breakable = |c: &CharInfo| c.ch.is_whitespace() && !runs[c.run].nowrap;
    let mut words = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if breakable(&chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !breakable(&chars[i]) {
            i += 1;
        }
        let space_after = chars.get(i).map_or(0.0, |c| space_width(c, runs));
        words.push(make_word(&chars[start..i], runs, space_after));
    }
    words
}

/// Build the word list from inline pieces in document order: consecutive text
/// runs are grouped (so a word may still span adjacent runs, e.g. `<b>x</b>y`),
/// and each atom becomes an unbreakable atom-word at its position.
fn build_pieces(pieces: &[Piece], reg: &FontRegistry, diags: &mut Diagnostics) -> Vec<Word> {
    let mut words = Vec::new();
    let mut group: Vec<InlineRun> = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Run(run) => group.push(run.clone()),
            Piece::Atom(atom) => {
                flush_run_group(&mut group, reg, diags, &mut words);
                words.push(Word {
                    segs: Vec::new(),
                    // The atom occupies its MARGIN box inline, so horizontal margins
                    // count toward wrapping + advance.
                    width: atom.width + atom.margin_left + atom.margin_right,
                    space_after: 0.0,
                    atom: Some(*atom),
                    break_after: false,
                });
            }
            Piece::Break => {
                flush_run_group(&mut group, reg, diags, &mut words);
                words.push(break_word());
            }
        }
    }
    flush_run_group(&mut group, reg, diags, &mut words);
    words
}

/// Flush a run of consecutive text pieces into words (itemize + word-break),
/// clearing the group.
fn flush_run_group(
    group: &mut Vec<InlineRun>,
    reg: &FontRegistry,
    diags: &mut Diagnostics,
    out: &mut Vec<Word>,
) {
    if !group.is_empty() {
        let chars = flatten_chars(group, reg, diags);
        hand_off_leading_space(&chars, group, out);
        out.extend(build_words(&chars, group));
        group.clear();
    }
}

/// Whitespace that opens a text group directly follows a preceding word —
/// typically an atom (`<span-ib> <span-ib>`) whose own `space_after` is 0, or an
/// atom before text (`<img> caption`). `build_words` drops leading whitespace, so
/// hand its collapsed space to that preceding word; otherwise the ~4px
/// inter-inline-block gap a browser paints would vanish.
fn hand_off_leading_space(chars: &[CharInfo], group: &[InlineRun], out: &mut [Word]) {
    let (Some(first), Some(last)) = (chars.first(), out.last_mut()) else {
        return;
    };
    let breakable = first.ch.is_whitespace() && !group[first.run].nowrap;
    if breakable && last.space_after == 0.0 {
        last.space_after = space_width(first, group);
    }
}

// --------------------------------------------------------------------------
// line breaking
// --------------------------------------------------------------------------

/// A line region query: given a line's top (relative to the paragraph origin),
/// return the `(indent, available_width)` the floats in the current BFC leave
/// free for that line. The default (no floats) is `(0, max_width)` for every y.
pub type LineRegion<'a> = &'a dyn Fn(f32) -> (f32, f32);

/// Running state of line breaking: the accumulated `cur` words, the current line
/// `y`, the float region `(indent, lw)` at that y, and the pen `x`. Threads `y`
/// top-down so each line's available width is queried from `region` at its own
/// vertical position — text wraps in the narrow column beside a float, then widens
/// once past its bottom — and flushes eagerly at a forced break (`<br>`).
struct Placer<'a> {
    align: Align,
    region: LineRegion<'a>,
    /// The block's line-box strut height — the minimum height of every line box,
    /// so an empty line (from consecutive `<br>`) is exactly one strut tall.
    strut: f32,
    out: Vec<InlineLine>,
    cur: Vec<Word>,
    y: f32,
    indent: f32,
    lw: f32,
    x: f32,
}

impl<'a> Placer<'a> {
    fn new(align: Align, region: LineRegion<'a>, strut: f32) -> Self {
        let (indent, lw) = region(0.0);
        Placer {
            align,
            region,
            strut,
            out: Vec::new(),
            cur: Vec::new(),
            y: 0.0,
            indent,
            lw,
            x: 0.0,
        }
    }

    /// Place the accumulated words into a line at the current `y`, advance `y` by
    /// its height, and re-query the float region for the next line.
    fn flush(&mut self) {
        if self.cur.is_empty() {
            return;
        }
        let mut line = place_line(
            std::mem::take(&mut self.cur),
            self.lw,
            self.align,
            self.strut,
        );
        line.top = self.y;
        line.indent = self.indent;
        self.y += line.height;
        self.out.push(line);
        (self.indent, self.lw) = (self.region)(self.y);
        self.x = 0.0;
    }

    /// Add one word: wrap first if it no longer fits, then flush after it if it is
    /// a forced break (so a `<br>` ends its line and the next word starts below).
    fn add(&mut self, word: Word) {
        if !self.cur.is_empty() && self.x + word.width > self.lw {
            self.flush();
        }
        let forced = word.break_after;
        self.x += word.width + word.space_after;
        self.cur.push(word);
        if forced {
            self.flush();
        }
    }

    fn finish(mut self) -> Vec<InlineLine> {
        self.flush();
        self.out
    }
}

/// Break `words` into placed lines via a [`Placer`].
fn wrap_and_place(
    words: Vec<Word>,
    align: Align,
    region: LineRegion,
    strut: f32,
) -> Vec<InlineLine> {
    let mut placer = Placer::new(align, region, strut);
    for word in words {
        placer.add(word);
    }
    placer.finish()
}

// --------------------------------------------------------------------------
// line placement & metrics
// --------------------------------------------------------------------------

fn valign_shift(v: VAlign, size: f32) -> f32 {
    match v {
        VAlign::Super => size * 0.33,
        VAlign::Sub => -size * 0.2,
        _ => 0.0,
    }
}

fn align_offset(align: Align, width: f32, max_width: f32) -> f32 {
    match align {
        Align::Left | Align::Justify => 0.0,
        Align::Right => (max_width - width).max(0.0),
        Align::Center => ((max_width - width) / 2.0).max(0.0),
    }
}

struct Metrics {
    ascent: f32,
    descent: f32,
    height: f32,
}

fn fold_seg_metrics(seg: &Seg, m: &mut Metrics) {
    let shift = valign_shift(seg.valign, seg.font_size);
    m.ascent = m.ascent.max(seg.face.ascent_px(seg.font_size) + shift);
    m.descent = m.descent.max(seg.face.descent_px(seg.font_size) - shift);
    let lh = seg
        .line_height
        .unwrap_or_else(|| seg.face.line_height_px(seg.font_size));
    m.height = m.height.max(lh);
}

/// Fold an atom's contribution into the line metrics. A baseline-aligned atom sits
/// with its bottom on the baseline (the CSS default for a replaced/empty
/// inline-block), so it contributes its full height as ascent and grows the line
/// above the baseline. A `top`/`bottom` atom is placed against the line box edges
/// instead, so it only grows the line's total height — folding it into `ascent`
/// would shove the baseline (and every other item) down under a tall top box.
fn fold_atom_metrics(atom: &InlineAtom, m: &mut Metrics) {
    let outer = atom.outer_height();
    if matches!(atom.valign, VAlign::Top | VAlign::Bottom) {
        m.height = m.height.max(outer);
    } else {
        m.ascent = m.ascent.max(outer);
        m.height = m.height.max(outer);
    }
}

/// The line-box metrics: the block `strut` seeds the height (so an empty line — a
/// `<br><br>` gap — is exactly one strut tall), then each word's text segments and
/// atoms grow it. Text ascent/descent still floor the height via the final `max`,
/// as a line box is never shorter than its own content.
fn line_metrics(words: &[Word], strut: f32) -> Metrics {
    let mut m = Metrics {
        ascent: 0.0,
        descent: 0.0,
        height: strut,
    };
    for word in words {
        for seg in &word.segs {
            fold_seg_metrics(seg, &mut m);
        }
        if let Some(atom) = word.atom {
            fold_atom_metrics(&atom, &mut m);
        }
    }
    m.height = m.height.max(m.ascent + m.descent);
    m
}

fn line_used_width(words: &[Word]) -> f32 {
    let mut w = 0.0;
    for (i, word) in words.iter().enumerate() {
        w += word.width;
        if i + 1 < words.len() {
            w += word.space_after;
        }
    }
    w
}

fn shape_seg(seg: &Seg, pen_x: f32, baseline: f32) -> GlyphRun {
    let scale = seg.face.scale(seg.font_size);
    let seg_baseline = baseline - valign_shift(seg.valign, seg.font_size);
    let mut glyphs = Vec::new();
    let mut x = pen_x;
    for g in seg.face.shape(&seg.text) {
        glyphs.push(PositionedGlyph {
            glyph_id: g.glyph_id,
            x: x + scale * g.x_offset as f32,
            y: seg_baseline - scale * g.y_offset as f32,
        });
        x += scale * g.x_advance as f32 + seg.letter_spacing;
    }
    GlyphRun {
        node_id: seg.node_id,
        glyphs,
        face: seg.face.clone(),
        font_size: seg.font_size,
        color: seg.color,
    }
}

fn same_face(a: &FontFace, b: &FontFace) -> bool {
    a.family() == b.family() && a.weight() == b.weight() && a.is_italic() == b.is_italic()
}

fn can_merge(a: &GlyphRun, b: &GlyphRun) -> bool {
    a.node_id == b.node_id
        && a.font_size == b.font_size
        && a.color == b.color
        && same_face(&a.face, &b.face)
}

fn merge_runs(runs: Vec<GlyphRun>) -> Vec<GlyphRun> {
    let mut out: Vec<GlyphRun> = Vec::new();
    for run in runs {
        match out.last_mut() {
            Some(last) if can_merge(last, &run) => last.glyphs.extend(run.glyphs),
            _ => out.push(run),
        }
    }
    out
}

/// The margin-box top of an atom within its line box, per `vertical-align`: `top`
/// pins to the line top, `bottom` to the line bottom, `middle` centres, `super`/
/// `sub` shift the baseline placement off the baseline by [`valign_shift`], else
/// the margin box's bottom sits on the baseline (the CSS default). The border box
/// is then its `margin-top` below the returned margin-box top.
fn atom_margin_top(atom: &InlineAtom, height: f32, baseline: f32) -> f32 {
    let outer = atom.outer_height();
    match atom.valign {
        VAlign::Top => 0.0,
        VAlign::Bottom => height - outer,
        VAlign::Middle => (height - outer) / 2.0,
        VAlign::Super | VAlign::Sub => baseline - outer - valign_shift(atom.valign, atom.font_size),
        _ => baseline - outer,
    }
}

fn place_line(words: Vec<Word>, max_width: f32, align: Align, strut: f32) -> InlineLine {
    let m = line_metrics(&words, strut);
    let baseline = m.ascent + (m.height - m.ascent - m.descent) / 2.0;
    let width = line_used_width(&words);
    let mut pen = align_offset(align, width, max_width);
    let mut runs = Vec::new();
    let mut atoms = Vec::new();
    for word in &words {
        if let Some(atom) = word.atom {
            let margin_top = atom_margin_top(&atom, m.height, baseline);
            atoms.push(PlacedAtom {
                id: atom.id,
                x: pen + atom.margin_left,
                y: margin_top + atom.margin_top,
            });
            pen += atom.width + atom.margin_left + atom.margin_right;
        } else {
            for seg in &word.segs {
                runs.push(shape_seg(seg, pen, baseline));
                pen += seg.width;
            }
        }
        pen += word.space_after;
    }
    InlineLine {
        runs: merge_runs(runs),
        atoms,
        width,
        top: 0.0,
        height: m.height,
        indent: 0.0,
    }
}

fn finalize(lines: Vec<InlineLine>) -> ParagraphLayout {
    let mut height = 0.0_f32;
    let mut width = 0.0_f32;
    for line in &lines {
        height = height.max(line.top + line.height);
        width = width.max(line.indent + line.width);
    }
    ParagraphLayout {
        lines,
        width,
        height,
    }
}

/// Lay out a paragraph of inline pieces (text runs + atomic boxes, in document
/// order) into lines fitting `max_width` px. Atoms flow within the line and their
/// placements come back on each [`InlineLine::atoms`].
pub fn layout_paragraph(
    pieces: &[Piece],
    reg: &FontRegistry,
    max_width: f32,
    align: Align,
    diags: &mut Diagnostics,
) -> ParagraphLayout {
    // Width-measurement callers ignore line heights, so no strut is needed here.
    layout_paragraph_in(pieces, reg, align, diags, &|_| (0.0, max_width), 0.0)
}

/// [`layout_paragraph`] with a per-line float region — each line's `(indent,
/// width)` comes from `region` at the line's own top, so text wraps beside a
/// float and widens below it — and a block `strut` (the minimum line-box height,
/// the block's own line-height), so an empty line from `<br><br>` is one strut
/// tall. `layout_paragraph` passes a full-width region and a zero strut.
pub fn layout_paragraph_in(
    pieces: &[Piece],
    reg: &FontRegistry,
    align: Align,
    diags: &mut Diagnostics,
    region: LineRegion,
    strut: f32,
) -> ParagraphLayout {
    let words = build_pieces(pieces, reg, diags);
    finalize(wrap_and_place(words, align, region, strut))
}

/// Lay out a paragraph of text runs only (no atoms) — a convenience for callers
/// that measure text width (e.g. flex/grid content sizing).
pub fn layout_runs(
    runs: &[InlineRun],
    reg: &FontRegistry,
    max_width: f32,
    align: Align,
    diags: &mut Diagnostics,
) -> ParagraphLayout {
    let pieces: Vec<Piece> = runs.iter().cloned().map(Piece::Run).collect();
    layout_paragraph(&pieces, reg, max_width, align, diags)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(reg: &FontRegistry) -> FontFace {
        reg.select(&["sans-serif"], 400, false)
            .expect("bundled sans-serif")
            .clone()
    }

    fn run(text: &str, reg: &FontRegistry, line_height: Option<f32>) -> Piece {
        Piece::Run(InlineRun {
            node_id: NodeId(0),
            text: text.to_string(),
            face: face(reg),
            families: vec!["sans-serif".to_string()],
            weight: 400,
            italic: false,
            font_size: 16.0,
            line_height,
            letter_spacing: 0.0,
            color: Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            },
            valign: VAlign::Baseline,
            nowrap: false,
        })
    }

    fn atom(valign: VAlign) -> InlineAtom {
        InlineAtom {
            id: 0,
            width: 10.0,
            height: 10.0,
            valign,
            font_size: 20.0,
            margin_top: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
            margin_right: 0.0,
        }
    }

    // A `<br>` splits the flow into lines; consecutive breaks leave an empty line
    // exactly one strut tall — never a fixed-height phantom block.
    #[test]
    fn forced_break_makes_strut_tall_lines() {
        let reg = FontRegistry::new();
        let mut diags = Diagnostics::default();
        let pieces = [
            run("a", &reg, Some(20.0)),
            Piece::Break,
            Piece::Break,
            run("b", &reg, Some(20.0)),
        ];
        let para = layout_paragraph_in(
            &pieces,
            &reg,
            Align::Left,
            &mut diags,
            &|_| (0.0, 500.0),
            20.0,
        );
        assert_eq!(para.lines.len(), 3, "a / empty / b");
        assert!(para.lines[1].runs.is_empty(), "middle line is empty");
        assert!(
            (para.lines[1].height - 20.0).abs() < 0.01,
            "empty line is one strut"
        );
        assert!((para.height - 60.0).abs() < 0.01, "three 20px lines");
    }

    // A trailing `<br>` ends its line but adds no phantom empty line after it.
    #[test]
    fn trailing_break_adds_no_extra_line() {
        let reg = FontRegistry::new();
        let mut diags = Diagnostics::default();
        let pieces = [run("a", &reg, Some(20.0)), Piece::Break];
        let para = layout_paragraph_in(
            &pieces,
            &reg,
            Align::Left,
            &mut diags,
            &|_| (0.0, 500.0),
            20.0,
        );
        assert_eq!(para.lines.len(), 1);
        assert!((para.height - 20.0).abs() < 0.01);
    }

    // Words wider than the region wrap onto successive lines (the `Placer` wrap arm).
    #[test]
    fn words_wrap_when_they_exceed_the_line() {
        let reg = FontRegistry::new();
        let mut diags = Diagnostics::default();
        let pieces = [run("aaaa bbbb cccc", &reg, Some(20.0))];
        let para = layout_paragraph_in(
            &pieces,
            &reg,
            Align::Left,
            &mut diags,
            &|_| (0.0, 40.0),
            20.0,
        );
        assert!(para.lines.len() >= 2, "narrow column wraps");
    }

    // `vertical-align: super`/`sub` shift an atom off the baseline by `valign_shift`;
    // the other keywords pin it to the line-box top/bottom/middle/baseline.
    #[test]
    fn atom_margin_top_honors_vertical_align() {
        let (h, base) = (30.0, 25.0);
        assert_eq!(atom_margin_top(&atom(VAlign::Top), h, base), 0.0);
        assert_eq!(atom_margin_top(&atom(VAlign::Bottom), h, base), 20.0);
        assert_eq!(atom_margin_top(&atom(VAlign::Middle), h, base), 10.0);
        assert_eq!(atom_margin_top(&atom(VAlign::Baseline), h, base), 15.0);
        // super raises (smaller margin-top), sub lowers, both by valign_shift(fs=20).
        let sup = atom_margin_top(&atom(VAlign::Super), h, base);
        let sub = atom_margin_top(&atom(VAlign::Sub), h, base);
        assert!((sup - (15.0 - 20.0 * 0.33)).abs() < 0.01);
        assert!((sub - (15.0 + 20.0 * 0.2)).abs() < 0.01);
    }

    // The super/sub baseline-shift factors (asserted against Chromium's measured
    // ~0.3383em / 0.2050em by the sub-sup conformance fixture); other aligns: 0.
    #[test]
    fn valign_shift_factors() {
        assert!((valign_shift(VAlign::Super, 100.0) - 33.0).abs() < 0.01);
        assert!((valign_shift(VAlign::Sub, 100.0) + 20.0).abs() < 0.01);
        assert_eq!(valign_shift(VAlign::Middle, 100.0), 0.0);
    }
}
