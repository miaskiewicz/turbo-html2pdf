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
mod hit_geometry;
mod metrics;
mod range_projection;
pub(crate) mod sources;
mod svg_viewports;
pub(crate) mod text;
mod text_caret_edges;
mod text_carets;
mod text_finite;
mod text_geometry;
mod text_ranges;
pub(crate) mod text_units;

use crate::layout::{block, boxgen};
use crate::{Cascade, Diagnostics, FontRegistry, Fragment, ImageCtx, Node, NodeId, StyledNode};
use std::collections::BTreeMap;

pub use geometry::{Point, Rect};
pub use metrics::UsedBoxMetrics;
pub use svg_viewports::SvgSnapshotError;
pub use text::{SnapshotTextGlyph, SnapshotTextRun, SnapshotTextSource};
pub use text_geometry::SourceClusterBox;
pub use text_ranges::{SourceRangeBox, TextGeometryError};

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

/// Original native DOM code units for one caller-owned text identity. The ordinary
/// Node/StyledNode string is its scalar projection; these units retain lone values.
#[derive(Debug, Clone, Copy)]
pub struct SourceTextUnits<'a> {
    pub source_id: SourceId,
    pub units: &'a [u16],
}

/// Native HTML source inputs; SVG identities and physical scale keep their existing
/// meaning. This additive route does not change ordinary scalar/PDF entry points.
pub struct NativeTextOptions<'a> {
    pub original_text: &'a [SourceTextUnits<'a>],
    pub svg_viewports: &'a [SourceId],
    pub layout_scale: f32,
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
    #[error("native HTML viewport dimensions must be finite and non-negative")]
    InvalidNativeViewport,
    #[error("original UTF-16 text source {0:?} is invalid or duplicated")]
    InvalidOriginalText(SourceId),
    #[error("original UTF-16 text inputs exceed the source unit budget")]
    OriginalTextBudget,
    #[error("fixed descendants of transformed ancestors require an unsupported containing block")]
    UnsupportedFixedContainingBlock,
    #[error("native positioned/floating block-in-inline splitting is unsupported")]
    UnsupportedInlineBlockSplit,
    #[error("native ordinary inline top/bottom/middle alignment is unsupported")]
    UnsupportedInlineAlignment,
    #[error("native ordinary inline vertical/RTL writing is unsupported")]
    UnsupportedInlineWriting,
    #[error("native ordinary inline absolute/fixed/sticky positioning is unsupported")]
    UnsupportedInlinePosition,
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
    /// Actual associated fragments. A split native inline also references the
    /// lifted block's existing border fragment; its source_id remains the block.
    pub fragment_indices: Vec<usize>,
    pub descendant_fragment_indices: Vec<usize>,
}

pub struct LayoutSnapshot {
    galley: Fragment,
    fragments: Vec<SnapshotFragment>,
    native_transforms: Vec<[f64; 6]>,
    native_hit_transforms: Vec<[f64; 6]>,
    range_projection: Vec<range_projection::Projection>,
    sources: Vec<SnapshotSource>,
    source_index: BTreeMap<SourceId, usize>,
    viewport: (f32, f32),
    text_runs: Vec<SnapshotTextRun>,
    text_sources: Vec<SnapshotTextSource>,
    original_text_units: BTreeMap<SourceId, std::sync::Arc<[u16]>>,
    collapsed_text_sources: std::collections::BTreeSet<SourceId>,
    html_layout_scale: Option<f32>,
    unsupported_svg_sources: std::collections::BTreeSet<SourceId>,
}

impl LayoutSnapshot {
    /// Global native affine coefficients retain double precision. Native box and
    /// text-selection quads additionally preserve each flat FloatQuad boundary;
    /// ordinary inline quads use this accumulated matrix. The legacy fragment
    /// matrix remains available with its original float type.
    pub fn native_world_transform(&self, fragment_index: usize) -> Option<[f64; 6]> {
        self.native_transforms.get(fragment_index).copied()
    }

    /// Actual HTML paint/hit scopes retain pixel-snapped origins separately from
    /// layout geometry. Generic snapshots use the layout transform unchanged.
    pub fn native_hit_transform(&self, fragment_index: usize) -> Option<[f64; 6]> {
        self.native_hit_transforms.get(fragment_index).copied()
    }

    /// SVG shape/nested-viewport identities stay present, but this viewport-only
    /// producer cannot supply their geometry. Absence is never a zero rectangle.
    pub fn unsupported_svg_source_geometry(&self, source: SourceId) -> bool {
        self.unsupported_svg_sources.contains(&source)
    }

