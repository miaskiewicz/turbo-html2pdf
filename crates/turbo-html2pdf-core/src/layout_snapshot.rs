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
//! Range rectangle. Original UTF-16 offsets, caret boxes, scrollbars, clipping,
//! and fragmented inline border boxes require further layout support.

mod geometry;
mod metrics;
pub(crate) mod sources;

use crate::layout::{block, boxgen};
use crate::{Cascade, Diagnostics, FontRegistry, Fragment, ImageCtx, Node, NodeId, StyledNode};
use std::collections::BTreeMap;

pub use geometry::{Point, Rect};
pub use metrics::UsedBoxMetrics;

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
    pub fragment_indices: Vec<usize>,
    pub descendant_fragment_indices: Vec<usize>,
}

pub struct LayoutSnapshot {
    galley: Fragment,
    fragments: Vec<SnapshotFragment>,
    sources: Vec<SnapshotSource>,
    source_index: BTreeMap<SourceId, usize>,
    viewport: (f32, f32),
}

impl LayoutSnapshot {
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
    validate_viewport(inputs)?;
    crate::style::set_layout_viewport(inputs.width, inputs.height);
    let (build_sources, mut sources) = sources::prepare(styled, identities)?;
    let (tree, records) = boxgen::build_sourced_tree(styled, build_sources);
    let (galley, metrics) = block::layout_tree_live(
        &tree,
        inputs.width,
        inputs.height,
        inputs.fonts,
        inputs.images,
        diags,
    );
    let fragments = geometry::collect(&galley, &records, &metrics);
    let source_index = sources.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
    sources::index_fragments(&mut sources, &source_index, &fragments);
    Ok(LayoutSnapshot {
        galley,
        fragments,
        sources,
        source_index,
        viewport: (inputs.width, inputs.height),
    })
}

fn validate_viewport(inputs: &SnapshotInputs) -> Result<(), SnapshotError> {
    for value in [inputs.width, inputs.height] {
        if !value.is_finite() || value <= 0.0 {
            return Err(SnapshotError::InvalidViewport);
        }
    }
    Ok(())
}
