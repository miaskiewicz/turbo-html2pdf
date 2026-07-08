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
use crate::image::probe;
use crate::node::TKind;
use crate::text::{Align, FontRegistry};

use super::boxgen::{BoxKind, ImageSource, InlineItem, LayoutBox};
use super::fragment::{BreakMeta, Fragment, FragmentContent, ImagePlacement, NodeId};
use super::imgsize::{size_replaced, SizeCtx, SizedImage};
use super::inline::{self, InlineRun};
use super::value::{
    resolve_box_style, BoxSizing, BoxStyle, Float, LengthPct, Position, ResolveCtx,
    DEFAULT_FONT_SIZE,
};
use super::ImageCtx;

/// Shared layout inputs threaded through the recursion. Shared with `flex.rs`.
pub(crate) struct Ctx<'a> {
    pub(crate) fonts: &'a FontRegistry,
    pub(crate) images: &'a ImageCtx<'a>,
    pub(crate) diags: &'a mut Diagnostics,
    /// Containing block for `position:absolute` descendants — the content-box
    /// origin (`x`,`y`) + width of the nearest positioned ancestor (the page at
    /// the root). `position:fixed` uses the page origin (`0,0`,[`Ctx::root_w`]).
    pub(crate) abs_cb_x: f32,
    pub(crate) abs_cb_y: f32,
    pub(crate) abs_cb_w: f32,
    /// The initial containing block width (page content width) — the CB for
    /// `position:fixed` boxes.
    pub(crate) root_w: f32,
    /// Floats active in the current **block formatting context**, in absolute
    /// galley coordinates. In-flow content within the same BFC flows in the
    /// inline region these leave free (a `float:right` infobox narrows the text
    /// column to its left), across block *and* sibling-section boundaries — a
    /// float belongs to its BFC, not to the nearest block. Saved/cleared when a
    /// box establishes a new BFC (`float`, out-of-flow, `overflow`≠visible,
    /// flex/grid/table), and that box grows to contain the floats placed inside.
    pub(crate) floats: Vec<FloatRect>,
}

/// A placed float's margin box in absolute galley coordinates, plus which edge
/// it packs against. Consulted to narrow the inline region of in-flow content
/// whose vertical span overlaps `[top, bottom)`.
#[derive(Clone, Copy)]
pub(crate) struct FloatRect {
    x0: f32,
    x1: f32,
    top: f32,
    bottom: f32,
    side: Float,
}

/// The containing-block origin (x, y) + width a positioned box resolves its
/// insets against: the page for `fixed`, else the nearest positioned ancestor.
fn containing_block(bs: &BoxStyle, ctx: &Ctx) -> (f32, f32, f32) {
    if bs.position == Position::Fixed {
        (0.0, 0.0, ctx.root_w)
    } else {
        (ctx.abs_cb_x, ctx.abs_cb_y, ctx.abs_cb_w)
    }
}

/// The border-box top-left of an out-of-flow box. `static_pos` is where the box
/// would sit in normal flow (its content-box origin there): per CSS an `auto`
/// inset resolves to that **static position**, NOT the containing-block origin —
/// so a `position:absolute` decoration with no `top`/`left` stays where it is in
/// the document (e.g. a navbox's rotated label at the bottom) instead of jumping
/// to the top-left of the CB and piling up with every other no-offset absolute.
fn out_of_flow_origin(
    bs: &BoxStyle,
    cb_width: f32,
    ctx: &Ctx,
    static_pos: (f32, f32),
) -> (f32, f32) {
    let (cbx, cby, cbw) = containing_block(bs, ctx);
    let bbw = border_box_width(bs, cbw.max(cb_width));
    let x = match (bs.inset_left.resolve(cbw), bs.inset_right.resolve(cbw)) {
        (Some(l), _) => cbx + l,
        (None, Some(r)) => cbx + cbw - bbw - r,
        (None, None) => static_pos.0,
    };
    let y = match (bs.inset_top.resolve(cbw), bs.inset_bottom.resolve(cbw)) {
        (Some(t), _) => cby + t,
        // The CB height is unknown mid-layout; anchor `bottom`-only boxes from the
        // static position too (a documented approximation).
        (None, Some(_)) | (None, None) => static_pos.1,
    };
    (x + bs.margin.left, y + bs.margin.top)
}

