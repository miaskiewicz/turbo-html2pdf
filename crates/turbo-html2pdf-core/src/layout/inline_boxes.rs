//! Native HTML inline boundaries survive flattening and line breaking. Border
//! fragments use the inline's own font box, not a union of descendant glyphs.
use super::block::Ctx;
use super::boxgen::InlineItem;
use super::fragment::{Fragment, FragmentContent, NodeId};
use super::inline::{InlineEdge, ParagraphLayout};
use super::value::{resolve_box_style, BoxStyle, Display, ResolveCtx};
use crate::layout_snapshot::UsedBoxMetrics;
use crate::ComputedStyle;
use std::collections::HashMap;

pub(crate) fn atomic_element(element: &crate::StyledElement) -> bool {
    matches!(&element.tag,crate::Tag::Html(name) if matches!(name.as_str(),"img"|"canvas"|"iframe"|"object"|"embed"|"input"|"select"|"textarea"|"button"|"video"|"audio"|"svg"))
}
pub(crate) fn validate(
    nodes: &[crate::StyledNode],
    inside: bool,
) -> Result<(), crate::layout_snapshot::SnapshotError> {
    for node in nodes {
        if let crate::StyledNode::Element(element) = node {
            validate_element(element, inside)?;
        }
    }
    Ok(())
}
fn validate_element(
    element: &crate::StyledElement,
    inside: bool,
) -> Result<(), crate::layout_snapshot::SnapshotError> {
    let display = super::value::display_of(&element.style);
    if display == Display::None {
        return Ok(());
    }
    if matches!(&element.tag,crate::Tag::Html(name) if name=="br") {
        return Ok(());
    }
    if element.style.get("display") == Some("contents") {
        return validate(&element.children, inside);
    }
    validate_rendered_element(element, display, inside)
}
fn validate_rendered_element(
    element: &crate::StyledElement,
    display: Display,
    inside: bool,
) -> Result<(), crate::layout_snapshot::SnapshotError> {
    let ordinary = ordinary_inline(element, display);
    validate_context(display, inside, ordinary, &element.style)?;
    validate_displaced_split(element, ordinary)?;
    validate(&element.children, inline_context(display, inside, ordinary))
}
fn validate_displaced_split(
    element: &crate::StyledElement,
    ordinary: bool,
) -> Result<(), crate::layout_snapshot::SnapshotError> {
    if ordinary && displaced_inline(&element.style) && contains_block(&element.children) {
        return Err(crate::layout_snapshot::SnapshotError::UnsupportedInlineBlockSplit);
    }
    Ok(())
}
fn contains_block(nodes: &[crate::StyledNode]) -> bool {
    let mut found = false;
    super::contents::visit(nodes, &ComputedStyle::default(), &mut |node, _| {
        found |= block_in_inline_node(node);
    });
    found
}
fn block_in_inline_node(node: &crate::StyledNode) -> bool {
    let crate::StyledNode::Element(element) = node else {
        return false;
    };
    let display = super::value::display_of(&element.style);
    if display == Display::None {
        return false;
    }
    if is_block_split(display) {
        return true;
    }
    display == Display::Inline && !atomic_element(element) && contains_block(&element.children)
}
fn displaced_inline(style: &ComputedStyle) -> bool {
    style.get("position") == Some("relative")
        || style.get("float").is_some_and(|value| value != "none")
}

