//! The fragmenter's per-box facts for the body flow ([`FlowBoxes`],
//! miaskiewicz/turbo-html2pdf#32), recorded from the styled document: each element's
//! used `position`, vertical margins, paddings and insets and its relative paint shift,
//! resolved the way layout resolves them (`resolve_box_style`, the parent's font size).
//! Percentages resolve against the flow's content width (the containing blocks of the
//! body's boxes are not known here; a percentage vertical margin or padding inside a
//! narrower box is approximate). Keyed by the box ids of the recorded layout.

use std::collections::HashMap;

use turbo_dom::Handle;
use turbo_dom_layout::layout::value::{
    resolve_box_style, BoxStyle, Position, ResolveCtx, DEFAULT_FONT_SIZE,
};
use turbo_dom_layout::{NodeSources, StyledNode};

use crate::paginate::{BoxFacts, FlowBoxes};

/// The facts of every element box of a layout recorded from `styled` (`sources`), with
/// percentages against `width`.
pub(crate) fn flow_boxes(styled: &[StyledNode], sources: &NodeSources, width: f32) -> FlowBoxes {
    let mut by_source = HashMap::new();
    collect(styled, DEFAULT_FONT_SIZE, width, &mut by_source);
    let mut boxes = FlowBoxes::default();
    for (&id, handle) in &sources.elements {
        if let Some(facts) = by_source.get(handle) {
            boxes.insert(id, *facts);
        }
    }
    boxes
}

fn collect(
    nodes: &[StyledNode],
    parent_font: f32,
    width: f32,
    out: &mut HashMap<Handle, BoxFacts>,
) {
    for element in nodes.iter().filter_map(StyledNode::as_element) {
        let ctx = ResolveCtx {
            parent_font_size: parent_font,
            cb_width: width,
        };
        let style = resolve_box_style(&element.style, ctx);
        if let Some(source) = element.source {
            out.insert(source, facts(&style, width));
        }
        collect(&element.children, style.font_size, width, out);
    }
}

/// One box's facts from its resolved style.
fn facts(style: &BoxStyle, width: f32) -> BoxFacts {
    BoxFacts {
        position: style.position,
        margin_top: style.margin.top,
        margin_bottom: style.margin.bottom,
        padding_top: style.padding.top,
        padding_bottom: style.padding.bottom,
        inset_top: style.inset_top,
        inset_bottom: style.inset_bottom,
        shift: relative_shift(style, width),
    }
}

/// The vertical paint shift layout applies to a `relative`/`sticky` box: `top`, else
/// minus `bottom` (both against the containing block's width, as layout resolves them).
fn relative_shift(style: &BoxStyle, width: f32) -> f32 {
    if !matches!(style.position, Position::Relative | Position::Sticky) {
        return 0.0;
    }
    style
        .inset_top
        .resolve(width)
        .or_else(|| style.inset_bottom.resolve(width).map(|b| -b))
        .unwrap_or(0.0)
}