    /// Resolve a candidate text source to a real placed cluster edge. The caller
    /// chooses a visible hit-test source; this does not perform DOM hit testing.
    /// Finite invertible affine transforms use actual native paint/hit scopes;
    /// degenerate transforms fail rather than guessing a caret.
    pub fn text_caret_offset_from_point(
        &self,
        source: SourceId,
        point: Point,
    ) -> Result<usize, TextGeometryError> {
        text_carets::offset(self, source, point)
    }

    /// Select one original text source using actual placed shaper cells. Partial
    /// clusters keep their full native cell; no interior caret is interpolated.
    /// Unplaced or unresolved sources fail explicitly instead of returning zeros.
    pub fn text_range_boxes(
        &self,
        source: SourceId,
        start: usize,
        end: usize,
    ) -> Result<Vec<SourceRangeBox>, TextGeometryError> {
        text_ranges::ranges(self, source, start, end)
    }

    pub fn text_original_units(&self, source: SourceId) -> Option<&[u16]> {
        self.original_text_units
            .get(&source)
            .map(std::sync::Arc::as_ref)
    }

    /// Actual glyph advance/font boxes, preserving source ranges and transforms.
    pub fn text_cluster_boxes(&self) -> Vec<SourceClusterBox> {
        text_geometry::cluster_boxes(self)
    }
    /// Capture only one source's real cluster boxes without materializing the
    /// rest of the document's glyph geometry. Unknown sources produce no cells.
    pub fn text_cluster_boxes_for_source(&self, source: SourceId) -> Vec<SourceClusterBox> {
        text_geometry::source_cluster_boxes(self, source)
    }
    /// Actual per-glyph source metadata; empty for the original box-only API.
    pub fn text_runs(&self) -> &[SnapshotTextRun] {
        &self.text_runs
    }
    /// Mapped native text before whitespace collapse; empty for box-only snapshots.
    pub fn text_sources(&self) -> &[SnapshotTextSource] {
        &self.text_sources
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
    #[error("native text provenance exceeds its source-byte, glyph or layout-ID budget")]
    Budget,
    #[error("native text provenance contains unrepresentable geometry")]
    NonFiniteGeometry,
}

fn checked_text_capture(
    snapshot: LayoutSnapshot,
    exceeded: bool,
) -> Result<LayoutSnapshot, TextSnapshotError> {
    if exceeded {
        Err(TextSnapshotError::Budget)
    } else if !text_finite::valid(&snapshot) {
        Err(TextSnapshotError::NonFiniteGeometry)
    } else {
        Ok(snapshot)
    }
}

/// Capture real shaping clusters/UTF-16 offsets alongside native source fragments.
/// This API keeps original Fragment/GlyphRun output types and the PDF path intact.
pub fn snapshot_styled_tree_with_text(
    styled: &[StyledNode],
    identities: &[SourceIdentity],
    inputs: &SnapshotInputs,
    diags: &mut Diagnostics,
) -> Result<LayoutSnapshot, TextSnapshotError> {
    validate_viewport(inputs)?;
    text::validate_source_budget(styled)?;
    let (snapshot, exceeded) = capture_styled(styled, identities, inputs, diags, Some(false))?;
    checked_text_capture(snapshot, exceeded)
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
    validate_native_viewport(inputs)?;
    text::validate_source_budget(styled)?;
    let (snapshot, exceeded) = capture_styled(styled, identities, inputs, diags, Some(true))?;
    checked_text_capture(snapshot, exceeded)
}

/// Style a live forest using the continuous HTML text geometry policy.
pub fn snapshot_html_live_tree_with_text(
    nodes: &[Node],
    identities: &[SourceIdentity],
    cascade: &Cascade,
    inputs: &SnapshotInputs,
    diags: &mut Diagnostics,
) -> Result<LayoutSnapshot, TextSnapshotError> {
    validate_native_viewport(inputs)?;
    crate::style::set_layout_viewport(inputs.width, inputs.height);
    let styled = crate::style_tree(nodes, cascade);
    snapshot_html_styled_tree_with_text(&styled, identities, inputs, diags)
}

/// Capture genuine outer-SVG viewport boxes and native HTML text. The caller
/// declares identities whose native namespace is SVG; all descendant identities
/// remain present but their SVG geometry is explicitly unsupported.
pub fn snapshot_html_styled_tree_with_svg_viewports(
    styled: &[StyledNode],
    identities: &[SourceIdentity],
    viewports: &[SourceId],
    layout_scale: f32,
    inputs: &SnapshotInputs,
    diags: &mut Diagnostics,
) -> Result<LayoutSnapshot, SvgSnapshotError> {
    validate_native_viewport(inputs)?;
    text::validate_source_budget(styled)?;
    let plan = svg_viewports::prepare(styled, identities, viewports, layout_scale)?;
    let (snapshot, exceeded) = capture_styled_with_plan(
        styled,
        identities,
        inputs,
        diags,
        Some(true),
        Some(plan),
        &[],
    )?;
    checked_svg_capture(snapshot, exceeded)
}

/// Capture native original-unit text using the same live HTML/SVG placement seam.
pub fn snapshot_html_styled_tree_with_native_text(
    styled: &[StyledNode],
    identities: &[SourceIdentity],
    options: &NativeTextOptions<'_>,
    inputs: &SnapshotInputs,
    diags: &mut Diagnostics,
) -> Result<LayoutSnapshot, SvgSnapshotError> {
    validate_native_viewport(inputs)?;
    text::validate_source_budget(styled)?;
    let plan = svg_viewports::prepare(
        styled,
        identities,
        options.svg_viewports,
        options.layout_scale,
    )?;
    let (snapshot, exceeded) = capture_styled_with_plan(
        styled,
        identities,
        inputs,
        diags,
        Some(true),
        Some(plan),
        options.original_text,
    )?;
    checked_svg_capture(snapshot, exceeded)
}

pub fn snapshot_html_live_tree_with_native_text(
    nodes: &[Node],
    identities: &[SourceIdentity],
    cascade: &Cascade,
    options: &NativeTextOptions<'_>,
    inputs: &SnapshotInputs,
    diags: &mut Diagnostics,
) -> Result<LayoutSnapshot, SvgSnapshotError> {
    validate_native_viewport(inputs)?;
    crate::style::set_layout_viewport(inputs.width, inputs.height);
    let styled = crate::style_tree(nodes, cascade);
    snapshot_html_styled_tree_with_native_text(&styled, identities, options, inputs, diags)
}

fn checked_svg_capture(
    snapshot: LayoutSnapshot,
    exceeded: bool,
) -> Result<LayoutSnapshot, SvgSnapshotError> {
    checked_text_capture(snapshot, exceeded).map_err(Into::into)
}

/// Style the complete caller-owned forest without rewriting/removing SVG nodes.
pub fn snapshot_html_live_tree_with_svg_viewports(
    nodes: &[Node],
    identities: &[SourceIdentity],
    viewports: &[SourceId],
    layout_scale: f32,
    cascade: &Cascade,
    inputs: &SnapshotInputs,
    diags: &mut Diagnostics,
) -> Result<LayoutSnapshot, SvgSnapshotError> {
    validate_native_viewport(inputs)?;
    crate::style::set_layout_viewport(inputs.width, inputs.height);
    let styled = crate::style_tree(nodes, cascade);
    snapshot_html_styled_tree_with_svg_viewports(
        &styled,
        identities,
        viewports,
        layout_scale,
        inputs,
        diags,
    )
}

fn capture_styled(
    styled: &[StyledNode],
    identities: &[SourceIdentity],
    inputs: &SnapshotInputs,
    diags: &mut Diagnostics,
    html_text_metrics: Option<bool>,
) -> Result<(LayoutSnapshot, bool), SnapshotError> {
    capture_styled_with_plan(
        styled,
        identities,
        inputs,
        diags,
        html_text_metrics,
        None,
        &[],
    )
}

fn validate_native_context(
    styled: &[StyledNode],
    html_text_metrics: Option<bool>,
) -> Result<(), SnapshotError> {
    fixed_containing::validate(styled, false)?;
    if html_text_metrics == Some(true) {
        crate::layout::inline_boxes::validate(styled, false)?;
    }
    Ok(())
}
fn validate_capture_viewport(
    inputs: &SnapshotInputs,
    html: Option<bool>,
) -> Result<(), SnapshotError> {
    if html == Some(true) {
        validate_native_viewport(inputs)
    } else {
        validate_viewport(inputs)
    }
}

fn capture_styled_with_plan(
    styled: &[StyledNode],
    identities: &[SourceIdentity],
    inputs: &SnapshotInputs,
    diags: &mut Diagnostics,
    html_text_metrics: Option<bool>,
    plan: Option<svg_viewports::Plan>,
    original_text: &[SourceTextUnits<'_>],
) -> Result<(LayoutSnapshot, bool), SnapshotError> {
    validate_capture_viewport(inputs, html_text_metrics)?;
    crate::style::set_layout_viewport(inputs.width, inputs.height);
    validate_native_context(styled, html_text_metrics)?;
    let (mut build_sources, mut sources) = sources::prepare(
        styled,
        identities,
        html_text_metrics.is_some(),
        html_text_metrics == Some(true),
    )?;
    build_sources.inline_borders = html_text_metrics == Some(true);
    let (font_layout_scale, unsupported_svg_sources) = apply_native_plan(&mut build_sources, plan);
    let text_sources = std::mem::take(&mut build_sources.text_sources);
    let original_text_units = text_units::prepare(&text_sources, original_text)?;
    let (tree, mut records, next_id) = boxgen::build_sourced_tree(styled, build_sources);
    let native_units =
        text_units::native_nodes(&records, &original_text_units, !original_text.is_empty());
    let (galley, metrics, text, transforms) = block::layout_tree_live(
        &tree,
        (inputs.width, inputs.height),
        inputs.fonts,
        inputs.images,
        diags,
        block::TextLayoutPolicy {
            start: html_text_metrics.map(|html| (next_id, html, font_layout_scale)),
            original_units: native_units,
        },
    );
    text::alias_sources(text.as_ref(), &mut records);
    let collapsed_text_sources = text
        .as_ref()
        .into_iter()
        .flat_map(|capture| capture.collapsed_sources.iter())
        .filter_map(|node| records.get(node).and_then(|record| record.id))
        .collect();
    let (fragments, native_transforms, native_hit_transforms) =
        geometry::collect(&galley, &records, &metrics, &transforms, font_layout_scale);
    let range_projection =
        range_projection::collect(&fragments, &transforms, font_layout_scale.is_some());
    let source_index = sources.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
    sources::index_fragments(&mut sources, &source_index, &fragments, &records);
    let exceeded = text.as_ref().is_some_and(|text| text.exceeded);
    let text_runs = text.map_or_else(Vec::new, |text| text.collect(&galley, &fragments));
    let mut snapshot = LayoutSnapshot {
        galley,
        fragments,
        native_transforms,
        native_hit_transforms,
        range_projection,
        sources,
        source_index,
        viewport: (inputs.width, inputs.height),
        text_runs,
        text_sources,
        original_text_units,
        collapsed_text_sources,
        html_layout_scale: font_layout_scale,
        unsupported_svg_sources,
    };
    range_projection::place_boxes(&mut snapshot);
    Ok((snapshot, exceeded))
}

fn apply_native_plan(
    sources: &mut sources::BuildSources,
    plan: Option<svg_viewports::Plan>,
) -> (Option<f32>, std::collections::BTreeSet<SourceId>) {
    match plan {
        Some(plan) => {
            sources.svg_viewports = plan.keys;
            sources.svg_layout_scale = plan.layout_scale;
            (Some(plan.layout_scale), plan.unsupported)
        }
        None => (None, Default::default()),
    }
}

fn validate_native_viewport(inputs: &SnapshotInputs) -> Result<(), SnapshotError> {
    for value in [inputs.width, inputs.height] {
        if !value.is_finite() || value < 0.0 {
            return Err(SnapshotError::InvalidNativeViewport);
        }
    }
    Ok(())
}

fn validate_viewport(inputs: &SnapshotInputs) -> Result<(), SnapshotError> {
    for value in [inputs.width, inputs.height] {
        if !value.is_finite() || value <= 0.0 {
            return Err(SnapshotError::InvalidViewport);
        }
    }
    Ok(())
}

#[cfg(test)]
mod text_budget_contracts {
    #[test]
    fn native_layout_identity_exhaustion_cannot_publish_partial_text_capture() {
        use super::*;

        use crate::{build_cascade, Diagnostics, FontRegistry, ImageCtx, NoImages, Node};
        let fonts = FontRegistry::new();
        let images = ImageCtx {
            resolver: &NoImages,
            body_height: None,
        };
        let nodes = [Node::Text("ab".into())];
        let snapshot = snapshot_html_live_tree_with_text(
            &nodes,
            &[],
            &build_cascade("", "", Default::default()),
            &SnapshotInputs {
                width: 80.0,
                height: 40.0,
                fonts: &fonts,
                images: &images,
            },
            &mut Diagnostics::default(),
        )
        .unwrap();
        let run = &snapshot.text_runs()[0];
        let mut capture = text::TextCapture::new(u32::MAX);
        capture.record(
            snapshot.fragments()[run.fragment_index].layout_node_id,
            &[run.glyphs[0].source],
            run.font_box,
        );
        assert!(capture.exceeded);
        assert!(matches!(
            checked_text_capture(snapshot, capture.exceeded),
            Err(TextSnapshotError::Budget)
        ));
    }
}