fn ordinary_inline(element: &crate::StyledElement, display: Display) -> bool {
    display == Display::Inline && !atomic_element(element)
}
fn inline_context(display: Display, inside: bool, ordinary: bool) -> bool {
    matches!(display, Display::Inline) && (inside || ordinary)
}
fn validate_context(
    display: Display,
    inside: bool,
    ordinary: bool,
    style: &ComputedStyle,
) -> Result<(), crate::layout_snapshot::SnapshotError> {
    if unsupported_block_split(display, inside, style) {
        return Err(crate::layout_snapshot::SnapshotError::UnsupportedInlineBlockSplit);
    }
    if ordinary {
        validate_inline_style(style)?;
    }
    Ok(())
}
fn unsupported_block_split(display: Display, inside: bool, style: &ComputedStyle) -> bool {
    inside && is_block_split(display) && displaced_block(style)
}
fn displaced_block(style: &ComputedStyle) -> bool {
    matches!(style.get("position"), Some("absolute" | "fixed" | "sticky"))
        || style.get("float").is_some_and(|value| value != "none")
}
fn is_block_split(display: Display) -> bool {
    !matches!(display, Display::Inline | Display::InlineBlock)
}
fn validate_inline_style(
    style: &ComputedStyle,
) -> Result<(), crate::layout_snapshot::SnapshotError> {
    use crate::layout_snapshot::SnapshotError;
    if matches!(
        style.get("vertical-align"),
        Some("top" | "bottom" | "middle")
    ) {
        return Err(SnapshotError::UnsupportedInlineAlignment);
    }
    if unsupported_writing(style) {
        return Err(SnapshotError::UnsupportedInlineWriting);
    }
    if matches!(style.get("position"), Some("absolute" | "fixed" | "sticky")) {
        return Err(SnapshotError::UnsupportedInlinePosition);
    }
    Ok(())
}
fn unsupported_writing(style: &ComputedStyle) -> bool {
    style.get("direction") == Some("rtl")
        || style
            .get("writing-mode")
            .is_some_and(|value| value != "horizontal-tb")
}

fn style(style: &ComputedStyle, font: f32, width: f32, scale: Option<f32>) -> BoxStyle {
    let mut used = resolve_box_style(
        style,
        ResolveCtx {
            parent_font_size: font,
            cb_width: width,
        },
    );
    super::value::quantize_html_box_style(&mut used, scale);
    used
}

pub(super) fn edge(
    node_id: NodeId,
    source: &ComputedStyle,
    start: bool,
    font: f32,
    width: f32,
    ctx: &Ctx,
) -> Option<InlineEdge> {
    let used = style(source, font, width, ctx.html_font_scale);
    let families = used
        .font_families
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let face = ctx
        .fonts
        .select(&families, used.font_weight, used.italic)?
        .clone();
    let border = used.border.widths();
    let advance = if start {
        used.margin.left + border.left + used.padding.left
    } else {
        used.padding.right + border.right + used.margin.right
    };
    Some(InlineEdge {
        node_id,
        start,
        advance,
        face,
        font_size: used.font_size,
        line_height: used.line_height,
        contributes_metrics: true,
        valign: used.vertical_align,
        baseline_shift: 0.0,
    })
}

