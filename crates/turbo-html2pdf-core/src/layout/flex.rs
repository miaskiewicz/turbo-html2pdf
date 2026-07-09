//! Flex layout (Â§5.3, AC-5.6). `taffy` owns the flexbox math (direction, wrap,
//! grow/shrink/basis, justify/align, gap); we map CSS to `taffy::Style`, feed it
//! each item's measured size, read back rects, then re-lay each item's content at
//! its assigned width (Â§5.3 decision: taffy owns flex, the engine owns the rest).
//!
//! Content sizing: an item's main size comes from its `flex-basis`/`width` when
//! set, otherwise from a max-content measurement of its content (`natural_width`);
//! the cross size comes from laying the content out at the proposed width. The
//! item's padding/border are folded into the measured border-box, and its margins
//! are handed to taffy. Per-item `align-self`/`order` are deferred (documented).

use std::collections::HashMap;

use taffy::prelude::{FromLength, FromPercent, TaffyAuto};
use taffy::style_helpers::{fr, line, minmax, percent};
use taffy::{
    AlignItems, AvailableSpace, Dimension, Display, FlexDirection, FlexWrap, GridPlacement,
    JustifyContent, Layout, LengthPercentage, LengthPercentageAuto, Line, MaxTrackSizingFunction,
    MinTrackSizingFunction, NodeId as TaffyId, Position, Rect, Size, Style, TaffyTree,
    TrackSizingFunction,
};

use crate::error::Diagnostics;
use crate::style::ComputedStyle;
use crate::text::{Align, FontRegistry};

use super::block::{self, Ctx};
use super::boxgen::{BoxKind, InlineItem, LayoutBox};
use super::fragment::Fragment;
use super::inline;
use super::value::{parse_px, BoxStyle, LengthPct, ResolveCtx, DEFAULT_FONT_SIZE};
use super::ImageCtx;

// --------------------------------------------------------------------------
// CSS -> taffy style mapping
// --------------------------------------------------------------------------

fn flex_direction(s: &ComputedStyle) -> FlexDirection {
    match s.get("flex-direction").unwrap_or("row").trim() {
        "row-reverse" => FlexDirection::RowReverse,
        "column" => FlexDirection::Column,
        "column-reverse" => FlexDirection::ColumnReverse,
        _ => FlexDirection::Row,
    }
}

fn flex_wrap(s: &ComputedStyle) -> FlexWrap {
    match s.get("flex-wrap").unwrap_or("nowrap").trim() {
        "wrap" => FlexWrap::Wrap,
        "wrap-reverse" => FlexWrap::WrapReverse,
        _ => FlexWrap::NoWrap,
    }
}

fn justify_content(s: &ComputedStyle) -> Option<JustifyContent> {
    Some(
        match s.get("justify-content").unwrap_or("flex-start").trim() {
            "flex-end" | "end" => JustifyContent::FlexEnd,
            "center" => JustifyContent::Center,
            "space-between" => JustifyContent::SpaceBetween,
            "space-around" => JustifyContent::SpaceAround,
            "space-evenly" => JustifyContent::SpaceEvenly,
            _ => JustifyContent::FlexStart,
        },
    )
}

fn align_items(s: &ComputedStyle) -> Option<AlignItems> {
    Some(match s.get("align-items").unwrap_or("stretch").trim() {
        "flex-start" | "start" => AlignItems::FlexStart,
        "flex-end" | "end" => AlignItems::FlexEnd,
        "center" => AlignItems::Center,
        "baseline" => AlignItems::Baseline,
        _ => AlignItems::Stretch,
    })
}

fn gap_len(s: &ComputedStyle) -> LengthPercentage {
    let px = s
        .get("gap")
        .and_then(|v| parse_px(v, DEFAULT_FONT_SIZE))
        .unwrap_or(0.0);
    LengthPercentage::length(px)
}

fn container_style(container: &LayoutBox, cw: f32, fs: f32) -> Style {
    let s = &container.style;
    let bs = container.resolved(ResolveCtx {
        parent_font_size: fs,
        cb_width: cw,
    });
    Style {
        display: Display::Flex,
        flex_direction: flex_direction(s),
        flex_wrap: flex_wrap(s),
        justify_content: justify_content(s),
        align_items: align_items(s),
        gap: Size {
            width: gap_len(s),
            height: gap_len(s),
        },
        // Carry the container's own `height` and `min/max-height` to taffy: a flex
        // box with only a `min-height` (a search bar's `min-height:50px`, a hero
        // banner) must not collapse to its content height. Width stays the given
        // content width.
        size: Size {
            width: Dimension::length(cw),
            height: dim(bs.height),
        },
        min_size: Size {
            width: dim(bs.min_width),
            height: dim(bs.min_height),
        },
        max_size: Size {
            width: dim(bs.max_width),
            height: dim(bs.max_height),
        },
        ..Default::default()
    }
}

