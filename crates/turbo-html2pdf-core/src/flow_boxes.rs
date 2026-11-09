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
use turbo_dom_layout::layout::fragment::{Fragment, FragmentContent};
use turbo_dom_layout::layout::value::{
    parse_px, resolve_box_style, BoxStyle, LengthPct, Position, ResolveCtx, DEFAULT_FONT_SIZE,
};
use turbo_dom_layout::style::ComputedStyle;
use turbo_dom_layout::{NodeSources, StyledNode};

use crate::paginate::{BoxFacts, FlowBoxes, MulticolFacts};

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
    for &marker in sources.list_markers.keys() {
        boxes.insert_marker(marker);
    }
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
    ///
    /// A multicol container's in-flow children are column-strip fragments laid out by
    /// `turbo-dom-layout::layout::multicol`; recursing into them with the pre-layout
    /// widths would record facts against the wrong containing block (each column strip
    /// is their CB, not the container's content box) and the paginator does not read
    /// those facts. BUT the container may hold out-of-flow descendants
    /// (`position: absolute` / `fixed`) whose CB is the multicol container itself
    /// (CSS Positioned Layout 3 §4, the nearest positioned ancestor) — the paginator
    /// needs their facts so [`crate::paginate::walk::Walker::place_one`] routes them
    /// through [`crate::paginate::walk::Walker::place_out_of_flow`] rather than flowing
    /// them as blocks (R1 HIGH-1, miaskiewicz/turbo-html2pdf#52). So under a multicol
    /// we still descend into every kid, but the CB we pass down is the multicol's own
    /// content width `inner` (the correct CB for OOF descendants), not any column
    /// strip's `used_width`.
    ///
    /// A multicol container emits pre-layout [`FragmentContent::ColumnRule`] fragments
    /// with the container's own `node_id` (they are not element boxes). Those must not
    /// re-overwrite the container's [`BoxFacts`] during descent, or the rule's narrow
    /// width (4 px) would re-resolve `multicol_facts` against a 4-px content box and
    /// drive `used_width` to zero. Skip them before resolve — they carry no element of
    /// their own.
    fn visit(&self, f: &Fragment, cb: f32, out: &mut FlowBoxes) {
        if matches!(f.content, FragmentContent::ColumnRule { .. }) {
            return;
        }
        let (inner, _multicol) = match self.resolve(f, cb) {
            Some((style, computed)) => {
                let border = style.border.widths();
                let inner = f.width - style.padding.horizontal() - border.horizontal();
                let multicol = multicol_facts(computed, inner.max(0.0), style.font_size);
                let mut box_facts = facts(&style, cb);
                box_facts.multicol = multicol;
                out.insert(f.node_id, box_facts);
                (inner, multicol)
            }
            None => (f.width, None),
        };
        for child in &f.children {
            self.visit(child, inner.max(0.0), out);
        }
    }

    /// `f`'s resolved and raw computed style when it is an element box.
    fn resolve(&self, f: &Fragment, cb: f32) -> Option<(BoxStyle, &ComputedStyle)> {
        let handle = self.sources.elements.get(&f.node_id)?;
        let &(computed, parent_font) = self.styles.get(handle)?;
        let ctx = ResolveCtx {
            parent_font_size: parent_font,
            cb_width: cb,
        };
        Some((resolve_box_style(computed, ctx), computed))
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
        definite_height: style.height != LengthPct::Auto,
        multicol: None,
    }
}

/// Resolve [`MulticolFacts`] for a box whose computed style opens a multi-column
/// formatting context (CSS Multicol 1 §7.1 "Pseudo-algorithm";
/// miaskiewicz/turbo-html2pdf#52): a non-`auto` `column-count` fixes the count,
/// else `column-width` picks `floor((cw + gap) / (width + gap))` columns; the used
/// width is then `(cw - (count - 1) * gap) / count`. `None` for an auto-both box.
fn multicol_facts(style: &ComputedStyle, cw: f32, fs: f32) -> Option<MulticolFacts> {
    let count_hint = parse_column_count(style);
    let width_hint = parse_column_width(style, fs);
    let used_count = match (count_hint, width_hint) {
        (Some(n), _) => n,
        (None, Some(w)) => widen_count(cw, w, parse_column_gap(style, fs)),
        (None, None) => return None,
    }
    .max(1);
    let gap = parse_column_gap(style, fs);
    let used_width = used_column_width(cw, gap, used_count);
    Some(MulticolFacts {
        used_count,
        used_width,
        gap,
    })
}

/// Parse `column-count` as an integer (CSS Multicol 1 §4.1): `auto` / absent / 0 /
/// unparsable → `None`.
fn parse_column_count(style: &ComputedStyle) -> Option<u32> {
    let raw = style.get("column-count")?.trim();
    if raw == "auto" || raw.is_empty() {
        return None;
    }
    raw.parse::<u32>().ok().filter(|n| *n >= 1)
}

/// Parse `column-width` as a CSS length in px (CSS Multicol 1 §4.2): `auto` /
/// absent / non-positive / unparsable → `None`.
fn parse_column_width(style: &ComputedStyle, fs: f32) -> Option<f32> {
    let raw = style.get("column-width")?.trim();
    if raw == "auto" || raw.is_empty() {
        return None;
    }
    parse_px(raw, fs).filter(|w| *w > 0.0)
}

/// Parse `column-gap` as a CSS length in px, with `normal` ≈ `1em` (CSS Multicol
/// 1 §5.1, matching Chrome 156's font-size basis).
fn parse_column_gap(style: &ComputedStyle, fs: f32) -> f32 {
    let raw = style.get("column-gap").map_or("normal", str::trim);
    if raw == "normal" {
        return fs;
    }
    parse_px(raw, fs).unwrap_or(fs)
}

/// `floor((cw + gap) / (width + gap))`, with the content-box edge included so a
/// container exactly as wide as one column admits it (CSS Multicol 1 §7.1).
fn widen_count(cw: f32, width: f32, gap: f32) -> u32 {
    if width <= 0.0 {
        return 1;
    }
    let raw = ((cw + gap) / (width + gap)).floor();
    if raw.is_finite() && raw >= 1.0 {
        raw as u32
    } else {
        1
    }
}

/// `(cw - (count - 1) * gap) / count`, clamped at zero so a container narrower
/// than its gaps still measures (matches `turbo-dom-layout::multicol`).
fn used_column_width(cw: f32, gap: f32, count: u32) -> f32 {
    let gaps_total = gap * (count.saturating_sub(1) as f32);
    ((cw - gaps_total) / count.max(1) as f32).max(0.0)
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