/// The `(dx, dy)` a `position:relative` box is visually shifted by (it still
/// occupies its normal-flow space). `left`/`top` win over `right`/`bottom`.
fn relative_offset(bs: &BoxStyle, cb_width: f32) -> (f32, f32) {
    let dx = bs
        .inset_left
        .resolve(cb_width)
        .or_else(|| bs.inset_right.resolve(cb_width).map(|r| -r))
        .unwrap_or(0.0);
    let dy = bs
        .inset_top
        .resolve(cb_width)
        .or_else(|| bs.inset_bottom.resolve(cb_width).map(|b| -b))
        .unwrap_or(0.0);
    (dx, dy)
}

fn resolve(lb: &LayoutBox, cb_width: f32, parent_fs: f32) -> BoxStyle {
    lb.resolved(ResolveCtx {
        parent_font_size: parent_fs,
        cb_width,
    })
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
    let mut h = match bs.height {
        LengthPct::Px(h) => h,
        _ => content_h,
    };
    // Honor `min-height`/`max-height` (px; `%` needs the CB height we don't track).
    // Without this an empty box with only `min-height` collapsed to 0 — e.g. the
    // Codex radio's `.cdx-radio__icon` (an empty span sized by min/height) vanished.
    if let LengthPct::Px(min) = bs.min_height {
        h = h.max(min);
    }
    if let LengthPct::Px(max) = bs.max_height {
        h = h.min(max);
    }
    h
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
        nowrap: matches!(
            style.get("white-space").map(str::trim),
            Some("nowrap") | Some("pre")
        ),
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
                cx + line.indent,
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

/// A directive fragment carrying a `<t:anchor name>`'s destination so emit can
/// register it as a named GoTo target (`xref` feature, AC-3.25).
#[cfg(feature = "xref")]
fn anchor_directive_frag(
    node_id: NodeId,
    kind: TKind,
    x: f32,
    y: f32,
    anchor: &Option<String>,
) -> Fragment {
    let mut frag = directive_frag(node_id, kind, x, y);
    frag.xref.anchor = anchor.clone();
    frag
}

/// Lay out an atomic inline (`inline-block` / replaced `<img>`) at the origin so
/// its size is known; the caller translates it to its placed line position. An
/// auto-width, non-replaced `inline-block` shrinks to its content (else it would
/// fill the line); replaced/explicit-width boxes size themselves normally.
fn lay_atomic(b: &LayoutBox, cw: f32, fs: f32, ctx: &mut Ctx) -> Fragment {
    let bs = resolve(b, cw, fs);
    let replaced = b.image.as_ref().is_some_and(|s| s.replaced);
    if !replaced && bs.width.resolve(cw).is_none() {
        let w = super::flex::natural_width(b, ctx.fonts).min(cw);
        layout_box_sized(b, &bs, 0.0, 0.0, w, ctx)
    } else {
        layout_box(b, 0.0, 0.0, cw, fs, ctx)
    }
}

/// Lay out an inline formatting context: text runs and atomic inline boxes flow
/// together into line boxes (atoms are unbreakable units placed on the baseline),
/// so an `inline-block`/`<img>` sits *within* the line next to text rather than
/// stacking below it. Inline directives become zero-size markers at the origin.
fn layout_lines(
    items: &[InlineItem],
    bs: &BoxStyle,
    cx: f32,
    cy: f32,
    cw: f32,
    ctx: &mut Ctx,
) -> (Vec<Fragment>, f32) {
    // Pre-lay each atomic (recursively) to learn its size, and build the inline
    // piece sequence in document order; directives are collected as markers.
    let mut atom_frags: Vec<Fragment> = Vec::new();
    let mut directives: Vec<Fragment> = Vec::new();
    let mut pieces: Vec<inline::Piece> = Vec::new();
    for item in items {
        match item {
            InlineItem::Text { .. } => {
                if let Some(run) = text_run(item, bs.font_size, cw, ctx.fonts) {
                    pieces.push(inline::Piece::Run(run));
                }
            }
            InlineItem::Atomic(b) => {
                let f = lay_atomic(b, cw, bs.font_size, ctx);
                pieces.push(inline::Piece::Atom(inline::InlineAtom {
                    id: atom_frags.len(),
                    width: f.width,
                    height: f.height,
                }));
                atom_frags.push(f);
            }
            #[cfg(not(feature = "xref"))]
            InlineItem::Directive { node_id, kind } => {
                directives.push(directive_frag(*node_id, *kind, cx, cy));
            }
            #[cfg(feature = "xref")]
            InlineItem::Directive {
                node_id,
                kind,
                anchor,
            } => {
                directives.push(anchor_directive_frag(*node_id, *kind, cx, cy, anchor));
            }
        }
    }
    let fonts = ctx.fonts;
    // Per-line float wrapping: each line's `(indent, width)` is the free inline
    // region the BFC's floats leave at that line's own y (absolute `cy + top`), so
    // text flows beside a `float:right` infobox and widens once below its bottom.
    // Clone the float set so the closure can borrow it while `ctx.diags` is used.
    let floats = ctx.floats.clone();
    let region = |top: f32| {
        let (rx, rw) = inline_region_from(&floats, cx, cw, cy + top);
        (rx - cx, rw)
    };
    let para = inline::layout_paragraph_in(&pieces, fonts, bs.text_align, ctx.diags, &region);
    let mut frags = Vec::new();
    lines_to_fragments(&para, cx, cy, &mut frags);
    // Translate each pre-laid atom to where it landed within its line.
    for line in &para.lines {
        for placed in &line.atoms {
            let mut f = atom_frags[placed.id].clone();
            f.translate(cx + line.indent + placed.x, cy + line.top + placed.y);
            frags.push(f);
        }
    }
    frags.extend(directives);
    (frags, para.height)
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
    align: Align,
    ctx: &mut Ctx,
) -> (Vec<Fragment>, f32) {
    let mut frags = Vec::new();
    let mut cursor = cy;
    let mut pending = 0.0_f32;
    for kid in kids {
        let kbs = resolve(kid, cw, fs);
        // `absolute`/`fixed`: taken out of flow — placed at its insets against the
        // containing block, or (for `auto` insets) at its static in-flow position.
        // Contributes nothing to the cursor or margin run.
        if kbs.position.is_out_of_flow() {
            let static_pos = (cx + kbs.margin.left, cursor + pending.max(kbs.margin.top));
            let (bx, by) = out_of_flow_origin(&kbs, cw, ctx, static_pos);
            frags.push(layout_box(kid, bx, by, cw, fs, ctx));
            continue;
        }
        // `float:left/right`: packed to its edge and registered in the BFC's float
        // set (so following in-flow content — even in later sibling blocks/sections
        // — wraps beside it). Contributes no cursor advance.
        if kbs.float != Float::None {
            frags.push(place_float(kid, &kbs, cx, cw, fs, cursor + pending, ctx));
            continue;
        }
        // `clear`: skip past the floats on the cleared side(s) before laying out.
        let cleared = clear_below(ctx, kid, cursor + pending);
        if cleared > cursor + pending {
            cursor = cleared;
            pending = 0.0;
        }
        // `relative`: flows normally (its space is reserved via the cursor) but
        // is painted shifted by its insets, so lay it out at the shifted origin.
        let (dx, dy) = if matches!(kbs.position, Position::Relative | Position::Sticky) {
            relative_offset(&kbs, cw)
        } else {
            (0.0, 0.0)
        };
        pending = pending.max(kbs.margin.top);
        let flow_y = cursor + pending;
        // A normal-flow block box spans the full content width; a float only shortens
        // the *line boxes* of its inline content (handled per-line in `layout_lines`),
        // not the block itself. Only a box that avoids floats as a unit — one that
        // establishes a BFC (table/flex/grid/`overflow`), or a replaced `<img>` — is
        // narrowed and shifted into the free region beside the float.
        let avoids_floats =
            establishes_bfc(kid, &kbs) || kid.image.as_ref().is_some_and(|s| s.replaced);
        let (region_x, region_w) = if avoids_floats {
            inline_region(ctx, cx, cw, flow_y)
        } else {
            (cx, cw)
        };
        let mut frag = layout_box(
            kid,
            region_x + kbs.margin.left + dx,
            flow_y + dy,
            region_w,
            fs,
            ctx,
        );
        // Horizontally align a width-constrained block within its inline region:
        // `margin: … auto` centers it, and a `text-align:center`/`right` container
        // centers/right-aligns block children (legacy `<center>` / `align=center`).
        let hx = block_h_offset(align, &kbs, kid, frag.width, region_w);
        if hx != 0.0 {
            frag.translate(hx, 0.0);
        }
        // Advance the cursor by the box's height at its *unshifted* flow position.
        if frag.height == 0.0 {
            pending = pending.max(kbs.margin.bottom);
        } else {
            cursor += pending + frag.height;
            pending = kbs.margin.bottom;
        }
        frags.push(frag);
    }
    // Height is the in-flow content only. Floats are contained by their BFC (see
    // `layout_box_sized`), not by every block they pass through — a non-BFC block
    // does not grow to enclose a float, so following content wraps beside it.
    (frags, cursor - cy)
}

/// The horizontal shift to align a width-constrained in-flow block within its
/// container (0 for a full-width/auto block, which fills the line). `margin:auto`
/// centers; otherwise the container's `text-align` centers/right-aligns block
/// children — the legacy behavior `<center>` and `align="center"` rely on.
fn block_h_offset(align: Align, kbs: &BoxStyle, kid: &LayoutBox, frag_w: f32, cw: f32) -> f32 {
    let avail = (cw - kbs.margin.horizontal()).max(0.0);
    if frag_w >= avail - 0.5 {
        return 0.0;
    }
    if auto_x_margins(&kid.style) || align == Align::Center {
        return (cw - frag_w) / 2.0 - kbs.margin.left;
    }
    if align == Align::Right {
        return cw - frag_w - kbs.margin.right - kbs.margin.left;
    }
    0.0
}

/// Whether a box centers itself via `margin: … auto` (horizontal auto margins).
fn auto_x_margins(s: &crate::style::ComputedStyle) -> bool {
    let is_auto = |p| s.get(p).map(str::trim) == Some("auto");
    is_auto("margin-left") && is_auto("margin-right")
        || s.get("margin")
            .is_some_and(|m| m.split_whitespace().any(|t| t == "auto"))
}

/// Whether a box establishes a new block formatting context — floats it contains
/// are isolated from the outer context (they don't wrap outer floats, and they
/// grow this box). Floats, out-of-flow boxes, non-`visible` `overflow`, and the
/// independent formatting roots (flex/grid/table) all qualify.
fn establishes_bfc(lb: &LayoutBox, bs: &BoxStyle) -> bool {
    if bs.float != Float::None || bs.position.is_out_of_flow() {
        return true;
    }
    if matches!(
        lb.kind,
        BoxKind::Flex(_) | BoxKind::Grid(_) | BoxKind::Table(_)
    ) {
        return true;
    }
    ["overflow", "overflow-x", "overflow-y"].iter().any(|p| {
        matches!(
            lb.style.get(p).map(str::trim),
            Some("hidden" | "auto" | "scroll" | "clip")
        )
    })
}

/// The free inline region `(x, width)` at vertical position `y` within the
/// content box `[cx, cx+cw]`, narrowed by every float in the current BFC whose
/// vertical span contains `y` (left floats push the start right, right floats
/// pull the end left). Width floors at 1px so a fully-covered row still lays out.
fn inline_region(ctx: &Ctx, cx: f32, cw: f32, y: f32) -> (f32, f32) {
    inline_region_from(&ctx.floats, cx, cw, y)
}

fn inline_region_from(floats: &[FloatRect], cx: f32, cw: f32, y: f32) -> (f32, f32) {
    let mut left = cx;
    let mut right = cx + cw;
    for f in floats {
        if y + 0.5 < f.top || y > f.bottom - 0.5 {
            continue;
        }
        match f.side {
            Float::Left => left = left.max(f.x1),
            Float::Right => right = right.min(f.x0),
            Float::None => {}
        }
    }
    (left, (right - left).max(1.0))
}

/// The y a box with `clear` must drop to: the lowest bottom edge of any active
/// float on the cleared side(s) (`clear:left/right/both`), or `y` if none.
fn clear_below(ctx: &Ctx, kid: &LayoutBox, y: f32) -> f32 {
    let (cl, cr) = match kid.style.get("clear").map(str::trim) {
        Some("both") => (true, true),
        Some("left") => (true, false),
        Some("right") => (false, true),
        _ => return y,
    };
    let mut ny = y;
    for f in &ctx.floats {
        let hit = matches!(f.side, Float::Left) && cl || matches!(f.side, Float::Right) && cr;
        if hit && f.bottom > ny {
            ny = f.bottom;
        }
    }
    ny
}

/// Place a floated box: size it (shrink-to-fit for auto width), drop it to the
/// first y at or below `y0` where it fits on its side beside earlier floats
/// (else below them), pack it to that edge, and register it in the BFC's float
/// set so following in-flow content wraps beside it.
fn place_float(
    kid: &LayoutBox,
    kbs: &BoxStyle,
    cx: f32,
    cw: f32,
    fs: f32,
    y0: f32,
    ctx: &mut Ctx,
) -> Fragment {
    let replaced = kid.image.as_ref().is_some_and(|s| s.replaced);
    let shrink = !replaced && kbs.width.resolve(cw).is_none();
    let w = if shrink {
        super::flex::natural_width(kid, ctx.fonts).min(cw)
    } else {
        border_box_width(kbs, cw)
    };
    // Find the y at/below y0 where a `w`-wide float fits beside existing same-BFC
    // floats; if the row is too full, drop past the nearest float bottom and retry.
    let mut y = y0;
    loop {
        let (_, avail) = inline_region(ctx, cx, cw, y);
        if avail >= w - 0.5 {
            break;
        }
        let next = ctx
            .floats
            .iter()
            .filter(|f| f.bottom > y + 0.5)
            .map(|f| f.bottom)
            .fold(f32::INFINITY, f32::min);
        if !next.is_finite() {
            break;
        }
        y = next;
    }
    let (rx, rw) = inline_region(ctx, cx, cw, y);
    let bx = match kbs.float {
        Float::Right => rx + rw - w,
        _ => rx,
    };
    let mut f = if shrink {
        layout_box_sized(kid, kbs, bx, y, w, ctx)
    } else {
        layout_box(kid, bx, y, cw, fs, ctx)
    };
    // Re-anchor a right float to the true right edge if its laid width differs.
    if kbs.float == Float::Right && (f.width - w).abs() > 0.5 {
        f.translate(rx + rw - f.width - f.x, 0.0);
    }
    ctx.floats.push(FloatRect {
        x0: f.x,
        x1: f.x + f.width,
        top: y,
        bottom: y + f.height,
        side: kbs.float,
    });
    f
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
        BoxKind::Block(kids) => {
            layout_block_flow(kids, cx, cy, cw, bs.font_size, bs.text_align, ctx)
        }
        BoxKind::Flex(kids) => super::flex::layout_flex(lb, kids, cx, cy, cw, bs.font_size, ctx),
        BoxKind::Grid(kids) => super::flex::layout_grid(lb, kids, cx, cy, cw, bs.font_size, ctx),
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

fn content_kind(lb: &LayoutBox, bs: &BoxStyle, bbw: f32, bbh: f32) -> FragmentContent {
    match &lb.kind {
        BoxKind::Directive(k) => FragmentContent::Directive(*k),
        _ => FragmentContent::Box {
            // A masked box paints its fill THROUGH the mask (a separate tinted image
            // fragment), so it must not also paint a solid rectangle behind it.
            background: bs.background.filter(|_| lb.mask.is_none()),
            border: bs.border,
            border_radius: resolve_radius(bs.border_radius, bbw, bbh),
        },
    }
}

/// The colour a `mask-image` box paints through the mask: its `background-color`,
/// falling back to `color` (`currentColor`, the common idiom) when the background
/// is transparent.
fn mask_tint(bs: &BoxStyle) -> super::value::Rgba {
    bs.background.unwrap_or(bs.color)
}

/// If the box carries a resolvable `mask-image`, insert a tinted `Image` fragment
/// filling its content box (in front of the background, behind its children) that
/// the raster paints as `mask_tint` stencilled by the mask's alpha.
fn prepend_mask_image(
    lb: &LayoutBox,
    bs: &BoxStyle,
    rect: (f32, f32, f32, f32),
    ctx: &Ctx,
    children: &mut Vec<Fragment>,
) {
    let Some(name) = lb.mask.as_ref() else {
        return;
    };
    let src = ImageSource {
        name: name.clone(),
        replaced: false,
    };
    let Some(intrinsic) = probe_source(&src, ctx) else {
        return;
    };
    let (cx, cy, cw, ch) = rect;
    let mut placement = super::imgsize::placement_of(name.clone(), intrinsic);
    placement.tint = Some(mask_tint(bs));
    let frag = Fragment::new(
        lb.node_id,
        cx,
        cy,
        cw,
        ch,
        FragmentContent::Image(placement),
    );
    children.insert(0, frag);
}

/// Resolve `border-radius` to px against the border box, clamped to half the
/// shorter side (so `50%` on a square is a circle, and no corner over-rounds).
fn resolve_radius(r: LengthPct, bbw: f32, bbh: f32) -> f32 {
    let px = match r {
        LengthPct::Px(v) => v,
        LengthPct::Pct(p) => p / 100.0 * bbw.min(bbh),
        LengthPct::Auto => 0.0,
    };
    px.clamp(0.0, (bbw.min(bbh) / 2.0).max(0.0))
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
    layout_box_sized_impl(lb, bs, bx, by, bbw, ctx, false)
}

/// [`layout_box_sized`] for a box that ALWAYS establishes a block formatting
/// context — a flex/grid item or a table cell (§CSS: each is an independent
/// formatting context). Without this, an item that floats its content (a clearfix
/// bar like Wikipedia's page-title row) doesn't contain the float, so its
/// min-content height measures 0 and the grid row collapses under the overflow.
pub(crate) fn layout_box_sized_isolated(
    lb: &LayoutBox,
    bs: &BoxStyle,
    bx: f32,
    by: f32,
    bbw: f32,
    ctx: &mut Ctx,
) -> Fragment {
    layout_box_sized_impl(lb, bs, bx, by, bbw, ctx, true)
}

fn layout_box_sized_impl(
    lb: &LayoutBox,
    bs: &BoxStyle,
    bx: f32,
    by: f32,
    bbw: f32,
    ctx: &mut Ctx,
    force_bfc: bool,
) -> Fragment {
    let bw = bs.border.widths();
    let content_x = bx + bw.left + bs.padding.left;
    let content_y = by + bw.top + bs.padding.top;
    let content_w = (bbw - bs.padding.horizontal() - bw.horizontal()).max(0.0);
    // A positioned box establishes the containing block for its `absolute`
    // descendants (its content box). Save/restore around the child layout.
    let saved_cb = (ctx.abs_cb_x, ctx.abs_cb_y, ctx.abs_cb_w);
    if bs.position != Position::Static {
        ctx.abs_cb_x = content_x;
        ctx.abs_cb_y = content_y;
        ctx.abs_cb_w = content_w;
    }
    // A box that establishes a block formatting context isolates floats: its
    // content does not wrap around outer floats, and the floats placed inside it
    // are contained by (grow) this box rather than escaping to sibling blocks.
    let bfc = force_bfc || establishes_bfc(lb, bs);
    let saved_floats = bfc.then(|| std::mem::take(&mut ctx.floats));
    let (mut children, mut content_h) =
        layout_content(lb, bs, content_x, content_y, content_w, ctx);
    if let Some(outer) = saved_floats {
        let inner_bottom = ctx
            .floats
            .iter()
            .map(|f| f.bottom)
            .fold(content_y, f32::max);
        content_h = content_h.max(inner_bottom - content_y);
        ctx.floats = outer;
    }
    (ctx.abs_cb_x, ctx.abs_cb_y, ctx.abs_cb_w) = saved_cb;
    // The content-box height honoring an explicit/min/max `height` — a `background`
    // or `mask` image fills the box even when it has no flow content (an empty icon
    // span sized only by `height`, whose measured `content_h` is 0).
    let fill_h = content_box_height(bs, content_h);
    let bbh = fill_h + bs.padding.vertical() + bw.vertical();
    prepend_mask_image(
        lb,
        bs,
        (content_x, content_y, content_w, fill_h),
        &*ctx,
        &mut children,
    );
    prepend_background_image(
        lb,
        content_x,
        content_y,
        content_w,
        fill_h,
        &*ctx,
        &mut children,
    );
    Fragment {
        node_id: lb.node_id,
        x: bx,
        y: by,
        width: bbw,
        height: bbh,
        content: content_kind(lb, bs, bbw, bbh),
        break_meta: break_meta_of(bs),
        children,
        z_index: bs.z_index,
        is_positioned: bs.position != Position::Static,
        #[cfg(feature = "xref")]
        xref: box_xref(lb),
        #[cfg(feature = "pdf-ua")]
        role: lb.ua_role,
        #[cfg(feature = "pdf-ua")]
        alt: None,
    }
}

/// The cross-reference payload for a box fragment: the `#name` target of an
/// `<a href="#name">` whose box this is, drawn from the box's HTML attributes
/// (`xref` feature, AC-3.25).
#[cfg(feature = "xref")]
fn box_xref(lb: &LayoutBox) -> super::fragment::XrefMeta {
    let link_href = lb
        .attrs
        .iter()
        .find(|a| a.name == "href")
        .and_then(|a| a.value.strip_prefix('#'))
        .filter(|t| !t.is_empty())
        .map(str::to_string);
    super::fragment::XrefMeta {
        anchor: None,
        link_href,
    }
}

/// If the box carries a resolvable `background-image`, insert an `Image`
/// fragment filling its content box *behind* the box's children (index 0, so it
/// paints first). A `background-image` does not resize the box.
fn prepend_background_image(
    lb: &LayoutBox,
    cx: f32,
    cy: f32,
    cw: f32,
    ch: f32,
    ctx: &Ctx,
    children: &mut Vec<Fragment>,
) {
    let Some(placement) = background_placement(lb, ctx) else {
        return;
    };
    let frag = Fragment::new(
        lb.node_id,
        cx,
        cy,
        cw,
        ch,
        FragmentContent::Image(placement),
    );
    children.insert(0, frag);
}

/// The placement for a box's `background-image`, if it has one the resolver can
/// supply and decode (probe succeeds).
fn background_placement(lb: &LayoutBox, ctx: &Ctx) -> Option<ImagePlacement> {
    let src = lb.image.as_ref().filter(|s| !s.replaced)?;
    let intrinsic = probe_source(src, ctx)?;
    Some(super::imgsize::placement_of(src.name.clone(), intrinsic))
}

/// Lay out one block-level box with its border-box top-left at `(bx, by)`,
/// resolving its width against the containing block. A replaced `<img>` is sized
/// from its intrinsic dimensions and the overflow caps instead of flowing.
fn layout_box(
    lb: &LayoutBox,
    bx: f32,
    by: f32,
    cb_width: f32,
    parent_fs: f32,
    ctx: &mut Ctx,
) -> Fragment {
    let bs = resolve(lb, cb_width, parent_fs);
    if let Some(frag) = replaced_image_box(lb, &bs, bx, by, cb_width, ctx) {
        return frag;
    }
    let mut bbw = border_box_width(&bs, cb_width);
    // A table never shrinks below its content: grow the border box to the columns'
    // min-content when a declared/`max-width` is narrower (CSS auto table layout).
    // Wikipedia's infobox is `width:200px` but its cat-image row is 267px wide — so
    // the box must be 267, not clip the images and squeeze the label column.
    if let BoxKind::Table(items) = &lb.kind {
        let frame = bs.padding.horizontal() + bs.border.widths().horizontal();
        bbw = bbw.max(super::table::min_content_width(items, &lb.style, ctx.fonts) + frame);
    }
    layout_box_sized(lb, &bs, bx, by, bbw, ctx)
}

/// Build the fragment for a replaced `<img>` box, or `None` when the box is not
/// a (resolvable) replaced image. The box size is the capped image size; the
/// image is the fragment's own content (no children flow inside an `<img>`).
fn replaced_image_box(
    lb: &LayoutBox,
    bs: &BoxStyle,
    bx: f32,
    by: f32,
    cb_width: f32,
    ctx: &Ctx,
) -> Option<Fragment> {
    let src = lb.image.as_ref().filter(|s| s.replaced)?;
    let intrinsic = probe_source(src, ctx)?;
    let sized = size_replaced(src.name.clone(), intrinsic, &size_ctx(bs, cb_width, ctx));
    Some(image_fragment(lb, bx, by, &sized, bs))
}

/// Assemble the [`SizeCtx`] for the image sizer from a box's style and the
/// layout image context (containing-block width + optional page body height).
fn size_ctx<'a>(bs: &'a BoxStyle, cb_width: f32, ctx: &Ctx) -> SizeCtx<'a> {
    SizeCtx {
        style: bs,
        cb_width,
        body_height: ctx.images.body_height,
    }
}