fn num(s: &ComputedStyle, prop: &str, default: f32) -> f32 {
    s.get(prop)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// A flex item's main-size basis. `flex-basis:auto` (the initial value) defers to
/// the item's `width` â without that fallback a `width:100%` flex item (e.g.
/// Wikipedia's `.mw-header`, a `width:100%` grid that is itself a flex child)
/// shrink-wrapped to its content instead of filling the row.
fn item_basis(s: &ComputedStyle, bs: &BoxStyle, fs: f32) -> Dimension {
    match s.get("flex-basis").map(str::trim) {
        None | Some("auto") => dim(bs.width),
        Some("content") => Dimension::auto(),
        Some(b) => {
            if let Some(px) = parse_px(b, fs) {
                Dimension::length(px)
            } else if let Some(p) = b
                .strip_suffix('%')
                .and_then(|n| n.trim().parse::<f32>().ok())
            {
                Dimension::percent(p / 100.0)
            } else {
                dim(bs.width)
            }
        }
    }
}

fn item_margins(bs: &BoxStyle) -> Rect<LengthPercentageAuto> {
    Rect {
        left: LengthPercentageAuto::length(bs.margin.left),
        right: LengthPercentageAuto::length(bs.margin.right),
        top: LengthPercentageAuto::length(bs.margin.top),
        bottom: LengthPercentageAuto::length(bs.margin.bottom),
    }
}

fn item_style(item: &LayoutBox, fs: f32) -> Style {
    let s = &item.style;
    let bs = item.resolved(ResolveCtx {
        parent_font_size: fs,
        cb_width: 0.0,
    });
    // A `position:absolute`/`fixed` child is out of flow: taffy must NOT treat it as
    // a flex item (else `align-items:stretch` in a column flex fills it to the
    // container width, ignoring its own width â the Codex radio's absolute icon blew
    // up from 18px to the whole row). Mark it absolute so taffy sizes it from its
    // width/height + insets and excludes it from the flex line.
    if bs.position.is_out_of_flow() {
        return Style {
            position: Position::Absolute,
            inset: item_inset(&bs),
            size: item_size(&bs),
            margin: item_margins(&bs),
            ..Default::default()
        };
    }
    Style {
        flex_grow: num(s, "flex-grow", 0.0),
        flex_shrink: num(s, "flex-shrink", 1.0),
        flex_basis: item_basis(s, &bs, fs),
        margin: item_margins(&bs),
        // A flex item's own `min/max-height` (and explicit cross-axis `height`) must
        // reach taffy â else an item whose height is only a `min-height` collapses.
        // The main axis stays driven by `flex_basis`.
        size: Size {
            width: Dimension::auto(),
            height: dim(bs.height),
        },
        min_size: Size {
            width: dim(bs.min_width),
            height: dim(bs.min_height),
        },
        max_size: Size {
            width: dim(bs.max_width),
            height: dim(bs.max_height),
        },
        ..Default::default()
    }
}

/// A `LengthPct` â taffy `Dimension` (`auto` when not a fixed/percentage length).
fn dim(lp: LengthPct) -> Dimension {
    match lp {
        LengthPct::Px(v) => Dimension::length(v),
        LengthPct::Pct(p) => Dimension::percent(p / 100.0),
        LengthPct::Auto => Dimension::auto(),
    }
}

/// The width/height of an out-of-flow flex child as a taffy `Size`.
fn item_size(bs: &BoxStyle) -> Size<Dimension> {
    Size {
        width: dim(bs.width),
        height: dim(bs.height),
    }
}

/// The `inset` (top/right/bottom/left) of an out-of-flow flex child; `auto` edges
/// let taffy keep the item at its static position on that axis.
fn item_inset(bs: &BoxStyle) -> Rect<LengthPercentageAuto> {
    let edge = |lp: LengthPct| match lp {
        LengthPct::Px(v) => LengthPercentageAuto::length(v),
        LengthPct::Pct(p) => LengthPercentageAuto::percent(p / 100.0),
        LengthPct::Auto => LengthPercentageAuto::auto(),
    };
    Rect {
        left: edge(bs.inset_left),
        right: edge(bs.inset_right),
        top: edge(bs.inset_top),
        bottom: edge(bs.inset_bottom),
    }
}

// --------------------------------------------------------------------------
// content measurement
// --------------------------------------------------------------------------

fn lines_natural(items: &[InlineItem], fs: f32, fonts: &FontRegistry) -> f32 {
    let runs = block::build_runs(items, fs, 0.0, fonts);
    let mut scratch = Diagnostics::default();
    inline::layout_runs(&runs, fonts, f32::MAX, Align::Left, &mut scratch).width
}

fn kids_natural(kids: &[LayoutBox], fonts: &FontRegistry) -> f32 {
    kids.iter()
        .map(|k| natural_width(k, fonts))
        .fold(0.0_f32, f32::max)
}

/// Max-content width of a flex container. A **row** flex lays its items side by
/// side, so its max-content is the SUM of the items' widths plus the column gaps
/// (not the widest item, as for block/column) â otherwise a shrink-to-fit row flex
/// collapses to one item's width and its children wrap/stack (Wikipedia's centered
/// header did exactly this). A **column** flex stacks, so max = widest item.
fn flex_natural(kids: &[LayoutBox], s: &ComputedStyle, fonts: &FontRegistry) -> f32 {
    if matches!(
        flex_direction(s),
        FlexDirection::Row | FlexDirection::RowReverse
    ) {
        let gap = s
            .get("column-gap")
            .or_else(|| s.get("gap"))
            .and_then(|v| parse_px(v, DEFAULT_FONT_SIZE))
            .unwrap_or(0.0);
        // Each item contributes its border-box natural width PLUS its horizontal
        // margins â the items are laid side by side including those margins, so
        // omitting them undersizes the row and its children overflow (Wikipedia's
        // page-action tabs are margin-spaced; the row measured short and "View
        // history" spilled past the toolbar into the next column).
        let sum: f32 = kids
            .iter()
            .map(|k| natural_width(k, fonts) + item_hmargin(k))
            .sum();
        sum + gap * kids.len().saturating_sub(1) as f32
    } else {
        kids_natural(kids, fonts)
    }
}

/// The item's positive horizontal margins (px), which sit between it and its flex
/// siblings. Negative/auto margins contribute nothing to the intrinsic width.
fn item_hmargin(k: &LayoutBox) -> f32 {
    let bs = k.resolved(ResolveCtx {
        parent_font_size: DEFAULT_FONT_SIZE,
        cb_width: 0.0,
    });
    bs.margin.left.max(0.0) + bs.margin.right.max(0.0)
}

pub(crate) fn natural_width(lb: &LayoutBox, fonts: &FontRegistry) -> f32 {
    crate::hot!("layout.natural_width");
    lb.natural_cached(|| {
        let bs = lb.resolved(ResolveCtx {
            parent_font_size: DEFAULT_FONT_SIZE,
            cb_width: 0.0,
        });
        let frame = bs.padding.horizontal() + bs.border.widths().horizontal();
        if let LengthPct::Px(w) = bs.width {
            return w + frame;
        }
        // A replaced `<img>`: its intrinsic width (stamped from the resolver before
        // layout) â so a `<div>` wrapping a logo measures the image, not 0.
        if let Some(iw) = lb.intrinsic_w.get() {
            return iw + frame;
        }
        let inner = match &lb.kind {
            BoxKind::Lines(items) => lines_natural(items, bs.font_size, fonts),
            BoxKind::Flex(k) => flex_natural(k, &lb.style, fonts),
            BoxKind::Block(k) | BoxKind::Grid(k) | BoxKind::Table(k) => kids_natural(k, fonts),
            BoxKind::Directive(_) => 0.0,
        };
        inner + frame
    })
}

/// The widest single unbreakable piece (max word / longest line at zero available
/// width) of an inline run â the text's min-content width.
fn lines_min(items: &[InlineItem], fs: f32, fonts: &FontRegistry) -> f32 {
    let runs = block::build_runs(items, fs, 0.0, fonts);
    let mut scratch = Diagnostics::default();
    inline::layout_runs(&runs, fonts, 0.0, Align::Left, &mut scratch).width
}

/// The **min-content** width of a box: the least width it can take without its
/// content overflowing. Text wraps to its widest word; a fixed/`%`-width box and a
/// replaced image keep their declared size; a container is the widest child's
/// min-content. Used so a table never shrinks a column below its content.
pub(crate) fn min_content_width(lb: &LayoutBox, fonts: &FontRegistry) -> f32 {
    lb.min_content_cached(|| {
        let bs = lb.resolved(ResolveCtx {
            parent_font_size: DEFAULT_FONT_SIZE,
            cb_width: 0.0,
        });
        let frame = bs.padding.horizontal() + bs.border.widths().horizontal();
        if let LengthPct::Px(w) = bs.width {
            return w + frame;
        }
        if lb.image.as_ref().is_some_and(|s| s.replaced) {
            return natural_width(lb, fonts); // replaced image: intrinsic/declared size
        }
        let inner = match &lb.kind {
            BoxKind::Lines(items) => lines_min(items, bs.font_size, fonts),
            BoxKind::Flex(k) | BoxKind::Block(k) | BoxKind::Grid(k) | BoxKind::Table(k) => k
                .iter()
                .map(|c| min_content_width(c, fonts))
                .fold(0.0_f32, f32::max),
            BoxKind::Directive(_) => 0.0,
        };
        inner + frame
    })
}

fn measure_width(
    known: Option<f32>,
    avail: AvailableSpace,
    item: &LayoutBox,
    fonts: &FontRegistry,
) -> f32 {
    match (known, avail) {
        (Some(w), _) => w,
        // Cross-axis width sizing: max-content capped by the offer, NOT the full offer
        // (returning the offer made an align-items:center item fill and never center
        // — a logo column left-aligned; taffy stretches for align:stretch itself).
        (None, AvailableSpace::Definite(w)) => natural_width(item, fonts).min(w),
        (None, _) => natural_width(item, fonts),
    }
}

/// The max-content width of a replaced `<img>` flex item: its explicit `width`, else
/// the intrinsic pixel width from the resolver (plus the box frame). `None` for a
/// non-replaced box (the caller falls back to the content measurement). Without this
/// an `<img>` with `width:auto` measured 0 in a flex row â its `max-width:100%` clamps
/// against a 0-width containing block in the scratch measurement â and the item (a
/// logo / hero image) collapsed to nothing.
fn replaced_probe_width(item: &LayoutBox, images: &ImageCtx, fs: f32) -> Option<f32> {
    let src = item.image.as_ref().filter(|s| s.replaced)?;
    let bs = item.resolved(ResolveCtx {
        parent_font_size: fs,
        cb_width: 0.0,
    });
    let frame = bs.padding.horizontal() + bs.border.widths().horizontal();
    if let LengthPct::Px(w) = bs.width {
        return Some(w + frame);
    }
    let intrinsic = images
        .resolver
        .resolve(&src.name)
        .and_then(crate::image::probe)?;
    Some(intrinsic.width as f32 + frame)
}

fn measure_item(
    known: Size<Option<f32>>,
    avail: Size<AvailableSpace>,
    item: &LayoutBox,
    fs: f32,
    fonts: &FontRegistry,
    images: &ImageCtx,
    scratch: &mut Diagnostics,
) -> Size<f32> {
    // A replaced `<img>` sizes from its intrinsic (or explicit) width; other items
    // from a max-content measurement of their content.
    let w = replaced_probe_width(item, images, fs)
        .unwrap_or_else(|| measure_width(known.width, avail.width, item, fonts));
    // Memoize the full sub-layout by proposed width: taffy probes each item several
    // times per solve, and each probe recurses a full layout, so nested flex is
    // exponential without this. `fs` (the flex container's font size) is stable per
    // item, so width alone keys the cache.
    let (cw, ch) = item.measure_cached(w, || {
        let bs = item.resolved(ResolveCtx {
            parent_font_size: fs,
            cb_width: w,
        });
        let mut sd = Diagnostics::default();
        let mut mctx = Ctx {
            fonts,
            // The real resolver, so a replaced `<img>` reaches its intrinsic size in
            // the scratch layout (an empty resolver measured every image to 0).
            images,
            diags: &mut sd,
            // Scratch measurement: the item is its own containing block at origin.
            abs_cb_x: 0.0,
            abs_cb_y: 0.0,
            abs_cb_w: w,
            abs_cb_h: 0.0,
            cb_h: 0.0,
            root_w: w,
            floats: Vec::new(),
        };
        let frag = block::layout_box_sized_isolated(item, &bs, 0.0, 0.0, w, &mut mctx);
        (frag.width, frag.height)
    });
    let _ = scratch;
    Size {
        width: known.width.unwrap_or(cw),
        height: known.height.unwrap_or(ch),
    }
}

// --------------------------------------------------------------------------
// solve + placement
// --------------------------------------------------------------------------

fn build_leaves(tree: &mut TaffyTree<usize>, items: &[LayoutBox], fs: f32) -> Vec<TaffyId> {
    items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            tree.new_leaf_with_context(item_style(it, fs), i)
                .expect("flex leaf")
        })
        .collect()
}

