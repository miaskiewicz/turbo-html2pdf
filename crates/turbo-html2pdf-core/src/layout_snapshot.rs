//! An immutable live-tree layout snapshot with caller-owned source identities.
//!
//! Unlike the PDF convenience entry points this API neither reparses HTML nor
//! strips nodes, rewrites attributes, drops nonpainting boxes, or fits images to
//! a page. A source ID is supplied by the caller's DOM, never inferred from a
//! CSS attribute or a layout ordinal. The galley remains the native engine's
//! actual output, including every anonymous, image, and text-line fragment.
//!
//! This is a provenance seam, not a complete CSSOM View implementation. In
//! particular a text-line fragment's rectangle is its line container, not a
//! Range rectangle. The optional text-enabled API preserves actual shaper
//! clusters and original UTF-16 offsets; complete caret boxes, whitespace/bidi
//! mapping, scrollbars, clipping and fragmented inline borders remain separate.

mod fixed_containing;
mod geometry;
mod metrics;
pub(crate) mod sources;
pub(crate) mod text;
mod text_geometry;

use crate::layout::{block, boxgen};
use crate::{Cascade, Diagnostics, FontRegistry, Fragment, ImageCtx, Node, NodeId, StyledNode};
use std::collections::BTreeMap;

pub use geometry::{Point, Rect};
pub use metrics::UsedBoxMetrics;
pub use text::{SnapshotTextGlyph, SnapshotTextRun};
pub use text_geometry::SourceClusterBox;

/// A stable identity from the caller's source tree, independent of its position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub u64);

/// A node identity at a path in this immutable forest (root index, child index,
/// ...). Include text nodes in child indices; do not count element siblings only.
/// The caller preserves `id` when the same node moves to a different path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceIdentity {
    pub id: SourceId,
    pub path: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Element,
    Text,
}

/// Layout inputs are scoped to a single snapshot. The caller resolves live CSS,
/// fonts and images before calling; this API performs no network or script work.
pub struct SnapshotInputs<'a> {
    pub width: f32,
    pub height: f32,
    pub fonts: &'a FontRegistry,
    pub images: &'a ImageCtx<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotError {
    #[error("viewport dimensions must be finite and greater than zero")]
    InvalidViewport,
    #[error("fixed descendants of transformed ancestors require an unsupported containing block")]
    UnsupportedFixedContainingBlock,
    #[error("source ID {0:?} is assigned more than once")]
    DuplicateId(SourceId),
    #[error("source path {0:?} is assigned more than once")]
    DuplicatePath(Vec<usize>),
    #[error("source path {0:?} does not identify a node in this snapshot")]
    MissingPath(Vec<usize>),
}

/// The fragment category describes actual layout output, not guessed DOM boxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryKind {
    BorderBox,
    ImagePaint,
    TextLineContainer,
    Directive,
}

/// One native fragment. Indices follow native subtree paint order; the parent
/// index retains the tree. Synthetic/anonymous fragments have no source ID.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotFragment {
    pub index: usize,
    pub parent: Option<usize>,
    pub layout_node_id: NodeId,
    pub source_id: Option<SourceId>,
    /// Mapped source ancestors, outermost first, including flattened inlines.
    pub source_ancestors: Vec<SourceId>,
    pub kind: GeometryKind,
    /// Present only for actual border boxes, captured during layout resolution.
    pub used_box: Option<UsedBoxMetrics>,
    /// Actual pre-transform native fragment rectangle in document CSS pixels.
    pub rect: Rect,
    /// Parent × own transform, about each box's actual absolute origin.
    pub world_transform: [f32; 6],
    pub quad: [Point; 4],
    pub transformed_bounds: Rect,
}

/// A mapped source node can legitimately have zero fragments (`display:none`,
/// or an inline whose text was flattened). Descendant indices expose provenance
/// without manufacturing a border box for that node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotSource {
    pub id: SourceId,
    pub path: Vec<usize>,
    pub kind: SourceKind,
    /// Actual display:none exclusion, including an unmapped ancestor. A visible
    /// flattened/empty inline also has no box fragments, so absence alone cannot
    /// justify manufacturing a zero CSSOM rectangle.
    pub excluded_from_layout: bool,
    pub fragment_indices: Vec<usize>,
    pub descendant_fragment_indices: Vec<usize>,
}

pub struct LayoutSnapshot {
    galley: Fragment,
    fragments: Vec<SnapshotFragment>,
    sources: Vec<SnapshotSource>,
    source_index: BTreeMap<SourceId, usize>,
    viewport: (f32, f32),
    text_runs: Vec<SnapshotTextRun>,
}

impl LayoutSnapshot {
    /// Actual per-glyph source metadata; empty for the original box-only API.
    /// Actual glyph advance/font boxes, preserving source ranges and transforms.
    pub fn text_cluster_boxes(&self) -> Vec<SourceClusterBox> {
        text_geometry::cluster_boxes(self)
    }
    pub fn text_runs(&self) -> &[SnapshotTextRun] {
        &self.text_runs
    }
    pub fn galley(&self) -> &Fragment {
        &self.galley
    }
    pub fn fragments(&self) -> &[SnapshotFragment] {
        &self.fragments
    }
    pub fn sources(&self) -> &[SnapshotSource] {
        &self.sources
    }
    pub fn viewport(&self) -> (f32, f32) {
        self.viewport
    }
    pub fn source(&self, id: SourceId) -> Option<&SnapshotSource> {
        self.source_index.get(&id).map(|&i| &self.sources[i])
    }
}

