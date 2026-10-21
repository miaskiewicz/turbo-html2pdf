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
#[path = "inline_advances.rs"]
mod inline_advances;
#[path = "native_text_advance.rs"]
mod native_text_advance;

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
    NativeAtom(InlineAtom, f32),
    SourceBreak(InlineEdge),
    Edge(InlineEdge),
    Run(InlineRun),
    Atom(InlineAtom),
    Break,
}

/// Source-owned inline edge. Vertical decorations do not grow the line box.
#[derive(Debug, Clone)]
pub struct InlineEdge {
    pub node_id: NodeId,
    pub start: bool,
    pub advance: f32,
    pub face: FontFace,
    pub font_size: f32,
    pub line_height: Option<f32>,
    pub baseline_shift: f32,
    pub contributes_metrics: bool,
    pub valign: VAlign,
}
#[derive(Debug, Clone)]
pub struct PlacedInlineEdge {
    pub edge: InlineEdge,
    pub x: f32,
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
    pub inline_edges: Vec<PlacedInlineEdge>,
    pub baseline: f32,
    pub inline_origin: f32,
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

/// Actual shaper cluster and pen advance for one positioned glyph. Byte and
/// UTF-16 ranges refer to its original InlineRun text, not normalized glyph text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphSourceCluster {
    pub byte_start: usize,
    pub byte_end: usize,
    pub utf16_start: usize,
    pub utf16_end: usize,
    pub pen_x: f32,
    pub advance: f32,
    /// The direction selected by the actual native shaper for this segment.
    pub right_to_left: bool,
    /// True for source cells created by CSS whitespace collapse, including the
    /// first rendered space. Pre/nowrap glyph spaces retain ordinary provenance.
    pub collapsed_whitespace: bool,
}

/// Native output and parallel per-line, per-run, per-glyph source records.
#[derive(Debug, Clone)]
pub struct ParagraphSourceLayout {
    pub layout: ParagraphLayout,
    pub clusters: Vec<Vec<Vec<GlyphSourceCluster>>>,
    pub font_boxes: Vec<Vec<SourceFontBox>>,
}
/// A run's real baseline and native HTML font box, relative to its fragment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceFontBox {
    pub baseline: f32,
    pub ascent: f32,
    pub descent: f32,
}

struct SourceRun {
    run: GlyphRun,
    clusters: Vec<GlyphSourceCluster>,
    font_box: SourceFontBox,
}
struct SourceLine {
    line: InlineLine,
    clusters: Vec<Vec<GlyphSourceCluster>>,
    font_boxes: Vec<SourceFontBox>,
}

// --------------------------------------------------------------------------
// itemization: chars -> per-glyph fallback faces
// --------------------------------------------------------------------------