fn solve(
    tree: &mut TaffyTree<usize>,
    root: TaffyId,
    items: &[LayoutBox],
    fs: f32,
    cw: f32,
    fonts: &FontRegistry,
    images: &ImageCtx,
) {
    let mut scratch = Diagnostics::default();
    let avail = Size {
        width: AvailableSpace::Definite(cw),
        height: AvailableSpace::MaxContent,
    };
    tree.compute_layout_with_measure(root, avail, |known, av, _node, ctx_idx, _style| {
        let idx = *ctx_idx.expect("leaf context");
        measure_item(known, av, &items[idx], fs, fonts, images, &mut scratch)
    })
    .expect("flex layout");
}

fn place_one(
    item: &LayoutBox,
    layout: &Layout,
    cx: f32,
    cy: f32,
    fs: f32,
    ctx: &mut Ctx,
) -> Fragment {
    let bs = item.resolved(ResolveCtx {
        parent_font_size: fs,
        cb_width: layout.size.width,
    });
    let mut frag = block::layout_box_sized_isolated(item, &bs, 0.0, 0.0, layout.size.width, ctx);
    frag.translate(cx + layout.location.x, cy + layout.location.y);
    frag
}

fn place_items(
    tree: &TaffyTree<usize>,
    leaves: &[TaffyId],
    items: &[LayoutBox],
    cx: f32,
    cy: f32,
    fs: f32,
    ctx: &mut Ctx,
) -> Vec<Fragment> {
    let mut frags = Vec::new();
    for (i, leaf) in leaves.iter().enumerate() {
        let layout = tree.layout(*leaf).expect("item layout");
        frags.push(place_one(&items[i], layout, cx, cy, fs, ctx));
    }
    frags
}

