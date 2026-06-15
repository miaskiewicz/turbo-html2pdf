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

use taffy::{
    AlignItems, AvailableSpace, Dimension, Display, FlexDirection, FlexWrap, JustifyContent,
    Layout, LengthPercentage, LengthPercentageAuto, NodeId as TaffyId, Rect, Size, Style,
    TaffyTree,
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
    Style {
        flex_grow: num(s, "flex-grow", 0.0),
        flex_shrink: num(s, "flex-shrink", 1.0),
        flex_basis: item_basis(s, fs),
        margin: item_margins(&bs),
        ..Default::default()
    }
}

// --------------------------------------------------------------------------
// content measurement
// --------------------------------------------------------------------------

fn lines_natural(items: &[InlineItem], fs: f32, fonts: &FontRegistry) -> f32 {
    let runs = block::build_runs(items, fs, 0.0, fonts);
    let mut scratch = Diagnostics::default();
    inline::layout_paragraph(&runs, fonts, f32::MAX, Align::Left, &mut scratch).width
}

fn kids_natural(kids: &[LayoutBox], fonts: &FontRegistry) -> f32 {
    kids.iter()
        .map(|k| natural_width(k, fonts))
        .fold(0.0_f32, f32::max)
}

pub(crate) fn natural_width(lb: &LayoutBox, fonts: &FontRegistry) -> f32 {
    crate::hot!("layout.natural_width");
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
        BoxKind::Block(k) | BoxKind::Flex(k) | BoxKind::Table(k) => kids_natural(k, fonts),
        BoxKind::Directive(_) => 0.0,
    };
    inner + frame
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
    };
    let frag = block::layout_box_sized(item, &bs, 0.0, 0.0, w, &mut mctx);
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
    let mut frag = block::layout_box_sized(item, &bs, 0.0, 0.0, layout.size.width, ctx);
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
