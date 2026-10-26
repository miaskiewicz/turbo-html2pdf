//! The fragmenter's per-box facts for the body flow ([`FlowBoxes`],
//! miaskiewicz/turbo-html2pdf#32), recorded from the styled document: each element's
//! used `position`, vertical margins, paddings and insets and its relative paint shift,
//! resolved the way layout resolves them (`resolve_box_style`, the parent's font size).
//! Percentages resolve against the box's containing block: the content width of its
//! parent box in the laid-out galley (miaskiewicz/turbo-html2pdf#37 item 2; a box without
//! a source — an anonymous box — passes its own width down). Keyed by the box ids of the
//! recorded layout.

use std::collections::HashMap;

use turbo_dom::Handle;
use turbo_dom_layout::layout::fragment::Fragment;
use turbo_dom_layout::layout::value::{
    resolve_box_style, BoxStyle, Position, ResolveCtx, DEFAULT_FONT_SIZE,
};
use turbo_dom_layout::style::ComputedStyle;
use turbo_dom_layout::{NodeSources, StyledNode};

use crate::paginate::{BoxFacts, FlowBoxes};

/// Each styled element's computed style and its parent's font size, by source node.
type Styles<'s> = HashMap<Handle, (&'s ComputedStyle, f32)>;

/// The facts of every element box of the layout `galley` recorded from `styled`
/// (`sources`); `width` is the galley root's containing block (the page content width).
pub(crate) fn flow_boxes(
    styled: &[StyledNode],
    sources: &NodeSources,
    galley: &Fragment,
    width: f32,
) -> FlowBoxes {
    let mut styles = HashMap::new();
    collect(styled, DEFAULT_FONT_SIZE, &mut styles);
    let mut boxes = FlowBoxes::default();
    let lookup = Lookup {
        styles: &styles,
        sources,
    };
    lookup.visit(galley, width, &mut boxes);
    boxes
}

/// Record every styled element's style with the font size it inherits.
fn collect<'s>(nodes: &'s [StyledNode], parent_font: f32, out: &mut Styles<'s>) {
    for element in nodes.iter().filter_map(StyledNode::as_element) {
        let ctx = ResolveCtx {
            parent_font_size: parent_font,
            cb_width: 0.0,
        };
        let font = resolve_box_style(&element.style, ctx).font_size;
        if let Some(source) = element.source {
            out.insert(source, (&element.style, parent_font));
        }
        collect(&element.children, font, out);
    }
}

/// The styles and box sources the galley walk resolves against.
struct Lookup<'a, 's> {
    styles: &'a Styles<'s>,
    sources: &'a NodeSources,
}

impl Lookup<'_, '_> {
    /// Record `f`'s facts against its containing block width `cb`, then its children's
    /// against `f`'s content width.
    fn visit(&self, f: &Fragment, cb: f32, out: &mut FlowBoxes) {
        let inner = match self.style(f, cb) {
            Some(style) => {
                out.insert(f.node_id, facts(&style, cb));
                let border = style.border.widths();
                f.width - style.padding.horizontal() - border.horizontal()
            }
            None => f.width,
        };
        for child in &f.children {
            self.visit(child, inner.max(0.0), out);
        }
    }

    /// `f`'s resolved style when it is an element box.
    fn style(&self, f: &Fragment, cb: f32) -> Option<BoxStyle> {
        let handle = self.sources.elements.get(&f.node_id)?;
        let &(style, parent_font) = self.styles.get(handle)?;
        let ctx = ResolveCtx {
            parent_font_size: parent_font,
            cb_width: cb,
        };
        Some(resolve_box_style(style, ctx))
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