// --------------------------------------------------------------------------
// grid (taffy owns the grid algorithm; we map CSS templates + gaps)
// --------------------------------------------------------------------------

/// One explicit-gap axis: `column-gap`/`row-gap`, falling back to `gap`.
fn gap_axis(s: &ComputedStyle, axis: &str) -> LengthPercentage {
    let px = s
        .get(axis)
        .or_else(|| s.get("gap"))
        .and_then(|v| parse_px(v, DEFAULT_FONT_SIZE))
        .unwrap_or(0.0);
    LengthPercentage::length(px)
}

/// Parse a `minmax(min, max)` track, if `t` is one (both sides required).
fn minmax_track(t: &str) -> Option<TrackSizingFunction> {
    let inner = t
        .strip_prefix("minmax(")
        .and_then(|x| x.strip_suffix(')'))?;
    let (a, b) = inner.split_once(',')?;
    Some(minmax(min_track(a), max_track(b)))
}

/// One grid track: `1fr`, `50%`, `200px`/`15.5rem`, `minmax(min, max)`, or
/// `auto`/`min-content`/`max-content` (â taffy `AUTO`). Unparsable â `AUTO`.
fn track_of(tok: &str) -> TrackSizingFunction {
    let t = tok.trim();
    if let Some(mm) = minmax_track(t) {
        return mm;
    }
    if let Some(f) = t
        .strip_suffix("fr")
        .and_then(|x| x.trim().parse::<f32>().ok())
    {
        return fr(f);
    }
    if let Some(p) = t
        .strip_suffix('%')
        .and_then(|x| x.trim().parse::<f32>().ok())
    {
        return percent(p / 100.0);
    }
    if let Some(px) = parse_px(t, DEFAULT_FONT_SIZE) {
        return TrackSizingFunction::from_length(px);
    }
    TrackSizingFunction::AUTO
}

