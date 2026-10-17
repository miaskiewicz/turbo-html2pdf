//! Source metadata follows actual native text fragments through cloning/moves.
//! Private layout IDs distinguish repeated identical runs on different lines;
//! caller-owned source identities still point to the original text node.
use super::{SnapshotFragment, SourceId};
use crate::layout::inline::{GlyphSourceCluster, SourceFontBox};
use crate::{Fragment, FragmentContent, NodeId};
use std::collections::HashMap;

/// One glyph's real shaper cluster and pen advance, relative to its text fragment.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotTextGlyph {
    pub glyph_id: u16,
    pub x: f32,
    pub y: f32,
    pub source: GlyphSourceCluster,
}

/// Text from one actual native fragment. Transform and fragment origin are in
/// LayoutSnapshot::fragments()[fragment_index]; this is not a guessed Range box.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotTextRun {
    pub fragment_index: usize,
    pub source_id: Option<SourceId>,
    pub source_ancestors: Vec<SourceId>,
    pub glyphs: Vec<SnapshotTextGlyph>,
    pub font_box: SourceFontBox,
}

pub(crate) struct CapturedRun {
    pub(crate) original: NodeId,
    clusters: Vec<GlyphSourceCluster>,
    font_box: SourceFontBox,
}
pub(crate) struct TextCapture {
    pub(crate) collapsed_sources: std::collections::HashSet<NodeId>,
    next: u32,
    count: usize,
    pub(crate) exceeded: bool,
    pub(crate) runs: HashMap<NodeId, CapturedRun>,
}
impl TextCapture {
    pub(crate) fn new(next: u32) -> Self {
        Self {
            collapsed_sources: Default::default(),
            next,
            count: 0,
            exceeded: false,
            runs: HashMap::new(),
        }
    }
    pub(crate) fn record(
        &mut self,
        original: NodeId,
        clusters: &[GlyphSourceCluster],
        font_box: SourceFontBox,
    ) -> NodeId {
        let Some(next) = self.next.checked_add(1) else {
            self.exceeded = true;
            return original;
        };
        self.count = self.count.saturating_add(clusters.len());
        if self.count > 1_000_000 {
            self.exceeded = true;
            return original;
        }
        let id = NodeId(self.next);
        self.next = next;
        self.runs.insert(
            id,
            CapturedRun {
                original,
                font_box,
                clusters: clusters.to_vec(),
            },
        );
        id
    }
    pub(crate) fn collect(
        &self,
        galley: &Fragment,
        fragments: &[SnapshotFragment],
    ) -> Vec<SnapshotTextRun> {
        let mut runs = Vec::new();
        self.append(galley, fragments, &mut 0, &mut runs);
        runs
    }
    fn append(
        &self,
        fragment: &Fragment,
        fragments: &[SnapshotFragment],
        index: &mut usize,
        out: &mut Vec<SnapshotTextRun>,
    ) {
        let snapshot = &fragments[*index];
        *index += 1;
        if let (Some(capture), FragmentContent::TextLine { glyphs, .. }) =
            (self.runs.get(&fragment.node_id), &fragment.content)
        {
            debug_assert_eq!(capture.clusters.len(), glyphs.len());
            out.push(SnapshotTextRun {
                font_box: capture.font_box,
                fragment_index: snapshot.index,
                source_id: snapshot.source_id,
                source_ancestors: snapshot.source_ancestors.clone(),
                glyphs: glyphs
                    .iter()
                    .zip(&capture.clusters)
                    .map(|(glyph, source)| SnapshotTextGlyph {
                        glyph_id: glyph.glyph_id,
                        x: glyph.x,
                        y: glyph.y,
                        source: *source,
                    })
                    .collect(),
            });
        }
        for i in fragment.paint_order() {
            self.append(&fragment.children[i], fragments, index, out);
        }
    }
}

pub(crate) fn alias_sources(
    capture: Option<&TextCapture>,
    records: &mut HashMap<NodeId, super::sources::SourceRecord>,
) {
    let Some(capture) = capture else {
        return;
    };
    for (&id, run) in &capture.runs {
        if let Some(source) = records.get(&run.original).cloned() {
            records.insert(id, source);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cluster() -> GlyphSourceCluster {
        GlyphSourceCluster {
            collapsed_whitespace: false,
            right_to_left: false,
            byte_start: 0,
            byte_end: 1,
            utf16_start: 0,
            utf16_end: 1,
            pen_x: 0.0,
            advance: 1.0,
        }
    }
    #[test]
    fn native_capture_never_wraps_layout_ids_or_exceeds_the_glyph_budget() {
        let original = NodeId(7);
        let mut ids = TextCapture::new(u32::MAX);
        assert_eq!(
            ids.record(
                original,
                &[cluster()],
                SourceFontBox {
                    baseline: 1.0,
                    ascent: 1.0,
                    descent: 0.0
                }
            ),
            original
        );
        assert!(ids.exceeded && ids.runs.is_empty());
        let mut capacity = TextCapture::new(100);
        capacity.count = 1_000_000;
        assert_eq!(
            capacity.record(
                original,
                &[cluster()],
                SourceFontBox {
                    baseline: 1.0,
                    ascent: 1.0,
                    descent: 0.0
                }
            ),
            original
        );
        assert!(capacity.exceeded && capacity.runs.is_empty());
    }
}

/// One mapped text node's actual native UTF-8 input, retained before whitespace
/// collapsing or fallback itemization. UTF-16 offsets index this same source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotTextSource {
    pub source_id: SourceId,
    pub text: String,
}

pub(crate) fn validate_source_budget(
    styled: &[crate::StyledNode],
) -> Result<(), super::TextSnapshotError> {
    let mut counts = (0, 0);
    count_source_bytes(styled, &mut counts)
}

fn count_source_bytes(
    nodes: &[crate::StyledNode],
    counts: &mut (usize, usize),
) -> Result<(), super::TextSnapshotError> {
    for node in nodes {
        match node {
            crate::StyledNode::Text(text) => charge_source_input(counts, text)?,
            crate::StyledNode::Element(element) => count_source_bytes(&element.children, counts)?,
        }
    }
    Ok(())
}

fn charge_source_input(
    counts: &mut (usize, usize),
    text: &str,
) -> Result<(), super::TextSnapshotError> {
    charge_source_count(&mut counts.0, text.len(), 16 * 1024 * 1024)?;
    charge_source_count(&mut counts.1, text.chars().count(), 1_000_000)
}

fn charge_source_count(
    count: &mut usize,
    amount: usize,
    limit: usize,
) -> Result<(), super::TextSnapshotError> {
    *count = count
        .checked_add(amount)
        .ok_or(super::TextSnapshotError::Budget)?;
    if *count > limit {
        return Err(super::TextSnapshotError::Budget);
    }
    Ok(())
}