struct CharInfo {
    byte_offset: usize,
    utf16_offset: usize,
    ch: char,
    run: usize,
    face: FontFace,
    original: bool,
    invalid_unit: Option<u16>,
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

// A font may shape a whole combining cluster through normalization even when
// its cmap does not contain an individual mark. Native text snapshots consult
// real primary-font shaping before character fallback; the legacy PDF route
// keeps its existing itemization policy.
fn primary_cluster_coverage(
    run: &InlineRun,
    enabled: bool,
    original: Option<&[u16]>,
) -> Vec<(usize, bool)> {
    if !enabled {
        return Vec::new();
    }
    let mut coverage = std::collections::BTreeMap::new();
    for glyph in original.map_or_else(
        || run.face.shape(&run.text),
        |units| run.face.shape_original_utf16(units),
    ) {
        *coverage.entry(glyph.cluster as usize).or_insert(true) &= glyph.glyph_id != 0;
    }
    coverage.into_iter().collect()
}
fn primary_covers(coverage: &[(usize, bool)], byte: usize) -> bool {
    let index = coverage.partition_point(|value| value.0 <= byte);
    index.checked_sub(1).is_some_and(|index| coverage[index].1)
}

fn flatten_chars(
    runs: &[InlineRun],
    reg: &FontRegistry,
    diags: &mut Diagnostics,
    capture_sources: bool,
    original_units: Option<&crate::layout_snapshot::text_units::NativeUnitMap>,
) -> Vec<CharInfo> {
    let mut out = Vec::new();
    for (i, run) in runs.iter().enumerate() {
        let mut missing = false;
        let original = original_units
            .and_then(|units| units.get(&run.node_id))
            .map(std::sync::Arc::as_ref);
        let coverage =
            primary_cluster_coverage(run, capture_sources || original.is_some(), original);
        let mut native_faces = NativeFallbacks::default();
        let mut utf16_offset = 0;
        for (byte_offset, ch) in run.text.char_indices() {
            let invalid_unit = invalid_source_unit(original, utf16_offset, ch);
            let (face, notdef) = select_character_face(
                CharacterSelection {
                    ch,
                    byte: byte_offset,
                    unit: invalid_unit,
                    native: original.is_some(),
                    run,
                    fonts: reg,
                    coverage: &coverage,
                },
                &mut native_faces,
            );
            missing |= notdef;
            out.push(CharInfo {
                byte_offset,
                utf16_offset,
                ch,
                run: i,
                face,
                original: original.is_some(),
                invalid_unit,
            });
            utf16_offset += ch.len_utf16();
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

struct CharacterSelection<'a> {
    ch: char,
    byte: usize,
    unit: Option<u16>,
    native: bool,
    run: &'a InlineRun,
    fonts: &'a FontRegistry,
    coverage: &'a [(usize, bool)],
}
fn select_character_face(
    selection: CharacterSelection<'_>,
    previous: &mut NativeFallbacks,
) -> (FontFace, bool) {
    let CharacterSelection {
        ch,
        byte,
        unit,
        native,
        run,
        fonts,
        coverage,
    } = selection;
    if native && !primary_covers(coverage, byte) {
        native_character_face(ch, unit, run, fonts, previous)
    } else {
        character_face(ch, byte, run, fonts, coverage)
    }
}

fn invalid_source_unit(original: Option<&[u16]>, offset: usize, character: char) -> Option<u16> {
    if character != char::REPLACEMENT_CHARACTER {
        return None;
    }
    original
        .and_then(|units| units.get(offset))
        .copied()
        .filter(|unit| (0xd800..=0xdfff).contains(unit))
}

#[derive(Default)]
struct NativeFallbacks {
    faces: Vec<FontFace>,
    invalid: Option<(FontFace, bool)>,
}

impl NativeFallbacks {
    fn cached(&self, ch: char, invalid: bool) -> Option<(FontFace, bool)> {
        if invalid {
            self.invalid.clone()
        } else {
            self.faces
                .iter()
                .find(|face| face.has_glyph(ch))
                .map(|face| (face.clone(), false))
        }
    }
    fn retain(&mut self, invalid: bool, selected: &(FontFace, bool)) {
        if invalid {
            self.invalid = Some(selected.clone());
        } else if !selected.1 {
            self.faces.push(selected.0.clone());
        }
    }
}
fn registered_fallback(ch: char, run: &InlineRun, fonts: &FontRegistry) -> Option<FontFace> {
    let (face, missing) = resolve_face(ch, run, fonts);
    (!missing).then_some(face)
}
fn native_character_face(
    ch: char,
    unit: Option<u16>,
    run: &InlineRun,
    fonts: &FontRegistry,
    previous: &mut NativeFallbacks,
) -> (FontFace, bool) {
    // Unknown-script original units do not compete with scalar fallback runs.
    if let Some(face) = previous.cached(ch, unit.is_some()) {
        return face;
    }
    let face = fonts
        .native_fallback(&run.face, ch, run.weight, run.italic)
        .or_else(|| registered_fallback(ch, run, fonts));
    let selected = face
        .map(|face| (face, false))
        .unwrap_or_else(|| resolve_face(ch, run, fonts));
    previous.retain(unit.is_some(), &selected);
    selected
}

fn character_face(
    ch: char,
    byte: usize,
    run: &InlineRun,
    fonts: &FontRegistry,
    coverage: &[(usize, bool)],
) -> (FontFace, bool) {
    if primary_covers(coverage, byte) {
        (run.face.clone(), false)
    } else {
        resolve_face(ch, run, fonts)
    }
}

// --------------------------------------------------------------------------
// words & segments
// --------------------------------------------------------------------------

struct Seg {
    baseline_shift: Option<f32>,
    source_bytes: usize,
    source_utf16: usize,
    face: FontFace,
    primary_face: FontFace,
    original_units: Option<Vec<u16>>,
    text: String,
    font_size: f32,
    color: Rgba,
    valign: VAlign,
    letter_spacing: f32,
    line_height: Option<f32>,
    node_id: NodeId,
    width: f32,
    native_width: f64,
}

struct Word {
    atom_shift: f32,
    spaces: Vec<Seg>,
    leading_spaces: Vec<Seg>,
    advance_space: Option<Seg>,
    native_space: Option<(NodeId, f64)>,
    edges: Vec<(usize, InlineEdge)>,
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
    let original_units = segment_original_units(chars);
    let width = original_units.as_deref().map_or_else(
        || face.measure(&text, run.font_size, run.letter_spacing),
        |units| face.measure_original_utf16(units, run.font_size, run.letter_spacing),
    );
    Seg {
        baseline_shift: None,
        source_bytes: chars[0].byte_offset,
        source_utf16: chars[0].utf16_offset,
        face,
        primary_face: run.face.clone(),
        original_units,
        text,
        font_size: run.font_size,
        color: run.color,
        valign: run.valign,
        letter_spacing: run.letter_spacing,
        line_height: run.line_height,
        node_id: run.node_id,
        width,
        native_width: f64::from(width),
    }
}

fn segment_original_units(chars: &[CharInfo]) -> Option<Vec<u16>> {
    if !chars[0].original {
        return None;
    }
    let mut units = Vec::new();
    for character in chars {
        if let Some(unit) = character.invalid_unit {
            units.push(unit);
        } else {
            units.extend_from_slice(character.ch.encode_utf16(&mut [0; 2]));
        }
    }
    Some(units)
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
        atom_shift: 0.0,
        spaces: Vec::new(),
        leading_spaces: Vec::new(),
        advance_space: None,
        native_space: None,
        edges: Vec::new(),
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
        atom_shift: 0.0,
        spaces: Vec::new(),
        leading_spaces: Vec::new(),
        advance_space: None,
        native_space: None,
        edges: Vec::new(),
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

fn collapsible_space(character: char, capture_sources: bool) -> bool {
    if !capture_sources {
        return character.is_whitespace();
    }
    matches!(character, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}

fn build_words(
    chars: &[CharInfo],
    runs: &[InlineRun],
    capture_sources: bool,
    native_advances: bool,
) -> Vec<Word> {
    // A space is a line-break opportunity only when its run wraps; a `nowrap` run's
    // spaces stay inside the word so its whole text is one unbreakable unit.
    let breakable = |c: &CharInfo| collapsible_space(c.ch, capture_sources) && !runs[c.run].nowrap;
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
        let mut word = make_word(&chars[start..i], runs, space_after);
        word.spaces = native_word_spaces(&chars[i..], runs, native_advances);
        words.push(word);
    }
    words
}

fn native_word_spaces(chars: &[CharInfo], runs: &[InlineRun], enabled: bool) -> Vec<Seg> {
    if !enabled {
        return Vec::new();
    }
    chars
        .iter()
        .take_while(|c| collapsible_space(c.ch, true) && !runs[c.run].nowrap)
        .map(|c| make_seg(std::slice::from_ref(c), runs))
        .collect()
}

#[derive(Clone, Copy)]
struct SourcePolicy {
    capture: bool,
    advances: bool,
}

/// Build words in document order; adjacent runs share a word, while each atom
/// remains unbreakable. Native advance metadata is kept out of PDF callers.
fn build_pieces(
    pieces: &[Piece],
    reg: &FontRegistry,
    diags: &mut Diagnostics,
    policy: SourcePolicy,
    original_units: Option<&crate::layout_snapshot::text_units::NativeUnitMap>,
) -> Vec<Word> {
    let mut words = Vec::new();
    let mut group: Vec<InlineRun> = Vec::new();
    let mut edges: Vec<(usize, InlineEdge)> = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Run(run) => group.push(run.clone()),
            Piece::Edge(edge) => edges.push((group.len(), edge.clone())),
            Piece::Atom(atom) => {
                flush_run_group(
                    &mut group,
                    &mut edges,
                    reg,
                    diags,
                    &mut words,
                    policy,
                    original_units,
                );
                words.push(atom_word(*atom, 0.0));
            }
            Piece::NativeAtom(atom, shift) => {
                flush_run_group(
                    &mut group,
                    &mut edges,
                    reg,
                    diags,
                    &mut words,
                    policy,
                    original_units,
                );
                let mut atom = *atom;
                if matches!(atom.valign, VAlign::Super | VAlign::Sub) {
                    atom.valign = VAlign::Baseline;
                }
                words.push(atom_word(atom, *shift));
            }
            Piece::SourceBreak(edge) => {
                flush_run_group(
                    &mut group,
                    &mut edges,
                    reg,
                    diags,
                    &mut words,
                    policy,
                    original_units,
                );
                let mut word = break_word();
                let mut end = edge.clone();
                end.start = false;
                word.edges = vec![(0, edge.clone()), (0, end)];
                words.push(word);
            }
            Piece::Break => {
                flush_run_group(
                    &mut group,
                    &mut edges,
                    reg,
                    diags,
                    &mut words,
                    policy,
                    original_units,
                );
                words.push(break_word());
            }
        }
    }
    flush_run_group(
        &mut group,
        &mut edges,
        reg,
        diags,
        &mut words,
        policy,
        original_units,
    );
    words
}

fn atom_word(atom: InlineAtom, shift: f32) -> Word {
    Word {
        atom_shift: shift,
        spaces: Vec::new(),
        leading_spaces: Vec::new(),
        advance_space: None,
        native_space: None,
        edges: Vec::new(),
        segs: Vec::new(),
        width: atom.width + atom.margin_left + atom.margin_right,
        space_after: 0.0,
        atom: Some(atom),
        break_after: false,
    }
}
/// Flush a run of consecutive text pieces into words (itemize + word-break),
/// clearing the group.
fn flush_run_group(
    group: &mut Vec<InlineRun>,
    edges: &mut Vec<(usize, InlineEdge)>,
    reg: &FontRegistry,
    diags: &mut Diagnostics,
    out: &mut Vec<Word>,
    policy: SourcePolicy,
    original_units: Option<&crate::layout_snapshot::text_units::NativeUnitMap>,
) {
    if !group.is_empty() || !edges.is_empty() {
        let native = policy.capture || original_units.is_some();
        let chars = flatten_chars(group, reg, diags, native, original_units);
        let continue_space = group_leading_space(&chars, group, edges, out, native, policy);
        if edges.is_empty() {
            out.extend(build_words(&chars, group, native, policy.advances));
        } else {
            out.extend(build_marked_words(
                chars,
                group,
                edges,
                native,
                !continue_space,
            ));
        }
        edges.clear();
        group.clear();
    }
}

fn group_leading_space(
    chars: &[CharInfo],
    group: &[InlineRun],
    edges: &[(usize, InlineEdge)],
    out: &mut [Word],
    capture: bool,
    policy: SourcePolicy,
) -> bool {
    if !policy.advances {
        hand_off_leading_space(chars, group, out, capture, false);
        return false;
    }
    let continuation = preceding_inline_content(out);
    if continuation && !edges.first().is_some_and(|(at, _)| *at == 0) {
        hand_off_leading_space(chars, group, out, capture, true);
    }
    continuation
}

fn contains_inline_content(word: &Word) -> bool {
    word.atom.is_some() || !word.segs.is_empty()
}
// Empty inline boundaries do not reset whitespace collapse, but forced line
// breaks and an already captured trailing space do.
fn preceding_inline_content(words: &[Word]) -> bool {
    for word in words.iter().rev() {
        if word.break_after || word.space_after != 0.0 {
            return false;
        }
        if contains_inline_content(word) {
            return true;
        }
    }
    false
}

enum SourceItem {
    Character(CharInfo),
    Edge(InlineEdge),
}
fn marked_items(
    chars: Vec<CharInfo>,
    runs: &[InlineRun],
    edges: &[(usize, InlineEdge)],
) -> Vec<SourceItem> {
    let mut chars = chars.into_iter().peekable();
    let mut events = edges.iter().peekable();
    let mut items = Vec::new();
    for run in 0..=runs.len() {
        while events.peek().is_some_and(|(at, _)| *at == run) {
            items.push(SourceItem::Edge(events.next().unwrap().1.clone()));
        }
        while chars.peek().is_some_and(|c| c.run == run) {
            items.push(SourceItem::Character(chars.next().unwrap()));
        }
    }
    items
}
struct MarkedWords<'a> {
    words: Vec<Word>,
    current: Vec<SourceItem>,
    in_space: bool,
    runs: &'a [InlineRun],
    native: bool,
}
impl MarkedWords<'_> {
    fn push(&mut self, item: SourceItem) {
        match item {
            SourceItem::Character(c)
                if collapsible_space(c.ch, self.native) && !self.runs[c.run].nowrap =>
            {
                self.space(c)
            }
            item => {
                if matches!(item, SourceItem::Character(_)) {
                    self.in_space = false;
                }
                self.current.push(item);
            }
        }
    }
    fn space(&mut self, c: CharInfo) {
        let width = if self.in_space {
            0.0
        } else {
            space_width(&c, self.runs)
        };
        if !self.current.is_empty() {
            self.words.push(marked_word(
                std::mem::take(&mut self.current),
                self.runs,
                width,
            ));
        }
        if let Some(previous) = self.words.last_mut() {
            previous
                .spaces
                .push(make_seg(std::slice::from_ref(&c), self.runs));
        }
        self.in_space = true;
    }
    fn finish(mut self) -> Vec<Word> {
        if !self.current.is_empty() {
            self.words.push(marked_word(self.current, self.runs, 0.0));
        }
        self.words
    }
}
fn build_marked_words(
    chars: Vec<CharInfo>,
    runs: &[InlineRun],
    edges: &[(usize, InlineEdge)],
    native: bool,
    in_space: bool,
) -> Vec<Word> {
    let mut builder = MarkedWords {
        words: Vec::new(),
        current: Vec::new(),
        in_space,
        runs,
        native,
    };
    for item in marked_items(chars, runs, edges) {
        builder.push(item);
    }
    builder.finish()
}
fn push_character(c: CharInfo, chars: &mut Vec<CharInfo>, segs: &mut Vec<Seg>, runs: &[InlineRun]) {
    if chars.first().is_some_and(|old| seg_key(old) != seg_key(&c)) {
        segs.push(make_seg(chars, runs));
        chars.clear();
    }
    chars.push(c);
}
fn marked_word(items: Vec<SourceItem>, runs: &[InlineRun], space_after: f32) -> Word {
    let mut segs = Vec::new();
    let mut edges = Vec::new();
    let mut chars = Vec::new();
    for item in items {
        match item {
            SourceItem::Edge(edge) => {
                if !chars.is_empty() {
                    segs.push(make_seg(&chars, runs));
                    chars.clear();
                }
                edges.push((segs.len(), edge));
            }
            SourceItem::Character(c) => push_character(c, &mut chars, &mut segs, runs),
        }
    }
    if !chars.is_empty() {
        segs.push(make_seg(&chars, runs));
    }
    let width = segs.iter().map(|s| s.width).sum::<f32>()
        + edges.iter().map(|(_, e)| e.advance).sum::<f32>();
    Word {
        atom_shift: 0.0,
        spaces: Vec::new(),
        leading_spaces: Vec::new(),
        advance_space: None,
        native_space: None,
        segs,
        edges,
        width,
        space_after,
        atom: None,
        break_after: false,
    }
}