/// The min side of a `minmax()` (no `fr` allowed): length/`%`, else `auto`.
fn min_track(t: &str) -> MinTrackSizingFunction {
    let t = t.trim();
    if let Some(p) = t
        .strip_suffix('%')
        .and_then(|x| x.trim().parse::<f32>().ok())
    {
        return MinTrackSizingFunction::from_percent(p / 100.0);
    }
    if !t.ends_with("fr") {
        if let Some(px) = parse_px(t, DEFAULT_FONT_SIZE) {
            return MinTrackSizingFunction::from_length(px);
        }
    }
    MinTrackSizingFunction::AUTO
}

/// The max side of a `minmax()`: `fr`/length/`%`, else `auto`.
fn max_track(t: &str) -> MaxTrackSizingFunction {
    let t = t.trim();
    if let Some(f) = t
        .strip_suffix("fr")
        .and_then(|x| x.trim().parse::<f32>().ok())
    {
        return fr(f);
    }
    if let Some(p) = t
        .strip_suffix('%')
        .and_then(|x| x.trim().parse::<f32>().ok())
    {
        return MaxTrackSizingFunction::from_percent(p / 100.0);
    }
    if let Some(px) = parse_px(t, DEFAULT_FONT_SIZE) {
        return MaxTrackSizingFunction::from_length(px);
    }
    MaxTrackSizingFunction::AUTO
}

/// A `grid-template-areas` map: area name â the grid-line rectangle it covers
/// `(row_start, row_end, col_start, col_end)`, 0-based cell indices (converted to
/// 1-based taffy lines at use).
type AreaMap = HashMap<String, (i16, i16, i16, i16)>;

/// Parse `grid-template-areas: "a b" "a c"` (each quoted string is a row of
/// space-separated cell names; `.` is an empty cell) into an [`AreaMap`].
fn grid_areas(value: &str) -> AreaMap {
    let mut map: AreaMap = HashMap::new();
    for (r, row) in quoted_rows(value).into_iter().enumerate() {
        for (c, name) in row.split_whitespace().enumerate() {
            if name == "." {
                continue;
            }
            let (r, c) = (r as i16, c as i16);
            let cell = map.entry(name.to_string()).or_insert((r, r, c, c));
            cell.0 = cell.0.min(r);
            cell.1 = cell.1.max(r);
            cell.2 = cell.2.min(c);
            cell.3 = cell.3.max(c);
        }
    }
    map
}

/// The quoted row strings of a `grid-template-areas` value (`'â¦'` or `"â¦"`).
fn quoted_rows(value: &str) -> Vec<String> {
    let mut rows = Vec::new();
    let mut rest = value;
    while let Some(open) = rest.find(['"', '\'']) {
        let quote = rest.as_bytes()[open] as char;
        rest = &rest[open + 1..];
        match rest.find(quote) {
            Some(close) => {
                rows.push(rest[..close].to_string());
                rest = &rest[close + 1..];
            }
            None => break,
        }
    }
    rows
}