pub(super) struct SourcePlacement {
    pub offsets: HashMap<NodeId, (f32, f32)>,
    pub baselines: HashMap<NodeId, f32>,
}
struct ActiveInline {
    offset: (f32, f32),
    font: f32,
    shift: f32,
}
struct SourceOffsets {
    placement: SourcePlacement,
    stack: Vec<ActiveInline>,
}
impl SourceOffsets {
    fn current(&self) -> (f32, f32) {
        self.stack
            .last()
            .map(|value| value.offset)
            .unwrap_or_default()
    }
    fn shift(&self) -> f32 {
        self.stack
            .last()
            .map(|value| value.shift)
            .unwrap_or_default()
    }
    fn record(&mut self, id: NodeId) {
        self.placement.offsets.insert(id, self.current());
        self.placement.baselines.insert(id, self.shift());
    }
    fn boundary(
        &mut self,
        id: NodeId,
        source: &ComputedStyle,
        start: bool,
        font: f32,
        width: f32,
        ctx: &Ctx,
    ) {
        if !start {
            self.stack.pop();
            return;
        }
        let used = style(source, font, width, ctx.html_font_scale);
        let parent_font = self.stack.last().map_or(font, |value| value.font);
        let shift = self.shift()
            + native_baseline_shift(
                used.vertical_align,
                parent_font,
                ctx.html_font_scale.unwrap_or(1.0),
            );
        let (dx, dy) = super::block::flow_relative_offset(&used, width);
        let (px, py) = self.current();
        self.stack.push(ActiveInline {
            offset: (px + dx, py + dy),
            font: used.font_size,
            shift,
        });
        self.record(id);
    }
    fn item(&mut self, item: &InlineItem, font: f32, width: f32, ctx: &Ctx) {
        match item {
            InlineItem::Boundary {
                node_id,
                style,
                start,
            } => self.boundary(*node_id, style, *start, font, width, ctx),
            InlineItem::Text { node_id, .. } | InlineItem::SourceBreak { node_id, .. } => {
                self.record(*node_id)
            }
            InlineItem::Atomic(child) => {
                self.record(child.node_id);
                let used = style(&child.style, font, width, ctx.html_font_scale);
                let parent_font = self.stack.last().map_or(font, |parent| parent.font);
                self.placement.baselines.insert(
                    child.node_id,
                    self.shift()
                        + native_baseline_shift(
                            used.vertical_align,
                            parent_font,
                            ctx.html_font_scale.unwrap_or(1.0),
                        ),
                );
            }
            _ => {}
        }
    }
}
fn native_baseline_shift(align: super::value::VAlign, parent_font: f32, scale: f32) -> f32 {
    let units = (parent_font * scale * 64.0).round() as i64;
    match align {
        super::value::VAlign::Super => (units / 3 + 64) as f32 / 64.0 / scale,
        super::value::VAlign::Sub => -(units / 5 + 64) as f32 / 64.0 / scale,
        _ => 0.0,
    }
}
pub(super) fn positions(items: &[InlineItem], font: f32, width: f32, ctx: &Ctx) -> SourcePlacement {
    let mut source = SourceOffsets {
        placement: SourcePlacement {
            offsets: HashMap::new(),
            baselines: HashMap::new(),
        },
        stack: Vec::new(),
    };
    if ctx.html_font_metrics {
        for item in items {
            source.item(item, font, width, ctx);
        }
    }
    source.placement
}
pub(super) fn strut(bs: &BoxStyle, ctx: &Ctx) -> Option<(f32, f32)> {
    let families = bs
        .font_families
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let face = ctx.fonts.select(&families, bs.font_weight, bs.italic)?;
    let scale = ctx.html_font_scale.unwrap_or(1.0);
    let font = face.html_metrics(bs.font_size * scale);
    Some(super::inline::native_font_height(
        font.ascent / scale,
        font.descent / scale,
        bs.line_height
            .unwrap_or((font.ascent + font.descent + font.line_gap) / scale),
        0.0,
        scale,
    ))
}

pub(super) fn native_atoms(
    pieces: &mut [super::inline::Piece],
    atoms: &[Fragment],
    baselines: &HashMap<NodeId, f32>,
    native: bool,
) {
    if !native {
        return;
    }
    for piece in pieces {
        if let super::inline::Piece::Atom(atom) = piece {
            let shift = baselines
                .get(&atoms[atom.id].node_id)
                .copied()
                .unwrap_or_default();
            *piece = super::inline::Piece::NativeAtom(*atom, shift);
        }
    }
}
fn visible_inline_item(item: &InlineItem, font: f32, width: f32, ctx: &Ctx) -> bool {
    match item {
        InlineItem::Boundary {
            style: source,
            start: true,
            ..
        } => has_horizontal_edges(&style(source, font, width, ctx.html_font_scale)),
        InlineItem::Boundary { .. } => false,
        InlineItem::Text { text, style, .. } => {
            text.chars()
                .any(|character| !matches!(character, ' ' | '\t' | '\r' | '\n' | '\u{c}'))
                || preserved_space(style) && !text.is_empty()
        }
        _ => true,
    }
}
fn preserved_space(style: &ComputedStyle) -> bool {
    matches!(
        style.get("white-space"),
        Some("pre" | "pre-wrap" | "break-spaces")
    )
}
fn collapse_text_sources(items: &[InlineItem], ctx: &mut Ctx) {
    if let Some(capture) = &mut ctx.text_capture {
        for item in items {
            if let InlineItem::Text { node_id, .. } = item {
                capture.collapsed_sources.insert(*node_id);
            }
        }
    }
}
fn has_horizontal_edges(used: &BoxStyle) -> bool {
    let border = used.border.widths();
    [
        used.margin.left,
        used.margin.right,
        used.padding.left,
        used.padding.right,
        border.left,
        border.right,
    ]
    .iter()
    .any(|value| *value != 0.0)
}
pub(super) fn collapsible_run(items: &[InlineItem], font: f32, width: f32) -> bool {
    items.iter().all(|item| match item {
        InlineItem::Boundary { style: source, .. } => {
            !has_horizontal_edges(&style(source, font, width, None))
        }
        InlineItem::Text { text, style, .. } => {
            !preserved_space(style)
                && text
                    .chars()
                    .all(|c| matches!(c, ' ' | '\t' | '\r' | '\n' | '\u{c}'))
        }
        _ => false,
    })
}