/// Whitespace that opens a text group directly follows a preceding word —
/// typically an atom (`<span-ib> <span-ib>`) whose own `space_after` is 0, or an
/// atom before text (`<img> caption`). `build_words` drops leading whitespace, so
/// hand its collapsed space to that preceding word; otherwise the ~4px
/// inter-inline-block gap a browser paints would vanish.
fn hand_off_leading_space(
    chars: &[CharInfo],
    group: &[InlineRun],
    out: &mut [Word],
    capture_sources: bool,
    native_advances: bool,
) {
    let (Some(first), Some(last)) = (chars.first(), out.last_mut()) else {
        return;
    };
    let breakable = collapsible_space(first.ch, capture_sources) && !group[first.run].nowrap;
    if breakable && last.space_after == 0.0 {
        last.space_after = space_width(first, group);
        if native_advances {
            last.advance_space = Some(make_seg(std::slice::from_ref(first), group));
            last.spaces.extend(
                chars
                    .iter()
                    .take_while(|c| {
                        collapsible_space(c.ch, capture_sources) && !group[c.run].nowrap
                    })
                    .map(|c| make_seg(std::slice::from_ref(c), group)),
            );
        }
    }
}

// --------------------------------------------------------------------------
// line breaking
// --------------------------------------------------------------------------