/// Column count implied by a `grid-template-areas` map (max column line used).
fn area_cols(areas: &AreaMap) -> usize {
    areas
        .values()
        .map(|&(_, _, _, c1)| c1 as usize + 1)
        .max()
        .unwrap_or(0)
}

/// Row count implied by a `grid-template-areas` map (max row line used).
fn area_rows(areas: &AreaMap) -> usize {
    areas
        .values()
        .map(|&(_, r1, _, _)| r1 as usize + 1)
        .max()
        .unwrap_or(0)
}

/// Keep explicit `tracks` if present; otherwise synthesize `n` AUTO tracks so a
/// grid declared only via `grid-template-areas` still has tracks to place into.
fn fill_tracks(tracks: Vec<TrackSizingFunction>, n: usize) -> Vec<TrackSizingFunction> {
    if tracks.is_empty() {
        vec![TrackSizingFunction::AUTO; n]
    } else {
        tracks
    }
}

/// A grid item's row/column line spans from `grid-area: <name>` resolved against
/// the container's `grid-template-areas`. `None` when the item names no area (taffy
/// then auto-places it). Grid lines are 1-based with an exclusive end line.
fn area_placement(
    item: &LayoutBox,
    areas: &AreaMap,
) -> Option<(Line<GridPlacement>, Line<GridPlacement>)> {
    let name = item.style.get("grid-area")?.trim();
    let &(r0, r1, c0, c1) = areas.get(name)?;
    let row = Line {
        start: line(r0 + 1),
        end: line(r1 + 2),
    };
    let col = Line {
        start: line(c0 + 1),
        end: line(c1 + 2),
    };
    Some((row, col))
}

/// `repeat(N, <tracks>)` expanded into N copies of its track list (integer count
/// only; `auto-fill`/`auto-fit` fall through to a single `AUTO` track).
fn parse_repeat(tok: &str) -> Option<Vec<TrackSizingFunction>> {
    let inner = tok.trim().strip_prefix("repeat(")?.strip_suffix(')')?;
    let (count, tracks) = inner.split_once(',')?;
    let count: usize = count.trim().parse().ok()?;
    let one: Vec<TrackSizingFunction> = super::value::css_value_tokens(tracks)
        .into_iter()
        .map(track_of)
        .collect();
    Some(
        one.iter()
            .cloned()
            .cycle()
            .take(count * one.len())
            .collect(),
    )
}

/// Parse a `grid-template-columns`/`-rows` value into taffy tracks. Empty /
/// `none` â no explicit tracks (taffy's implicit-grid auto-placement applies).
fn grid_tracks(spec: Option<&str>) -> Vec<TrackSizingFunction> {
    let Some(spec) = spec
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "none")
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for tok in super::value::css_value_tokens(spec) {
        match parse_repeat(tok) {
            Some(rep) => out.extend(rep),
            None => out.push(track_of(tok)),
        }
    }
    out
}

/// The track list for one axis: the longhand `grid-template-columns`/`-rows`, or,
/// if absent, the corresponding side of the `grid-template` / `grid` shorthand
/// (`<rows> / <cols>`). Parsing the shorthand matters for real pages (e.g.
/// Wikipedia's Vector `grid-template: â¦ / 12.25rem minmax(0,1fr)`): without it the
/// axis falls back to AUTO tracks, which with named areas makes taffy content-size
/// a huge subtree per track â pathologically slow â instead of using fixed tracks.
fn axis_tracks(s: &ComputedStyle, longhand: &str, want_cols: bool) -> Vec<TrackSizingFunction> {
    let explicit = grid_tracks(s.get(longhand));
    if !explicit.is_empty() {
        return explicit;
    }
    grid_tracks(shorthand_axis(s, want_cols).as_deref())
}

/// The rows (`before /`) or columns (`after /`) side of a `grid-template` / `grid`
/// shorthand, with any quoted `grid-template-areas` row strings stripped out.
fn shorthand_axis(s: &ComputedStyle, want_cols: bool) -> Option<String> {
    let value = s.get("grid-template").or_else(|| s.get("grid"))?;
    let (rows, cols) = value.split_once('/')?;
    let part = strip_quoted(if want_cols { cols } else { rows });
    let part = part.trim();
    (!part.is_empty()).then(|| part.to_string())
}

/// Advance `chars` past the next occurrence of `quote` (its closing delimiter).
fn skip_quoted(chars: &mut impl Iterator<Item = char>, quote: char) {
    for c in chars {
        if c == quote {
            break;
        }
    }
}

/// Remove single/double-quoted segments (area-template rows) from a value.
fn strip_quoted(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '"' || ch == '\'' {
            skip_quoted(&mut chars, ch);
        } else {
            out.push(ch);
        }
    }
    out
}

