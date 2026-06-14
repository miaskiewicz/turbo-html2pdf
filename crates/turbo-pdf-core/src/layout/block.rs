//! Block layout (§5.3, AC-5.5): turns the box tree into positioned fragments in
//! the galley's continuous top-down coordinate space. Widths resolve top-down
//! (honoring `box-sizing`, `auto` fill, and min/max clamps); heights/positions
//! accumulate as children stack, with **margin collapsing** between siblings and
//! collapse-through of empty blocks (a running-margin model). `Lines` boxes defer
//! to the inline builder; `Flex`/`Table` fall back to block flow until their own
//! modules replace the dispatch.
//!
//! Deferred in v1 (documented): parent/child margin collapse, negative margins,
//! `%`/auto explicit heights, and true inline placement of atomic inlines (an
//! `inline-block` is stacked below its line rather than placed within it).

use crate::error::Diagnostics;
use crate::node::TKind;
use crate::text::FontRegistry;

use super::boxgen::{BoxKind, InlineItem, LayoutBox};
use super::fragment::{BreakMeta, Fragment, FragmentContent, NodeId};
use super::inline::{self, InlineRun};
use super::value::{
    resolve_box_style, BoxSizing, BoxStyle, LengthPct, ResolveCtx, DEFAULT_FONT_SIZE,
};

/// Shared layout inputs threaded through the recursion. Shared with `flex.rs`.
pub(crate) struct Ctx<'a> {
    pub(crate) fonts: &'a FontRegistry,
    pub(crate) diags: &'a mut Diagnostics,
}

fn resolve(lb: &LayoutBox, cb_width: f32, parent_fs: f32) -> BoxStyle {
    resolve_box_style(
        &lb.style,
        ResolveCtx {
            parent_font_size: parent_fs,
            cb_width,
        },
    )
}

// --------------------------------------------------------------------------
// width resolution
// --------------------------------------------------------------------------

fn to_border(value: f32, sizing: BoxSizing, extra: f32) -> f32 {
    match sizing {
        BoxSizing::BorderBox => value,
        BoxSizing::ContentBox => value + extra,
    }
}

fn clamp_width(mut bb: f32, bs: &BoxStyle, cb_width: f32, extra: f32) -> f32 {
    if let Some(min) = bs.min_width.resolve(cb_width) {
        bb = bb.max(to_border(min, bs.box_sizing, extra));
    }
    if let Some(max) = bs.max_width.resolve(cb_width) {
        bb = bb.min(to_border(max, bs.box_sizing, extra));
    }
    bb
}

fn border_box_width(bs: &BoxStyle, cb_width: f32) -> f32 {
    let avail = (cb_width - bs.margin.horizontal()).max(0.0);
    let extra = bs.padding.horizontal() + bs.border.widths().horizontal();
    let bb = match bs.width.resolve(cb_width) {
        None => avail,
        Some(w) => to_border(w, bs.box_sizing, extra),
    };
    clamp_width(bb, bs, cb_width, extra)
}

fn content_box_height(bs: &BoxStyle, content_h: f32) -> f32 {
    match bs.height {
        LengthPct::Px(h) => h,
        _ => content_h,
    }
}

// --------------------------------------------------------------------------
// inline (Lines) layout
// --------------------------------------------------------------------------

fn text_run(item: &InlineItem, parent_fs: f32, cw: f32, fonts: &FontRegistry) -> Option<InlineRun> {
    let (node_id, style, text) = match item {
        InlineItem::Text {
            node_id,
            style,
            text,
        } => (node_id, style, text),
        _ => return None,
    };
    let bs = resolve_box_style(
        style,
        ResolveCtx {
            parent_font_size: parent_fs,
            cb_width: cw,
        },
    );
    let families: Vec<&str> = bs.font_families.iter().map(String::as_str).collect();
    let face = fonts.select(&families, bs.font_weight, bs.italic)?.clone();
    Some(InlineRun {
        node_id: *node_id,
        text: text.clone(),
        face,
        families: bs.font_families.clone(),
        weight: bs.font_weight,
        italic: bs.italic,
        font_size: bs.font_size,
        line_height: bs.line_height,
        letter_spacing: bs.letter_spacing,
        color: bs.color,
        valign: bs.vertical_align,
    })
}

pub(crate) fn build_runs(
    items: &[InlineItem],
    parent_fs: f32,
    cw: f32,
    fonts: &FontRegistry,
) -> Vec<InlineRun> {
    items
        .iter()
        .filter_map(|it| text_run(it, parent_fs, cw, fonts))
        .collect()
}

fn lines_to_fragments(para: &inline::ParagraphLayout, cx: f32, cy: f32, out: &mut Vec<Fragment>) {
    for line in &para.lines {
        for gr in &line.runs {
            let content = FragmentContent::TextLine {
                glyphs: gr.glyphs.clone(),
                face: gr.face.clone(),
                font_size: gr.font_size,
                color: gr.color,
            };
            out.push(Fragment::new(
                gr.node_id,
                cx,
                cy + line.top,
                line.width,
                line.height,
                content,
            ));
        }
    }
}

fn directive_frag(node_id: NodeId, kind: TKind, x: f32, y: f32) -> Fragment {
    Fragment::new(node_id, x, y, 0.0, 0.0, FragmentContent::Directive(kind))
}