pub(super) fn empty_fragments(
    items: &[InlineItem],
    font: f32,
    origin: (f32, f32),
    width: f32,
    placement: &SourcePlacement,
    ctx: &mut Ctx,
) -> Option<Vec<Fragment>> {
    if !ctx.html_font_metrics
        || items
            .iter()
            .any(|item| visible_inline_item(item, font, width, ctx))
    {
        return None;
    }
    collapse_text_sources(items, ctx);
    let mut out = Vec::new();
    for item in items {
        if let InlineItem::Boundary {
            node_id,
            start: true,
            ..
        } = item
        {
            out.push(empty_fragment(*node_id, origin, placement, ctx));
        }
    }
    Some(out)
}
fn empty_fragment(
    id: NodeId,
    origin: (f32, f32),
    placement: &SourcePlacement,
    ctx: &mut Ctx,
) -> Fragment {
    if let Some(metrics) = &mut ctx.used_metrics {
        metrics.insert(
            id,
            UsedBoxMetrics {
                inline_fragment: true,
                border: Default::default(),
                padding: Default::default(),
                margin: Default::default(),
                content_width: 0.0,
                content_height: 0.0,
                position: super::value::Position::Static,
                display: Display::Inline,
            },
        );
    }
    let (dx, dy) = placement.offsets.get(&id).copied().unwrap_or_default();
    Fragment::new(
        id,
        origin.0 + dx,
        origin.1 + dy,
        0.0,
        0.0,
        FragmentContent::Box {
            background: None,
            border: Default::default(),
            border_radius: 0.0,
            shadow: None,
            gradient: None,
            transform: None,
        },
    )
}
#[derive(Clone)]
struct Open {
    edge: InlineEdge,
    x: f32,
    first: bool,
    paused: bool,
}
struct InlineFragments<'a, 'b> {
    styles: HashMap<NodeId, BoxStyle>,
    open: Vec<Open>,
    out: Vec<Fragment>,
    ctx: &'a mut Ctx<'b>,
    origin: (f32, f32, f32),
    offsets: &'a HashMap<NodeId, (f32, f32)>,
}
impl InlineFragments<'_, '_> {
    fn line(&mut self, line: &super::inline::InlineLine, cx: f32, cy: f32) {
        self.origin = (cx + line.indent, cy + line.top, line.baseline);
        for value in &mut self.open {
            value.x = line.inline_origin;
            value.first = false;
        }
        for placed in &line.inline_edges {
            if placed.edge.start {
                self.start(placed);
            } else {
                self.end(placed);
            }
        }
        self.finish_line(line.inline_origin + line.width);
    }
    fn start(&mut self, placed: &super::inline::PlacedInlineEdge) {
        let parent = self
            .open
            .last()
            .filter(|parent| transparent(&self.styles[&parent.edge.node_id]))
            .cloned();
        if let Some(parent) = parent {
            self.flush(&parent, placed.x, false);
            if let Some(parent) = self.open.last_mut() {
                parent.paused = true;
            }
        }
        self.open.push(Open {
            edge: placed.edge.clone(),
            x: placed.x,
            first: true,
            paused: false,
        });
    }
    fn end(&mut self, placed: &super::inline::PlacedInlineEdge) {
        let Some(index) = self
            .open
            .iter()
            .rposition(|value| value.edge.node_id == placed.edge.node_id)
        else {
            return;
        };
        let value = self.open.remove(index);
        if !value.paused {
            self.flush(&value, placed.x + placed.edge.advance, true);
        }
        if let Some(parent) = self.open.last_mut().filter(|parent| parent.paused) {
            parent.x = placed.x + placed.edge.advance;
            parent.paused = false;
            parent.first = false;
        }
    }
    fn finish_line(&mut self, end: f32) {
        let active = self
            .open
            .iter()
            .filter(|value| !value.paused)
            .cloned()
            .collect::<Vec<_>>();
        for value in active {
            self.flush(&value, end, false);
        }
    }
    fn flush(&mut self, value: &Open, end: f32, last: bool) {
        let used = &self.styles[&value.edge.node_id];
        if empty_tail(value, end, used) {
            return;
        }
        let mut part = fragment(value, used, end, last, self.origin, self.ctx);
        let (dx, dy) = self.offsets.get(&part.node_id).copied().unwrap_or_default();
        part.translate(dx, dy);
        if skip_empty_part(&part, value, last, used) {
            return;
        }
        append_part(&mut self.out, part, &self.open, &self.styles, self.ctx);
    }
}
pub(super) fn fragments(
    layout: &ParagraphLayout,
    items: &[InlineItem],
    font: f32,
    origin: (f32, f32),
    width: f32,
    ctx: &mut Ctx,
    offsets: &HashMap<NodeId, (f32, f32)>,
) -> Vec<Fragment> {
    let styles = items
        .iter()
        .filter_map(|item| match item {
            InlineItem::Boundary {
                node_id,
                style: source,
                start: true,
            }
            | InlineItem::SourceBreak {
                node_id,
                style: source,
            } => Some((*node_id, style(source, font, width, ctx.html_font_scale))),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let mut builder = InlineFragments {
        styles,
        open: Vec::new(),
        out: Vec::new(),
        ctx,
        origin: (origin.0, origin.1, 0.0),
        offsets,
    };
    for line in &layout.lines {
        builder.line(line, origin.0, origin.1);
    }
    builder.out
}

fn skip_empty_part(part: &Fragment, value: &Open, last: bool, used: &BoxStyle) -> bool {
    part.width == 0.0 && transparent(used) && (!last || !value.first)
}
fn empty_tail(value: &Open, end: f32, used: &BoxStyle) -> bool {
    !value.first && transparent(used) && end <= value.x
}
fn transparent(used: &BoxStyle) -> bool {
    used.padding == super::value::Edges::default()
        && used.border.widths() == super::value::Edges::default()
        && used.background.is_none_or(|color| color.a == 0)
        && used.background_gradient.is_none()
        && used.box_shadow.is_none()
}
fn append_part(
    out: &mut Vec<Fragment>,
    part: Fragment,
    open: &[Open],
    styles: &HashMap<NodeId, BoxStyle>,
    ctx: &mut Ctx,
) {
    // A boxless inline's rect list consists of its own runs and the actual
    // descendant inline boxes; decoration establishes one enclosing box instead.
    for parent in open
        .iter()
        .rev()
        .filter(|parent| parent.edge.node_id != part.node_id)
        .take_while(|parent| parent.paused && transparent(&styles[&parent.edge.node_id]))
    {
        let mut inherited = part.clone();
        inherited.node_id = parent.edge.node_id;
        inherited.content = FragmentContent::Box {
            background: None,
            border: Default::default(),
            border_radius: 0.0,
            shadow: None,
            gradient: None,
            transform: None,
        };
        if let Some(metrics) = &mut ctx.used_metrics {
            metrics.entry(inherited.node_id).or_insert(UsedBoxMetrics {
                inline_fragment: true,
                border: Default::default(),
                padding: Default::default(),
                margin: styles[&inherited.node_id].margin,
                content_width: part.width,
                content_height: part.height,
                position: styles[&inherited.node_id].position,
                display: Display::Inline,
            });
        }
        out.push(inherited);
    }
    out.push(part);
}

fn fragment(
    open: &Open,
    used: &BoxStyle,
    end: f32,
    last: bool,
    origin: (f32, f32, f32),
    ctx: &mut Ctx,
) -> Fragment {
    let scale = ctx.html_font_scale.unwrap_or(1.0);
    let font = open.edge.face.html_metrics(open.edge.font_size * scale);
    let content_height = (font.ascent + font.descent) / scale;
    let mut border = used.border;
    let mut padding = used.padding;
    let x = open.x
        + if open.first {
            used.margin.left
        } else {
            border.left.width = 0;
            padding.left = 0.0;
            0.0
        };
    let end = end
        - if last {
            used.margin.right
        } else {
            border.right.width = 0;
            padding.right = 0.0;
            0.0
        };
    let border_width = border.widths();
    let height = content_height + padding.vertical() + border_width.vertical();
    let width = (end - x).max(0.0);
    let shift = open.edge.baseline_shift;
    let y = origin.1 + origin.2 - shift - font.ascent / scale - padding.top - border_width.top;
    if let Some(metrics) = &mut ctx.used_metrics {
        metrics.entry(open.edge.node_id).or_insert(UsedBoxMetrics {
            inline_fragment: true,
            border: border_width,
            padding,
            margin: used.margin,
            content_width: (width - padding.horizontal() - border_width.horizontal()).max(0.0),
            content_height,
            position: used.position,
            display: Display::Inline,
        });
    }
    Fragment::new(
        open.edge.node_id,
        origin.0 + x,
        y,
        width,
        height,
        FragmentContent::Box {
            background: used.background,
            border,
            border_radius: 0.0,
            shadow: used.box_shadow,
            gradient: used.background_gradient.clone(),
            transform: None,
        },
    )
}

#[cfg(test)]
mod defensive_contract_tests {
    use super::*;

    #[test]
    fn unmatched_closing_marker_does_not_create_an_element_fragment() {
        // Valid DOM capture emits balanced boundaries. A corrupt intermediate
        // closing marker must not acquire a synthetic source box.
        let fonts = crate::FontRegistry::new();
        let images = crate::ImageCtx {
            resolver: &crate::NoImages,
            body_height: None,
        };
        let mut diagnostics = crate::Diagnostics::default();
        let mut ctx = Ctx {
            fonts: &fonts,
            images: &images,
            diags: &mut diagnostics,
            abs_cb_x: 0.0,
            abs_cb_y: 0.0,
            abs_cb_w: 200.0,
            abs_cb_h: 100.0,
            abs_cb_h_known: true,
            cb_h: 100.0,
            cb_h_known: true,
            root_w: 200.0,
            root_h: 100.0,
            live_geometry: true,
            used_metrics: Some(HashMap::new()),
            text_capture: None,
            html_font_metrics: true,
            html_font_scale: Some(2.0),
            original_units: None,
            native_transforms: Some(HashMap::new()),
            floats: Vec::new(),
        };
        let edge = super::super::inline::PlacedInlineEdge {
            edge: InlineEdge {
                node_id: NodeId(7),
                start: false,
                advance: 3.0,
                face: fonts.select(&["sans-serif"], 400, false).unwrap().clone(),
                font_size: 16.0,
                line_height: Some(24.0),
                baseline_shift: 0.0,
                contributes_metrics: true,
                valign: super::super::value::VAlign::Baseline,
            },
            x: 20.0,
        };
        let offsets = HashMap::new();
        let mut projection = InlineFragments {
            styles: HashMap::new(),
            open: Vec::new(),
            out: Vec::new(),
            ctx: &mut ctx,
            origin: (0.0, 0.0, 18.0),
            offsets: &offsets,
        };
        projection.end(&edge);
        assert!(projection.out.is_empty());
        assert!(projection.open.is_empty());
        drop(projection);
        assert!(ctx.used_metrics.unwrap().is_empty());
    }
}