fn grid_container_style(container: &LayoutBox, cw: f32, areas: &AreaMap) -> Style {
    let s = &container.style;
    // Explicit tracks (longhand or the `grid-template` shorthand), else â when only
    // `grid-template-areas` is given â one AUTO track per area column/row so named
    // placement still has a grid to land in.
    let cols = axis_tracks(s, "grid-template-columns", true);
    let rows = axis_tracks(s, "grid-template-rows", false);
    let cols = fill_tracks(cols, area_cols(areas));
    let rows = fill_tracks(rows, area_rows(areas));
    Style {
        display: Display::Grid,
        grid_template_columns: cols.into_iter().collect(),
        grid_template_rows: rows.into_iter().collect(),
        gap: Size {
            width: gap_axis(s, "column-gap"),
            height: gap_axis(s, "row-gap"),
        },
        justify_content: justify_content(s),
        align_items: align_items(s),
        size: Size {
            width: Dimension::length(cw),
            height: Dimension::auto(),
        },
        ..Default::default()
    }
}

/// Grid leaves, each carrying its `grid-area` line placement (against `areas`)
/// on top of the shared item style; unnamed items keep taffy auto-placement.
fn build_grid_leaves(
    tree: &mut TaffyTree<usize>,
    items: &[LayoutBox],
    fs: f32,
    areas: &AreaMap,
) -> Vec<TaffyId> {
    items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let mut style = item_style(it, fs);
            if let Some((row, col)) = area_placement(it, areas) {
                style.grid_row = row;
                style.grid_column = col;
            }
            tree.new_leaf_with_context(style, i).expect("grid leaf")
        })
        .collect()
}

/// Lay out a grid container's items into the content box at `(cx, cy)` of width
/// `cw`. Items are placed by `grid-area` (against the container's
/// `grid-template-areas`) or auto-placed into the tracks. Returns galley-absolute
/// fragments and the content height.
pub(crate) fn layout_grid(
    container: &LayoutBox,
    items: &[LayoutBox],
    cx: f32,
    cy: f32,
    cw: f32,
    fs: f32,
    ctx: &mut Ctx,
) -> (Vec<Fragment>, f32) {
    if items.is_empty() {
        return (Vec::new(), 0.0);
    }
    let areas = container
        .style
        .get("grid-template-areas")
        .map(grid_areas)
        .unwrap_or_default();
    let mut tree: TaffyTree<usize> = TaffyTree::new();
    let leaves = build_grid_leaves(&mut tree, items, fs, &areas);
    let root = tree
        .new_with_children(grid_container_style(container, cw, &areas), &leaves)
        .expect("grid root");
    solve(&mut tree, root, items, fs, cw, ctx.fonts, ctx.images);
    let frags = place_items(&tree, &leaves, items, cx, cy, fs, ctx);
    let height = tree.layout(root).expect("root layout").size.height;
    (frags, height)
}

/// Lay out a flex container's items into the content box at `(cx, cy)` of width
/// `cw`. Returns the item fragments (galley-absolute) and the content height.
pub(crate) fn layout_flex(
    container: &LayoutBox,
    items: &[LayoutBox],
    cx: f32,
    cy: f32,
    cw: f32,
    fs: f32,
    ctx: &mut Ctx,
) -> (Vec<Fragment>, f32) {
    if items.is_empty() {
        return (Vec::new(), 0.0);
    }
    let mut tree: TaffyTree<usize> = TaffyTree::new();
    let leaves = build_leaves(&mut tree, items, fs);
    let root = tree
        .new_with_children(container_style(container, cw, fs), &leaves)
        .expect("flex root");
    solve(&mut tree, root, items, fs, cw, ctx.fonts, ctx.images);
    let frags = place_items(&tree, &leaves, items, cx, cy, fs, ctx);
    let height = tree.layout(root).expect("root layout").size.height;
    (frags, height)
}

#[cfg(test)]
mod coverage_tests {
    use super::*;
    use crate::layout::boxgen::build_box_tree;
    use crate::node::{Attr, TKind, Tag};
    use crate::text::FontRegistry;
    use crate::{StyledElement, StyledNode};