/// A line region query: given a line's top (relative to the paragraph origin),
/// return the `(indent, available_width)` the floats in the current BFC leave
/// free for that line. The default (no floats) is `(0, max_width)` for every y.
pub type LineRegion<'a> = &'a dyn Fn(f32) -> (f32, f32);

fn opening_space_only(word: &Word) -> bool {
    word.width == 0.0 && word.atom.is_none() && word.segs.is_empty() && opening_edges(&word.edges)
}
fn opening_edges(edges: &[(usize, InlineEdge)]) -> bool {
    !edges.is_empty() && edges.iter().all(|(_, edge)| edge.start)
}
fn atom_source_prefix(previous: &Word, next: &Word) -> bool {
    previous.atom.is_some()
        && previous
            .native_space
            .is_some_and(|(source, _)| next.segs.first().is_some_and(|seg| seg.node_id == source))
}
fn wrapped_source_spaces(previous: &mut Word, next: &Word) -> Vec<Seg> {
    if opening_space_only(previous) || atom_source_prefix(previous, next) {
        return std::mem::take(&mut previous.spaces);
    }
    if wrapped_extra_spaces(previous, next) {
        // The first separator retains the preceding line's end affinity; extra
        // collapsed units map to the next line's start, retaining their source IDs.
        return previous.spaces.split_off(1);
    }
    Vec::new()
}
fn wrapped_extra_spaces(previous: &Word, next: &Word) -> bool {
    previous.spaces.len() > 1
        && !previous.segs.is_empty()
        && previous
            .spaces
            .first()
            .zip(next.segs.first())
            .is_some_and(|(space, next)| space.node_id == next.node_id)
}
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
    capture_sources: bool,
    html_font_metrics: bool,
    html_font_scale: Option<f32>,
    advances: Option<inline_advances::Cursor>,
    strut_metrics: Option<(f32, f32)>,
    first_indent: f32,
    clearance: Option<&'a dyn Fn(f32, f32) -> f32>,
    out: Vec<SourceLine>,
    cur: Vec<Word>,
    open_edges: Vec<InlineEdge>,
    y: f32,
    indent: f32,
    lw: f32,
    x: f32,
}