/// Build a positioned `Image` fragment from a sized image, carrying the box's
/// break metadata so a tall image still obeys break hints.
fn image_fragment(lb: &LayoutBox, bx: f32, by: f32, sized: &SizedImage, bs: &BoxStyle) -> Fragment {
    Fragment {
        node_id: lb.node_id,
        x: bx,
        y: by,
        width: sized.width,
        height: sized.height,
        content: FragmentContent::Image(sized.placement.clone()),
        break_meta: break_meta_of(bs),
        children: Vec::new(),
        z_index: bs.z_index,
        is_positioned: bs.position != Position::Static,
        #[cfg(feature = "xref")]
        xref: super::fragment::XrefMeta::default(),
        #[cfg(feature = "pdf-ua")]
        role: lb.ua_role,
        #[cfg(feature = "pdf-ua")]
        alt: lb.ua_alt.clone(),
    }
}

/// Probe an image source for its intrinsic size + alpha flag via the resolver,
/// or `None` if it does not resolve or decode.
fn probe_source(src: &ImageSource, ctx: &Ctx) -> Option<crate::image::Intrinsic> {
    let bytes = ctx.images.resolver.resolve(&src.name)?;
    probe(bytes)
}

/// Lay out a box tree (root from `boxgen::build_box_tree`) into a galley fragment
/// rooted at `(0, 0)` with the given containing-block width. Embeds no images;
/// use [`layout_tree_with_images`] to size `<img>`/`background-image` boxes.
pub fn layout_tree(
    root: &LayoutBox,
    cb_width: f32,
    fonts: &FontRegistry,
    diags: &mut Diagnostics,
) -> Fragment {
    layout_tree_with_images(root, cb_width, fonts, &ImageCtx::none(), diags)
}

/// Lay out a box tree, sizing raster images against `images` (its resolver +
/// page-height basis). See [`layout_tree`] for the image-free entry.
pub fn layout_tree_with_images(
    root: &LayoutBox,
    cb_width: f32,
    fonts: &FontRegistry,
    images: &ImageCtx,
    diags: &mut Diagnostics,
) -> Fragment {
    let mut ctx = Ctx {
        fonts,
        images,
        diags,
        // The initial containing block is the page: origin (0,0), page width.
        abs_cb_x: 0.0,
        abs_cb_y: 0.0,
        abs_cb_w: cb_width,
        root_w: cb_width,
        floats: Vec::new(),
    };
    let mut frag = layout_box(root, 0.0, 0.0, cb_width, DEFAULT_FONT_SIZE, &mut ctx);
    // The galley (initial containing block) contains every float — grow it to the
    // lowest float edge if a float outran the in-flow content (a tall right-floated
    // infobox on a short page). Floats inside a nested BFC are already contained.
    let float_bottom = ctx.floats.iter().map(|f| f.bottom).fold(0.0, f32::max);
    if float_bottom > frag.height {
        frag.height = float_bottom;
    }
    frag
}