    fn cs(pairs: &[(&str, &str)]) -> ComputedStyle {
        ComputedStyle::from_pairs(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
    }

    fn bs_of(pairs: &[(&str, &str)]) -> BoxStyle {
        super::super::value::resolve_box_style(
            &cs(pairs),
            ResolveCtx {
                parent_font_size: 16.0,
                cb_width: 200.0,
            },
        )
    }

    fn el(tag: &str, pairs: &[(&str, &str)], children: Vec<StyledNode>) -> StyledNode {
        StyledNode::Element(StyledElement {
            tag: Tag::Html(tag.to_string()),
            attrs: vec![],
            style: cs(pairs),
            children,
        })
    }

    fn text_item(t: &str) -> StyledNode {
        el("div", &[], vec![StyledNode::Text(t.to_string())])
    }

    // --- item_basis: flex-basis %, and the non-length fallback to `width` ---
    #[test]
    fn item_basis_percent_and_width_fallback() {
        // `flex-basis: 50%` -> a percentage dimension (the `strip_suffix('%')` arm).
        let s = cs(&[("flex-basis", "50%")]);
        let bs = bs_of(&[("flex-basis", "50%")]);
        assert_eq!(item_basis(&s, &bs, 16.0), Dimension::percent(0.5));

        // A basis that is neither a length nor a percentage (`min-content`) falls back
        // to the item's own `width`.
        let s = cs(&[("flex-basis", "min-content"), ("width", "77px")]);
        let bs = bs_of(&[("flex-basis", "min-content"), ("width", "77px")]);
        assert_eq!(item_basis(&s, &bs, 16.0), Dimension::length(77.0));
    }

    // --- item_inset: a percentage inset edge on an out-of-flow flex child ---
    #[test]
    fn item_inset_percentage_edge() {
        let bs = bs_of(&[("top", "25%"), ("left", "10%")]);
        let inset = item_inset(&bs);
        assert_eq!(inset.top, LengthPercentageAuto::percent(0.25));
        assert_eq!(inset.left, LengthPercentageAuto::percent(0.10));
    }

    // --- flex_natural: row sums items, column takes the widest ---
    #[test]
    fn flex_natural_row_and_column() {
        let fonts = FontRegistry::new();
        let row = build_box_tree(&[el(
            "div",
            &[("display", "flex")],
            vec![text_item("aa"), text_item("bbbb")],
        )]);
        let col = build_box_tree(&[el(
            "div",
            &[("display", "flex"), ("flex-direction", "column")],
            vec![text_item("aa"), text_item("bbbb")],
        )]);
        let rw = natural_width(&row, &fonts);
        let cw = natural_width(&col, &fonts);
        // Row lays items side by side (sum), column stacks them (widest) -> row >= col.
        assert!(rw >= cw);
        assert!(rw >= 0.0 && cw >= 0.0);
    }

    // --- min_content_width: replaced image path + directive (zero) path ---
    #[test]
    fn min_content_replaced_image_and_directive() {
        let fonts = FontRegistry::new();

        // A replaced `<img src>` (no explicit width) takes the intrinsic/declared
        // branch of `min_content_width`. Wrapped as a flex item so it stays a box.
        let img = StyledNode::Element(StyledElement {
            tag: Tag::Html("img".to_string()),
            attrs: vec![Attr {
                name: "src".to_string(),
                value: "x.png".to_string(),
            }],
            style: cs(&[]),
            children: vec![],
        });
        let root = build_box_tree(&[el("div", &[("display", "flex")], vec![img])]);
        assert!(min_content_width(&root, &fonts) >= 0.0);

        // A paged-media directive box has zero min-content width.
        let directive = StyledNode::Element(StyledElement {
            tag: Tag::Directive(TKind::Anchor),
            attrs: vec![],
            style: cs(&[]),
            children: vec![],
        });
        let root = build_box_tree(&[el("div", &[("display", "flex")], vec![directive])]);
        assert_eq!(min_content_width(&root, &fonts), 0.0);
    }

    // --- grid track parsers: percentage tracks and the AUTO fallbacks ---
    #[test]
    fn track_parsers_percent_and_auto_branches() {
        // An explicit percentage track.
        assert_eq!(track_of("50%"), percent(0.5));
        // minmax() min side: percentage, and `fr` (not a valid min) -> AUTO.
        assert_eq!(min_track("50%"), MinTrackSizingFunction::from_percent(0.5));
        assert_eq!(min_track("1fr"), MinTrackSizingFunction::AUTO);
        // minmax() max side: percentage, and a non-length (`auto`) -> AUTO.
        assert_eq!(max_track("50%"), MaxTrackSizingFunction::from_percent(0.5));
        assert_eq!(max_track("auto"), MaxTrackSizingFunction::AUTO);
        // A full minmax() track round-trips both sides.
        assert_eq!(
            track_of("minmax(50%, 1fr)"),
            minmax(MinTrackSizingFunction::from_percent(0.5), fr(1.0))
        );
    }

    // --- grid-template-areas quoting: `.` cells, unbalanced quotes, strip_quoted ---
    #[test]
    fn grid_area_quoting_helpers() {
        // `.` marks an empty cell and is skipped; named cells become rectangles.
        let map = grid_areas("'a .' 'a b'");
        assert!(map.contains_key("a"));
        assert!(map.contains_key("b"));
        assert!(!map.contains_key("."));

        // An unterminated quote stops row scanning cleanly (no infinite loop).
        assert!(quoted_rows("'unterminated").is_empty());

        // `strip_quoted` drops the quoted area-row segment, keeping the track list.
        assert_eq!(strip_quoted("10px 'a b' 1fr").trim(), "10px  1fr".trim());
    }
}
