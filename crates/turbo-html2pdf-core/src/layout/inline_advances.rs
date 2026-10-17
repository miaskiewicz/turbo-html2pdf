//! Native source items retain unsnapped glyph positions but admit their aggregate
//! advance to the physical LayoutUnit grid before wrapping and placement.
use super::{native_text_advance, NodeId, Seg, Word};

#[derive(Clone)]
pub(super) struct Cursor {
    source: Option<NodeId>,
    origin: f64,
    advance: f64,
    units: f64,
}
impl Cursor {
    pub(super) fn new(scale: f32, origin: f32) -> Self {
        Self {
            source: None,
            origin: f64::from(origin),
            advance: 0.0,
            units: f64::from(scale) * 64.0,
        }
    }
    pub(super) fn position(&self) -> f32 {
        (self.origin + self.advance) as f32
    }
    pub(super) fn snapped(&self) -> f32 {
        (self.origin + (self.advance * self.units).ceil() / self.units) as f32
    }
    pub(super) fn begin(&mut self, source: NodeId) {
        if self.source != Some(source) {
            self.barrier(0.0);
            self.source = Some(source);
        }
    }
    pub(super) fn text(&mut self, source: NodeId, advance: f64) {
        self.begin(source);
        self.advance += advance;
    }
    pub(super) fn barrier(&mut self, advance: f32) {
        self.origin += (self.advance * self.units).ceil() / self.units + f64::from(advance);
        self.advance = 0.0;
        self.source = None;
    }
    pub(super) fn word(&mut self, word: &Word) {
        if word.atom.is_some() {
            self.barrier(word.width);
            return;
        }
        for index in 0..=word.segs.len() {
            for (_, edge) in word.edges.iter().filter(|(at, _)| *at == index) {
                self.barrier(edge.advance);
            }
            if let Some(seg) = word.segs.get(index) {
                self.text(seg.node_id, seg.native_width);
            }
        }
    }
    pub(super) fn space(&mut self, word: &Word) {
        if let Some((source, advance)) = word.native_space {
            self.text(source, advance);
        } else if word.space_after != 0.0 {
            self.barrier(word.space_after);
        }
    }
    pub(super) fn candidate_width(&self, word: &Word) -> f32 {
        let mut candidate = self.clone();
        candidate.word(word);
        candidate.snapped()
    }
}

fn prepare_segment(seg: &mut Seg) {
    seg.native_width = native_text_advance::measure(
        &seg.face,
        seg.font_size,
        seg.letter_spacing,
        &seg.text,
        seg.original_units.as_deref(),
    );
}
fn prepare_word(word: &mut Word) {
    word.segs.iter_mut().for_each(prepare_segment);
    let source = word.spaces.first().or(word.advance_space.as_ref());
    if let Some(source) = source.filter(|_| word.space_after != 0.0) {
        word.native_space = Some((
            source.node_id,
            native_text_advance::measure(
                &source.face,
                source.font_size,
                source.letter_spacing,
                " ",
                None,
            ),
        ));
    }
}
pub(super) fn prepare(words: &mut [Word]) {
    words.iter_mut().for_each(prepare_word);
}
pub(super) fn line_width(words: &[Word], scale: f32) -> f32 {
    let mut cursor = Cursor::new(scale, 0.0);
    for (index, word) in words.iter().enumerate() {
        cursor.word(word);
        if index + 1 < words.len() {
            cursor.space(word);
        }
    }
    cursor.snapped()
}