/// Style the actual borrowed tree, preserving its forest and child paths. The
/// cascade must already include the caller's live stylesheet and viewport state.
pub fn snapshot_live_tree(
    nodes: &[Node],
    identities: &[SourceIdentity],
    cascade: &Cascade,
    inputs: &SnapshotInputs,
    diags: &mut Diagnostics,
) -> Result<LayoutSnapshot, SnapshotError> {
    validate_viewport(inputs)?;
    crate::style::set_layout_viewport(inputs.width, inputs.height);
    let styled = crate::style_tree(nodes, cascade);
    snapshot_styled_tree(&styled, identities, inputs, diags)
}

/// Capture an already styled live tree. All fragments are produced from one
/// box tree; there is no second allocation-order reconstruction for identity.
pub fn snapshot_styled_tree(
    styled: &[StyledNode],
    identities: &[SourceIdentity],
    inputs: &SnapshotInputs,
    diags: &mut Diagnostics,
) -> Result<LayoutSnapshot, SnapshotError> {
    capture_styled(styled, identities, inputs, diags, None).map(|(snapshot, _)| snapshot)
}

/// Failures specific to optional text provenance do not change the original API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextSnapshotError {
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    #[error("native text provenance exceeds its glyph or layout-ID budget")]
    Budget,
}

/// Capture real shaping clusters/UTF-16 offsets alongside native source fragments.
/// This API keeps original Fragment/GlyphRun output types and the PDF path intact.
pub fn snapshot_styled_tree_with_text(
    styled: &[StyledNode],
    identities: &[SourceIdentity],
    inputs: &SnapshotInputs,
    diags: &mut Diagnostics,
) -> Result<LayoutSnapshot, TextSnapshotError> {
    let (snapshot, exceeded) = capture_styled(styled, identities, inputs, diags, Some(false))?;
    if exceeded {
        Err(TextSnapshotError::Budget)
    } else {
        Ok(snapshot)
    }
}

/// Style a borrowed live forest and capture actual native glyph source metadata.
pub fn snapshot_live_tree_with_text(
    nodes: &[Node],
    identities: &[SourceIdentity],
    cascade: &Cascade,
    inputs: &SnapshotInputs,
    diags: &mut Diagnostics,
) -> Result<LayoutSnapshot, TextSnapshotError> {
    validate_viewport(inputs)?;
    crate::style::set_layout_viewport(inputs.width, inputs.height);
    let styled = crate::style_tree(nodes, cascade);
    snapshot_styled_tree_with_text(&styled, identities, inputs, diags)
}

/// Continuous HTML text layout with rounded native font metrics and real source
/// clusters. The original box-only/PDF and generic source APIs keep their policy.
pub fn snapshot_html_styled_tree_with_text(
    styled: &[StyledNode],
    identities: &[SourceIdentity],
    inputs: &SnapshotInputs,
    diags: &mut Diagnostics,
) -> Result<LayoutSnapshot, TextSnapshotError> {
    let (snapshot, exceeded) = capture_styled(styled, identities, inputs, diags, Some(true))?;
    if exceeded {
        Err(TextSnapshotError::Budget)
    } else {
        Ok(snapshot)
    }
}

/// Style a live forest using the continuous HTML text geometry policy.
pub fn snapshot_html_live_tree_with_text(
    nodes: &[Node],
    identities: &[SourceIdentity],
    cascade: &Cascade,
    inputs: &SnapshotInputs,
    diags: &mut Diagnostics,
) -> Result<LayoutSnapshot, TextSnapshotError> {
    validate_viewport(inputs)?;
    crate::style::set_layout_viewport(inputs.width, inputs.height);
    let styled = crate::style_tree(nodes, cascade);
    snapshot_html_styled_tree_with_text(&styled, identities, inputs, diags)
}

fn capture_styled(
    styled: &[StyledNode],
    identities: &[SourceIdentity],
    inputs: &SnapshotInputs,
    diags: &mut Diagnostics,
    html_text_metrics: Option<bool>,
) -> Result<(LayoutSnapshot, bool), SnapshotError> {
    validate_viewport(inputs)?;
    crate::style::set_layout_viewport(inputs.width, inputs.height);
    fixed_containing::validate(styled, false)?;
    let (build_sources, mut sources) = sources::prepare(styled, identities)?;
    let (tree, mut records, next_id) = boxgen::build_sourced_tree(styled, build_sources);
    let (galley, metrics, text) = block::layout_tree_live(
        &tree,
        (inputs.width, inputs.height),
        inputs.fonts,
        inputs.images,
        diags,
        html_text_metrics.map(|html| (next_id, html)),
    );
    text::alias_sources(text.as_ref(), &mut records);
    let fragments = geometry::collect(&galley, &records, &metrics);
    let source_index = sources.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
    sources::index_fragments(&mut sources, &source_index, &fragments);
    let exceeded = text.as_ref().is_some_and(|text| text.exceeded);
    let text_runs = text.map_or_else(Vec::new, |text| text.collect(&galley, &fragments));
    Ok((
        LayoutSnapshot {
            galley,
            fragments,
            sources,
            source_index,
            viewport: (inputs.width, inputs.height),
            text_runs,
        },
        exceeded,
    ))
}

fn validate_viewport(inputs: &SnapshotInputs) -> Result<(), SnapshotError> {
    for value in [inputs.width, inputs.height] {
        if !value.is_finite() || value <= 0.0 {
            return Err(SnapshotError::InvalidViewport);
        }
    }
    Ok(())
}
