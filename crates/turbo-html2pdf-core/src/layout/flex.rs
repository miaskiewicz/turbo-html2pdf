//! Flex layout (§5.3, AC-5.6). `taffy` owns the flexbox math (direction, wrap,
//! grow/shrink/basis, justify/align, gap); we map CSS to `taffy::Style`, feed it
//! each item's measured size, read back rects, then re-lay each item's content at
//! its assigned width (§5.3 decision: taffy owns flex, the engine owns the rest).
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

fn container_style(container: &LayoutBox, cw: f32) -> Style {
    let s = &container.style;
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
        size: Size {
            width: Dimension::length(cw),
            height: Dimension::auto(),
        },
        ..Default::default()
    }
}

fn num(s: &ComputedStyle, prop: &str, default: f32) -> f32 {
    s.get(prop)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn item_basis(s: &ComputedStyle, fs: f32) -> Dimension {
    if let Some(b) = s.get("flex-basis") {
        if let Some(px) = parse_px(b, fs) {
            return Dimension::length(px);
        }
    }
    Dimension::auto()
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
    // container width, ignoring its own width — the Codex radio's absolute icon blew
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
        flex_basis: item_basis(s, fs),
        margin: item_margins(&bs),
        ..Default::default()
    }
}

/// A `LengthPct` → taffy `Dimension` (`auto` when not a fixed/percentage length).
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
/// (not the widest item, as for block/column) — otherwise a shrink-to-fit row flex
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
        let sum: f32 = kids.iter().map(|k| natural_width(k, fonts)).sum();
        sum + gap * kids.len().saturating_sub(1) as f32
    } else {
        kids_natural(kids, fonts)
    }
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
/// width) of an inline run — the text's min-content width.
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
        (None, AvailableSpace::Definite(w)) => w,
        (None, _) => natural_width(item, fonts),
    }
}

fn measure_item(
    known: Size<Option<f32>>,
    avail: Size<AvailableSpace>,
    item: &LayoutBox,
    fs: f32,
    fonts: &FontRegistry,
    scratch: &mut Diagnostics,
) -> Size<f32> {
    let w = measure_width(known.width, avail.width, item, fonts);
    let bs = item.resolved(ResolveCtx {
        parent_font_size: fs,
        cb_width: w,
    });
    let images = super::ImageCtx::none();
    let mut mctx = Ctx {
        fonts,
        images: &images,
        diags: scratch,
        // Scratch measurement: the item is its own containing block at origin.
        abs_cb_x: 0.0,
        abs_cb_y: 0.0,
        abs_cb_w: w,
        root_w: w,
        floats: Vec::new(),
    };
    let frag = block::layout_box_sized_isolated(item, &bs, 0.0, 0.0, w, &mut mctx);
    Size {
        width: known.width.unwrap_or(frag.width),
        height: known.height.unwrap_or(frag.height),
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
) {
    let mut scratch = Diagnostics::default();
    let avail = Size {
        width: AvailableSpace::Definite(cw),
        height: AvailableSpace::MaxContent,
    };
    tree.compute_layout_with_measure(root, avail, |known, av, _node, ctx_idx, _style| {
        let idx = *ctx_idx.expect("leaf context");
        measure_item(known, av, &items[idx], fs, fonts, &mut scratch)
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

/// One grid track: `1fr`, `50%`, `200px`/`15.5rem`, `minmax(min, max)`, or
/// `auto`/`min-content`/`max-content` (→ taffy `AUTO`). Unparsable → `AUTO`.
fn track_of(tok: &str) -> TrackSizingFunction {
    let t = tok.trim();
    if let Some(inner) = t.strip_prefix("minmax(").and_then(|x| x.strip_suffix(')')) {
        if let Some((a, b)) = inner.split_once(',') {
            return minmax(min_track(a), max_track(b));
        }
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

/// A `grid-template-areas` map: area name → the grid-line rectangle it covers
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

/// The quoted row strings of a `grid-template-areas` value (`'…'` or `"…"`).
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
/// `none` → no explicit tracks (taffy's implicit-grid auto-placement applies).
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
/// Wikipedia's Vector `grid-template: … / 12.25rem minmax(0,1fr)`): without it the
/// axis falls back to AUTO tracks, which with named areas makes taffy content-size
/// a huge subtree per track — pathologically slow — instead of using fixed tracks.
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

/// Remove single/double-quoted segments (area-template rows) from a value.
fn strip_quoted(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '"' || ch == '\'' {
            for c in chars.by_ref() {
                if c == ch {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn grid_container_style(container: &LayoutBox, cw: f32, areas: &AreaMap) -> Style {
    let s = &container.style;
    // Explicit tracks (longhand or the `grid-template` shorthand), else — when only
    // `grid-template-areas` is given — one AUTO track per area column/row so named
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
    solve(&mut tree, root, items, fs, cw, ctx.fonts);
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
        .new_with_children(container_style(container, cw), &leaves)
        .expect("flex root");
    solve(&mut tree, root, items, fs, cw, ctx.fonts);
    let frags = place_items(&tree, &leaves, items, cx, cy, fs, ctx);
    let height = tree.layout(root).expect("root layout").size.height;
    (frags, height)
}