/// Atomic inlines are stacked below the text lines (v1 simplification); inline
/// directives become zero-size markers at the box origin.
fn layout_atomics(
    items: &[InlineItem],
    cx: f32,
    cy: f32,
    cw: f32,
    fs: f32,
    ctx: &mut Ctx,
    start_h: f32,
) -> (Vec<Fragment>, f32) {
    let mut frags = Vec::new();
    let mut height = start_h;
    for item in items {
        match item {
            InlineItem::Atomic(b) => {
                let f = layout_box(b, cx, cy + height, cw, fs, ctx);
                height += f.height;
                frags.push(f);
            }
            InlineItem::Directive { node_id, kind } => {
                frags.push(directive_frag(*node_id, *kind, cx, cy));
            }
            InlineItem::Text { .. } => {}
        }
    }
    (frags, height)
}

fn layout_lines(
    items: &[InlineItem],
    bs: &BoxStyle,
    cx: f32,
    cy: f32,
    cw: f32,
    ctx: &mut Ctx,
) -> (Vec<Fragment>, f32) {
    let fonts = ctx.fonts;
    let runs = build_runs(items, bs.font_size, cw, fonts);
    let para = inline::layout_paragraph(&runs, fonts, cw, bs.text_align, ctx.diags);
    let mut frags = Vec::new();
    lines_to_fragments(&para, cx, cy, &mut frags);
    let (extra, height) = layout_atomics(items, cx, cy, cw, bs.font_size, ctx, para.height);
    frags.extend(extra);
    (frags, height)
}

// --------------------------------------------------------------------------
// block flow + margin collapsing
// --------------------------------------------------------------------------

fn layout_block_flow(
    kids: &[LayoutBox],
    cx: f32,
    cy: f32,
    cw: f32,
    fs: f32,
    ctx: &mut Ctx,
) -> (Vec<Fragment>, f32) {
    let mut frags = Vec::new();
    let mut cursor = cy;
    let mut pending = 0.0_f32;
    for kid in kids {
        let kbs = resolve(kid, cw, fs);
        pending = pending.max(kbs.margin.top);
        let frag = layout_box(kid, cx + kbs.margin.left, cursor + pending, cw, fs, ctx);
        if frag.height == 0.0 {
            pending = pending.max(kbs.margin.bottom);
        } else {
            cursor += pending + frag.height;
            pending = kbs.margin.bottom;
        }
        frags.push(frag);
    }
    (frags, cursor - cy)
}

fn layout_content(
    lb: &LayoutBox,
    bs: &BoxStyle,
    cx: f32,
    cy: f32,
    cw: f32,
    ctx: &mut Ctx,
) -> (Vec<Fragment>, f32) {
    match &lb.kind {
        BoxKind::Block(kids) => layout_block_flow(kids, cx, cy, cw, bs.font_size, ctx),
        BoxKind::Flex(kids) => super::flex::layout_flex(lb, kids, cx, cy, cw, bs.font_size, ctx),
        BoxKind::Table(kids) => super::table::layout_table(lb, kids, cx, cy, cw, bs.font_size, ctx),
        BoxKind::Lines(items) => layout_lines(items, bs, cx, cy, cw, ctx),
        BoxKind::Directive(_) => (Vec::new(), 0.0),
    }
}

fn break_meta_of(bs: &BoxStyle) -> BreakMeta {
    BreakMeta {
        break_before: bs.break_before,
        break_after: bs.break_after,
        break_inside_avoid: bs.break_inside_avoid,
        orphans: bs.orphans,
        widows: bs.widows,
        repeatable: None,
        footnotes: Vec::new(),
    }
}

fn content_kind(lb: &LayoutBox, bs: &BoxStyle) -> FragmentContent {
    match &lb.kind {
        BoxKind::Directive(k) => FragmentContent::Directive(*k),
        _ => FragmentContent::Box {
            background: bs.background,
            border: bs.border,
        },
    }
}

/// Lay out a box whose border-box width is already decided (`bbw`), at border-box
/// top-left `(bx, by)`. Used by `flex.rs`, which sizes items itself.
pub(crate) fn layout_box_sized(
    lb: &LayoutBox,
    bs: &BoxStyle,
    bx: f32,
    by: f32,
    bbw: f32,
    ctx: &mut Ctx,
) -> Fragment {
    let bw = bs.border.widths();
    let content_x = bx + bw.left + bs.padding.left;
    let content_y = by + bw.top + bs.padding.top;
    let content_w = (bbw - bs.padding.horizontal() - bw.horizontal()).max(0.0);
    let (children, content_h) = layout_content(lb, bs, content_x, content_y, content_w, ctx);
    let bbh = content_box_height(bs, content_h) + bs.padding.vertical() + bw.vertical();
    Fragment {
        node_id: lb.node_id,
        x: bx,
        y: by,
        width: bbw,
        height: bbh,
        content: content_kind(lb, bs),
        break_meta: break_meta_of(bs),
        children,
    }
}

/// Lay out one block-level box with its border-box top-left at `(bx, by)`,
/// resolving its width against the containing block.
fn layout_box(
    lb: &LayoutBox,
    bx: f32,
    by: f32,
    cb_width: f32,
    parent_fs: f32,
    ctx: &mut Ctx,
) -> Fragment {
    let bs = resolve(lb, cb_width, parent_fs);
    let bbw = border_box_width(&bs, cb_width);
    layout_box_sized(lb, &bs, bx, by, bbw, ctx)
}

/// Lay out a box tree (root from `boxgen::build_box_tree`) into a galley fragment
/// rooted at `(0, 0)` with the given containing-block width.
pub fn layout_tree(
    root: &LayoutBox,
    cb_width: f32,
    fonts: &FontRegistry,
    diags: &mut Diagnostics,
) -> Fragment {
    let mut ctx = Ctx { fonts, diags };
    layout_box(root, 0.0, 0.0, cb_width, DEFAULT_FONT_SIZE, &mut ctx)
}