impl<'a> Placer<'a> {
    fn new(
        align: Align,
        region: LineRegion<'a>,
        strut: f32,
        policy: (bool, bool, Option<f32>),
    ) -> Self {
        let (capture_sources, html_font_metrics, html_font_scale) = policy;
        let (indent, lw) = region(0.0);
        Placer {
            align,
            region,
            strut,
            capture_sources,
            html_font_metrics,
            html_font_scale,
            advances: html_font_scale.map(|scale| inline_advances::Cursor::new(scale, 0.0)),
            strut_metrics: None,
            first_indent: 0.0,
            clearance: None,
            out: Vec::new(),
            cur: Vec::new(),
            open_edges: Vec::new(),
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
        let current = std::mem::take(&mut self.cur);
        let mut next_open = self.open_edges.clone();
        for word in &current {
            for (_, edge) in &word.edges {
                if edge.start {
                    next_open.push(edge.clone());
                } else {
                    next_open.retain(|open| open.node_id != edge.node_id);
                }
            }
        }
        let mut line = place_line(
            current,
            self.lw,
            self.align,
            self.strut,
            &self.open_edges,
            self.strut_metrics,
            (
                self.capture_sources,
                self.html_font_metrics,
                self.html_font_scale,
            ),
        );
        self.open_edges = next_open;
        line.line.top = self.y;
        line.line.indent = self.indent;
        self.y += line.line.height;
        self.out.push(line);
        (self.indent, self.lw) = (self.region)(self.y);
        self.x = 0.0;
        self.advances = self
            .html_font_scale
            .map(|scale| inline_advances::Cursor::new(scale, 0.0));
    }

    /// Add one word: wrap first if it no longer fits, then flush after it if it is
    /// a forced break (so a `<br>` ends its line and the next word starts below).
    fn add(&mut self, mut word: Word) {
        if self.attach_trailing_close(&word) {
            return;
        }
        if self.needs_wrap(&word) {
            word.leading_spaces = self.wrapped_leading_spaces(&word);
            self.flush();
        }
        let width = self.html_font_scale.map_or(word.width, |scale| {
            inline_advances::Cursor::new(scale, 0.0).candidate_width(&word)
        });
        self.avoid_floats(width);
        let forced = word.break_after;
        self.x += word.width + word.space_after;
        if let Some(advances) = &mut self.advances {
            advances.word(&word);
            advances.space(&word);
        }
        self.cur.push(word);
        if forced {
            self.flush();
        }
    }

    fn wrapped_leading_spaces(&mut self, next: &Word) -> Vec<Seg> {
        if self.html_font_scale.is_none() {
            return Vec::new();
        }
        let last = self
            .cur
            .last_mut()
            .expect("wrapping requires existing line content");
        wrapped_source_spaces(last, next)
    }

    fn attach_trailing_close(&mut self, word: &Word) -> bool {
        if !self.cur.is_empty() || !closing_only(word) {
            return false;
        }
        let Some(last) = self.out.last_mut() else {
            return false;
        };
        let mut x = last.line.inline_origin + last.line.width;
        for (_, edge) in &word.edges {
            last.line.inline_edges.push(PlacedInlineEdge {
                edge: edge.clone(),
                x,
            });
            x += edge.advance;
            self.open_edges.retain(|open| open.node_id != edge.node_id);
        }
        last.line.width = x - last.line.inline_origin;
        true
    }
    fn needs_wrap(&self, word: &Word) -> bool {
        if !self.html_font_metrics {
            return !self.cur.is_empty() && self.x + word.width > self.lw;
        }
        let width = self
            .advances
            .as_ref()
            .map_or(self.x + word.width, |advances| {
                advances.candidate_width(word)
            });
        word.width > 0.0
            && width > self.lw
            && self
                .cur
                .iter()
                .any(|previous| previous.width > 0.0 || previous.atom.is_some())
    }
    fn avoid_floats(&mut self, width: f32) {
        if !self.cur.is_empty() {
            return;
        }
        let Some(clearance) = self.clearance else {
            return;
        };
        let indent = if self.out.is_empty() {
            self.first_indent
        } else {
            0.0
        };
        self.y = clearance(self.y, width + indent);
        (self.indent, self.lw) = (self.region)(self.y);
        self.indent += indent;
        self.lw -= indent;
    }

    fn finish(mut self) -> Vec<SourceLine> {
        self.flush();
        self.out
    }
}

fn closing_only(word: &Word) -> bool {
    !word.edges.is_empty()
        && word.segs.is_empty()
        && word.spaces.is_empty()
        && word.edges.iter().all(|(_, edge)| !edge.start)
}
/// Break `words` into placed lines via a [`Placer`].
fn wrap_and_place(
    words: Vec<Word>,
    align: Align,
    region: LineRegion,
    strut: f32,
    policy: (bool, bool, Option<f32>),
) -> Vec<SourceLine> {
    let mut placer = Placer::new(align, region, strut, policy);
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

// Source pins and the Chromium leading-policy attribution are recorded in
// text/html-font-metrics-provenance.json. Quantize actual font metrics before
// placement so physical-scale geometry never rewrites returned rectangles.
fn source_font_metrics(seg: &Seg, scale: Option<f32>) -> crate::text::HtmlFontMetrics {
    let scale = scale.unwrap_or(1.0);
    let metrics = seg.primary_face.html_metrics(seg.font_size * scale);
    crate::text::HtmlFontMetrics {
        ascent: metrics.ascent / scale,
        descent: metrics.descent / scale,
        line_gap: metrics.line_gap / scale,
    }
}

fn fold_seg_metrics(seg: &Seg, m: &mut Metrics, capture_sources: bool, scale: Option<f32>) {
    let shift = valign_shift(seg.valign, seg.font_size);
    let (ascent, descent, normal) = if capture_sources {
        let metrics = source_font_metrics(seg, scale);
        (
            metrics.ascent,
            metrics.descent,
            metrics.ascent + metrics.descent + metrics.line_gap,
        )
    } else {
        (
            seg.face.ascent_px(seg.font_size),
            seg.face.descent_px(seg.font_size),
            seg.face.line_height_px(seg.font_size),
        )
    };
    m.ascent = m.ascent.max(ascent + shift);
    m.descent = m.descent.max(descent - shift);
    let lh = seg.line_height.unwrap_or(normal);
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
/// atoms grow it. The PDF policy floors the height at the font box; explicit
/// HTML line-height permits negative leading and overlapping glyph font boxes.
pub(super) fn native_font_height(
    ascent: f32,
    descent: f32,
    line_height: f32,
    shift: f32,
    scale: f32,
) -> (f32, f32) {
    let line_height = super::value::html_used_value(line_height, Some(scale));
    let extra = line_height - ascent - descent;
    let units = (extra * scale * 64.0).round() as i64;
    let above = (units / 2).div_euclid(64) as f32 / scale;
    (ascent + above + shift, descent + extra - above - shift)
}
fn native_line_metrics(
    words: &[Word],
    open: &[InlineEdge],
    strut: (f32, f32),
    scale: Option<f32>,
) -> Metrics {
    let mut metrics = Metrics {
        ascent: strut.0,
        descent: strut.1,
        height: 0.0,
    };
    for edge in open {
        fold_native_edge(edge, &mut metrics, scale);
    }
    for word in words {
        fold_native_word(word, &mut metrics, scale);
    }
    metrics.height = metrics.height.max(metrics.ascent + metrics.descent);
    metrics
}
fn fold_native_word(word: &Word, metrics: &mut Metrics, scale: Option<f32>) {
    for (_, edge) in &word.edges {
        fold_native_edge(edge, metrics, scale);
    }
    for seg in &word.segs {
        let font = source_font_metrics(seg, scale);
        let lh = seg
            .line_height
            .unwrap_or(font.ascent + font.descent + font.line_gap);
        unite_native(
            metrics,
            native_font_height(
                font.ascent,
                font.descent,
                lh,
                seg.baseline_shift.unwrap_or_default(),
                scale.unwrap_or(1.0),
            ),
        );
    }
    if let Some(atom) = word.atom {
        fold_native_atom(&atom, word.atom_shift, metrics);
    }
}
fn fold_native_atom(atom: &InlineAtom, shift: f32, metrics: &mut Metrics) {
    if matches!(atom.valign, VAlign::Top | VAlign::Bottom | VAlign::Middle) {
        fold_atom_metrics(atom, metrics);
        return;
    }
    metrics.ascent = metrics.ascent.max(atom.outer_height() + shift);
    metrics.descent = metrics.descent.max(-shift);
}
fn fold_native_edge(edge: &InlineEdge, metrics: &mut Metrics, scale: Option<f32>) {
    if !edge.contributes_metrics {
        return;
    }
    let scale = scale.unwrap_or(1.0);
    let font = edge.face.html_metrics(edge.font_size * scale);
    let lh = edge
        .line_height
        .unwrap_or((font.ascent + font.descent + font.line_gap) / scale);
    unite_native(
        metrics,
        native_font_height(
            font.ascent / scale,
            font.descent / scale,
            lh,
            edge.baseline_shift,
            scale,
        ),
    );
}
fn unite_native(metrics: &mut Metrics, height: (f32, f32)) {
    metrics.ascent = metrics.ascent.max(height.0);
    metrics.descent = metrics.descent.max(height.1);
}
fn apply_baselines(words: &mut [Word], baselines: &std::collections::HashMap<NodeId, f32>) {
    for word in words {
        for (_, edge) in &mut word.edges {
            edge.baseline_shift = baselines.get(&edge.node_id).copied().unwrap_or_default();
        }
        apply_run_baselines(word, baselines);
    }
}
fn apply_run_baselines(word: &mut Word, baselines: &std::collections::HashMap<NodeId, f32>) {
    for seg in word.segs.iter_mut().chain(word.spaces.iter_mut()) {
        seg.baseline_shift = baselines.get(&seg.node_id).copied();
    }
}

fn line_metrics(
    words: &[Word],
    strut: f32,
    capture_sources: bool,
    scale: Option<f32>,
    open_edges: &[InlineEdge],
) -> Metrics {
    let mut m = Metrics {
        ascent: 0.0,
        descent: 0.0,
        height: strut,
    };
    for edge in open_edges {
        fold_edge_metrics(edge, &mut m, scale);
    }
    for word in words {
        fold_word_metrics(word, &mut m, capture_sources, scale);
    }
    finish_line_height(&mut m, words, capture_sources);
    m
}

fn fold_word_metrics(word: &Word, m: &mut Metrics, capture_sources: bool, scale: Option<f32>) {
    for (_, edge) in &word.edges {
        fold_edge_metrics(edge, m, scale);
    }
    for seg in &word.segs {
        fold_seg_metrics(seg, m, capture_sources, scale);
    }
    if let Some(atom) = word.atom {
        fold_atom_metrics(&atom, m);
    }
}
fn fold_edge_metrics(edge: &InlineEdge, m: &mut Metrics, scale: Option<f32>) {
    if !edge.contributes_metrics {
        return;
    }
    let scale = scale.unwrap_or(1.0);
    let font = edge.face.html_metrics(edge.font_size * scale);
    let shift = valign_shift(edge.valign, edge.font_size);
    m.ascent = m.ascent.max(font.ascent / scale + shift);
    m.descent = m.descent.max(font.descent / scale - shift);
    m.height = m.height.max(
        edge.line_height
            .unwrap_or((font.ascent + font.descent + font.line_gap) / scale),
    );
}

fn finish_line_height(metrics: &mut Metrics, words: &[Word], html_font_metrics: bool) {
    // Atomic baseline placement still uses the existing ascent/descent policy.
    if !html_font_metrics || words.iter().any(|word| word.atom.is_some()) {
        metrics.height = metrics.height.max(metrics.ascent + metrics.descent);
    }
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

fn source_shaper_is_rtl(text: &str, enabled: bool) -> bool {
    if !enabled {
        return false;
    }
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    // rustybuzz::shape applies this same native property selection before shaping.
    buffer.guess_segment_properties();
    buffer.direction() == rustybuzz::Direction::RightToLeft
}

fn shape_seg(seg: &Seg, pen_x: f32, baseline: f32, policy: (bool, bool, Option<f32>)) -> SourceRun {
    let (capture_sources, html_font_metrics, html_font_scale) = policy;
    let scale = seg.face.scale(seg.font_size);
    let shift = seg
        .baseline_shift
        .unwrap_or_else(|| valign_shift(seg.valign, seg.font_size));
    let seg_baseline = baseline - shift;
    let mut glyphs = Vec::new();
    let mut clusters = Vec::new();
    let mut x = pen_x;
    let mut native_x = f64::from(pen_x);
    let shaped = seg.original_units.as_deref().map_or_else(
        || seg.face.shape(&seg.text),
        |units| seg.face.shape_original_utf16(units),
    );
    let right_to_left = source_shaper_is_rtl(&seg.text, capture_sources);
    let (boundaries, utf16) = if capture_sources {
        let mut boundaries: Vec<_> = shaped.iter().map(|glyph| glyph.cluster as usize).collect();
        boundaries.push(seg.text.len());
        boundaries.sort_unstable();
        boundaries.dedup();
        let mut offset = 0;
        let mut utf16: Vec<_> = seg
            .text
            .char_indices()
            .map(|(byte, character)| {
                let result = (byte, offset);
                offset += character.len_utf16();
                result
            })
            .collect();
        utf16.push((seg.text.len(), offset));
        (boundaries, utf16)
    } else {
        (Vec::new(), Vec::new())
    };
    for g in shaped {
        let ordinary_advance = scale * g.x_advance as f32 + seg.letter_spacing;
        let native_advance = html_font_scale.map_or(f64::from(ordinary_advance), |_| {
            native_text_advance::glyph_advance(
                &seg.face,
                seg.font_size,
                g.x_advance,
                seg.letter_spacing,
            )
        });
        let advance = html_font_scale.map_or(ordinary_advance, |_| native_advance as f32);
        let position = html_font_scale.map_or(x, |_| native_x as f32);
        if capture_sources {
            let start = g.cluster as usize;
            let end = boundaries[boundaries
                .binary_search(&start)
                .expect("shaper cluster exists")
                + 1];
            let offset = |byte| {
                utf16[utf16
                    .binary_search_by_key(&byte, |value| value.0)
                    .expect("shaper cluster is a character boundary")]
                .1
            };
            clusters.push(GlyphSourceCluster {
                collapsed_whitespace: false,
                right_to_left,
                byte_start: seg.source_bytes + start,
                byte_end: seg.source_bytes + end,
                utf16_start: seg.source_utf16 + offset(start),
                utf16_end: seg.source_utf16 + offset(end),
                pen_x: position,
                advance,
            });
        }
        glyphs.push(PositionedGlyph {
            glyph_id: g.glyph_id,
            x: position + scale * g.x_offset as f32,
            y: seg_baseline - scale * g.y_offset as f32,
        });
        x += advance;
        native_x += native_advance;
    }
    let (ascent, descent) = if html_font_metrics {
        let metrics = source_font_metrics(seg, html_font_scale);
        (metrics.ascent, metrics.descent)
    } else {
        (
            seg.face.ascent_px(seg.font_size),
            seg.face.descent_px(seg.font_size),
        )
    };
    SourceRun {
        font_box: SourceFontBox {
            baseline: seg_baseline,
            ascent,
            descent,
        },
        clusters,
        run: GlyphRun {
            node_id: seg.node_id,
            glyphs,
            face: seg.face.clone(),
            font_size: seg.font_size,
            color: seg.color,
        },
    }
}

fn same_face(a: &FontFace, b: &FontFace) -> bool {
    a.family() == b.family()
        && a.weight() == b.weight()
        && a.is_italic() == b.is_italic()
        && a.index() == b.index()
}

fn can_merge(a: &GlyphRun, b: &GlyphRun) -> bool {
    a.node_id == b.node_id
        && a.font_size == b.font_size
        && a.color == b.color
        && same_face(&a.face, &b.face)
}

fn can_merge_source(a: &SourceRun, b: &SourceRun, capture_sources: bool) -> bool {
    can_merge(&a.run, &b.run) && source_metadata_compatible(a, b, capture_sources)
}
fn source_metadata_compatible(a: &SourceRun, b: &SourceRun, enabled: bool) -> bool {
    if !enabled {
        return true;
    }
    a.font_box == b.font_box
        && a.run.face.data().as_ptr() == b.run.face.data().as_ptr()
        && a.run.face.index() == b.run.face.index()
}

fn merge_runs(runs: Vec<SourceRun>, capture_sources: bool) -> Vec<SourceRun> {
    let mut out: Vec<SourceRun> = Vec::new();
    for run in runs {
        match out.last_mut() {
            Some(last) if can_merge_source(last, &run, capture_sources) => {
                last.run.glyphs.extend(run.run.glyphs);
                last.clusters.extend(run.clusters);
            }
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

struct LinePlacement {
    pen: f32,
    baseline: f32,
    height: f32,
    policy: (bool, bool, Option<f32>),
    runs: Vec<SourceRun>,
    atoms: Vec<PlacedAtom>,
    inline_edges: Vec<PlacedInlineEdge>,
    advances: Option<inline_advances::Cursor>,
}
impl LinePlacement {
    fn word(&mut self, word: &Word, has_next: bool) {
        for space in &word.leading_spaces {
            self.runs.push(shape_space(
                space,
                self.pen,
                self.baseline,
                self.policy,
                0.0,
            ));
        }
        if let Some(atom) = word.atom {
            self.atom(&atom, word.atom_shift);
        } else {
            self.text(word);
        }
        self.begin_space(word, has_next);
        self.spaces(word, has_next);
        self.advance_space(word, has_next);
    }
    fn atom(&mut self, atom: &InlineAtom, shift: f32) {
        self.native_barrier(0.0);
        let margin_top = atom_margin_top(atom, self.height, self.baseline) - shift;
        self.atoms.push(PlacedAtom {
            id: atom.id,
            x: self.pen + atom.margin_left,
            y: margin_top + atom.margin_top,
        });
        let width = atom.width + atom.margin_left + atom.margin_right;
        self.pen += width;
        self.native_barrier(width);
    }
    fn begin_space(&mut self, word: &Word, has_next: bool) {
        if !has_next {
            return;
        }
        if let (Some(advances), Some((source, _))) = (&mut self.advances, word.native_space) {
            advances.begin(source);
            self.pen = advances.position();
        }
    }
    fn advance_space(&mut self, word: &Word, has_next: bool) {
        if let Some(advances) = &mut self.advances {
            if has_next {
                advances.space(word);
            }
            self.pen = advances.position();
        } else {
            self.pen += word.space_after;
        }
    }
    fn native_barrier(&mut self, advance: f32) {
        if let Some(advances) = &mut self.advances {
            advances.barrier(advance);
            self.pen = advances.position();
        }
    }
    fn edge(&mut self, edge: &InlineEdge) {
        self.native_barrier(0.0);
        self.inline_edges.push(PlacedInlineEdge {
            edge: edge.clone(),
            x: self.pen,
        });
        self.pen += edge.advance;
        self.native_barrier(edge.advance);
    }
    fn segment(&mut self, seg: &Seg) {
        if let Some(advances) = &mut self.advances {
            advances.begin(seg.node_id);
            self.pen = advances.position();
        }
        self.runs
            .push(shape_seg(seg, self.pen, self.baseline, self.policy));
        if let Some(advances) = &mut self.advances {
            advances.text(seg.node_id, seg.native_width);
            self.pen = advances.position();
        } else {
            self.pen += seg.width;
        }
    }
    fn text(&mut self, word: &Word) {
        for index in 0..=word.segs.len() {
            for (_, edge) in word.edges.iter().filter(|(at, _)| *at == index) {
                self.edge(edge);
            }
            if let Some(seg) = word.segs.get(index) {
                self.segment(seg);
            }
        }
    }
    fn spaces(&mut self, word: &Word, has_next: bool) {
        if !self.policy.1 {
            return;
        }
        let mut pen = self.pen;
        for (index, space) in word.spaces.iter().enumerate() {
            let advance = visible_space_advance(word, index, has_next);
            self.runs
                .push(shape_space(space, pen, self.baseline, self.policy, advance));
            if self.policy.2.is_some() {
                pen += advance;
            }
        }
    }
}
fn visible_space_advance(word: &Word, index: usize, has_next: bool) -> f32 {
    if index == 0 && has_next {
        word.native_space
            .map_or(word.space_after, |(_, advance)| advance as f32)
    } else {
        0.0
    }
}
fn shape_space(
    space: &Seg,
    pen: f32,
    baseline: f32,
    policy: (bool, bool, Option<f32>),
    advance: f32,
) -> SourceRun {
    let mut run = shape_seg(space, pen, baseline, policy);
    for glyph in &mut run.clusters {
        glyph.advance = advance;
        glyph.collapsed_whitespace = true;
    }
    run
}
fn place_line(
    words: Vec<Word>,
    max_width: f32,
    align: Align,
    strut: f32,
    open_edges: &[InlineEdge],
    strut_metrics: Option<(f32, f32)>,
    policy: (bool, bool, Option<f32>),
) -> SourceLine {
    let m = match strut_metrics {
        Some(strut) => native_line_metrics(&words, open_edges, strut, policy.2),
        None => line_metrics(&words, strut, policy.1, policy.2, open_edges),
    };
    let half_leading = (m.height - m.ascent - m.descent) / 2.0;
    let leading = policy
        .2
        .map_or(half_leading, |scale| (half_leading * scale).floor() / scale);
    let baseline = m.ascent
        + if strut_metrics.is_some() {
            0.0
        } else {
            leading
        };
    let width = policy.2.map_or_else(
        || line_used_width(&words),
        |scale| inline_advances::line_width(&words, scale),
    );
    let pen = align_offset(align, width, max_width);
    let mut placement = LinePlacement {
        pen,
        runs: Vec::new(),
        atoms: Vec::new(),
        inline_edges: Vec::new(),
        baseline,
        height: m.height,
        policy,
        advances: policy
            .2
            .map(|scale| inline_advances::Cursor::new(scale, pen)),
    };
    for (index, word) in words.iter().enumerate() {
        placement.word(word, index + 1 < words.len());
    }
    let LinePlacement {
        runs,
        atoms,
        inline_edges,
        ..
    } = placement;
    let mut output_runs = Vec::new();
    let mut clusters = Vec::new();
    let mut font_boxes = Vec::new();
    for value in merge_runs(runs, policy.0) {
        output_runs.push(value.run);
        clusters.push(value.clusters);
        font_boxes.push(value.font_box);
    }
    SourceLine {
        font_boxes,
        clusters,
        line: InlineLine {
            inline_edges,
            baseline,
            inline_origin: align_offset(align, width, max_width),
            runs: output_runs,
            atoms,
            width,
            top: 0.0,
            height: m.height,
            indent: 0.0,
        },
    }
}

fn finalize(lines: Vec<SourceLine>) -> ParagraphSourceLayout {
    let mut height = 0.0_f32;
    let mut width = 0.0_f32;
    for source in &lines {
        let line = &source.line;
        height = height.max(line.top + line.height);
        width = width.max(line.indent + line.width);
    }
    let mut output_lines = Vec::new();
    let mut clusters = Vec::new();
    let mut font_boxes = Vec::new();
    for value in lines {
        output_lines.push(value.line);
        clusters.push(value.clusters);
        font_boxes.push(value.font_boxes);
    }
    ParagraphSourceLayout {
        font_boxes,
        clusters,
        layout: ParagraphLayout {
            lines: output_lines,
            width,
            height,
        },
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
    let words = build_pieces(
        pieces,
        reg,
        diags,
        SourcePolicy {
            capture: false,
            advances: false,
        },
        None,
    );
    finalize(wrap_and_place(
        words,
        align,
        region,
        strut,
        (false, false, None),
    ))
    .layout
}

/// The same native shaping and line-breaking result with glyph source offsets.
/// Offsets address the original input run, including collapsed whitespace; this
/// exposes real clusters rather than dividing line rectangles into guessed carets.
pub fn layout_paragraph_in_with_sources(
    pieces: &[Piece],
    reg: &FontRegistry,
    align: Align,
    diags: &mut Diagnostics,
    region: LineRegion,
    strut: f32,
) -> ParagraphSourceLayout {
    let words = build_pieces(
        pieces,
        reg,
        diags,
        SourcePolicy {
            capture: true,
            advances: false,
        },
        None,
    );
    finalize(wrap_and_place(
        words,
        align,
        region,
        strut,
        (true, false, None),
    ))
}

/// Continuous HTML text geometry with actual source clusters and rounded native
/// HTML font metrics. The original paragraph/PDF functions retain their policy.
pub fn layout_html_paragraph_in_with_sources(
    pieces: &[Piece],
    reg: &FontRegistry,
    align: Align,
    diags: &mut Diagnostics,
    region: LineRegion,
    strut: f32,
) -> ParagraphSourceLayout {
    let words = build_pieces(
        pieces,
        reg,
        diags,
        SourcePolicy {
            capture: true,
            advances: false,
        },
        None,
    );
    finalize(wrap_and_place(
        words,
        align,
        region,
        strut,
        (true, true, None),
    ))
}

pub(crate) fn layout_html_paragraph_in_with_sources_at_scale(
    pieces: &[Piece],
    reg: &FontRegistry,
    align: Align,
    diags: &mut Diagnostics,
    region: LineRegion,
    strut: f32,
    options: NativeLineOptions<'_>,
) -> ParagraphSourceLayout {
    let mut words = build_pieces(
        pieces,
        reg,
        diags,
        SourcePolicy {
            capture: true,
            advances: options.scale.is_some(),
        },
        options.original_units,
    );
    apply_baselines(&mut words, options.baselines);
    if options.scale.is_some() {
        inline_advances::prepare(&mut words);
    }
    let mut placer = Placer::new(align, region, strut, (true, true, options.scale));
    placer.strut_metrics = options.strut_metrics;
    placer.first_indent = options.first_indent;
    placer.clearance = Some(options.clearance);
    for word in words {
        placer.add(word);
    }
    finalize(placer.finish())
}

pub(crate) struct NativeLineOptions<'a> {
    pub baselines: &'a std::collections::HashMap<NodeId, f32>,
    pub strut_metrics: Option<(f32, f32)>,
    pub scale: Option<f32>,
    pub first_indent: f32,
    pub clearance: &'a dyn Fn(f32, f32) -> f32,
    pub original_units: Option<&'a crate::layout_snapshot::text_units::NativeUnitMap>,
}

pub(crate) fn layout_runs_with_units(
    runs: &[InlineRun],
    fonts: &FontRegistry,
    width: f32,
    diags: &mut Diagnostics,
    original: Option<&crate::layout_snapshot::text_units::NativeUnitMap>,
    html_metrics: bool,
) -> ParagraphLayout {
    let pieces: Vec<_> = runs.iter().cloned().map(Piece::Run).collect();
    let words = build_pieces(
        &pieces,
        fonts,
        diags,
        SourcePolicy {
            capture: html_metrics,
            advances: false,
        },
        original,
    );
    finalize(wrap_and_place(
        words,
        Align::Left,
        &|_| (0.0, width),
        0.0,
        (html_metrics, html_metrics, None),
    ))
    .layout
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

    fn collection_source_run(face: FontFace) -> SourceRun {
        SourceRun {
            run: GlyphRun {
                node_id: NodeId(7),
                glyphs: vec![PositionedGlyph {
                    glyph_id: face.shape("A")[0].glyph_id,
                    x: 0.0,
                    y: 0.0,
                }],
                face,
                font_size: 16.0,
                color: Rgba {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
            },
            clusters: Vec::new(),
            font_box: SourceFontBox {
                baseline: 12.0,
                ascent: 12.0,
                descent: 4.0,
            },
        }
    }

    #[test]
    fn shared_collection_indexes_do_not_merge_text_runs() {
        let [first, second] = FontFace::collection_fixture_faces();
        for capture_sources in [false, true] {
            let runs = vec![
                collection_source_run(first.clone()),
                collection_source_run(second.clone()),
            ];
            let merged = merge_runs(runs, capture_sources);
            assert_eq!(merged.len(), 2);
            assert_eq!(
                [merged[0].run.face.index(), merged[1].run.face.index()],
                [0, 1]
            );
            let copies = vec![
                collection_source_run(first.clone()),
                collection_source_run(first.clone()),
            ];
            let merged = merge_runs(copies, capture_sources);
            assert_eq!(merged.len(), 1);
            assert_eq!(merged[0].run.glyphs.len(), 2);
        }
    }

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
