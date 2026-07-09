//! Table layout (§5.4, AC-5.7–5.10). Builds a cell grid (with `colspan`/`rowspan`
//! occupancy), resolves column widths by the `fixed` or `auto` algorithm, lays
//! each cell out at its column-span width, sizes rows to their tallest cell, and
//! applies per-cell `vertical-align`. `<thead>`/`<tfoot>` rows are marked
//! repeatable (`BreakMeta.repeatable`) so the fragmenter can re-emit them on each
//! page a table spans (§6.3, AC-5.8).
//!
//! Deferred in v1 (documented): `<caption>`, `border-spacing`, and full
//! `border-collapse` border merging (collapse and separate both lay out with no
//! spacing); a cell taller than its row span expands the last spanned row.

use crate::node::Attr;
use crate::style::ComputedStyle;
use crate::text::FontRegistry;

use super::block::{self, Ctx};
use super::boxgen::{BoxKind, LayoutBox};
use super::flex::natural_width;
use super::fragment::{Fragment, FragmentContent, NodeId, RepeatKind};
use super::value::{
    parse_px, resolve_box_style, BorderEdges, Display, LengthPct, ResolveCtx, VAlign,
    DEFAULT_FONT_SIZE,
};

// --------------------------------------------------------------------------
// row collection
// --------------------------------------------------------------------------

struct RowRef<'a> {
    node_id: NodeId,
    cells: Vec<&'a LayoutBox>,
    /// The row's explicit `height` in px (0 if none) — a floor on the row height,
    /// so an empty spacer row (`<tr style="height:5px">`, common in table layouts
    /// like Hacker News) reserves its space instead of collapsing to zero.
    min_height: f32,
    repeat: Option<RepeatKind>,
    /// The row's PDF/UA structure role (`pdf-ua`), so the synthetic row fragment
    /// can carry `TableRow` for the tagged-PDF struct tree (AC-11.1).
    #[cfg(feature = "pdf-ua")]
    ua_role: Option<crate::layout::fragment::UaRole>,
}

fn cells_of(row: &LayoutBox) -> Vec<&LayoutBox> {
    match &row.kind {
        BoxKind::Block(cells) => cells.iter().collect(),
        _ => Vec::new(),
    }
}

fn rows_of(group: &LayoutBox) -> Vec<&LayoutBox> {
    match &group.kind {
        BoxKind::Block(rows) => rows.iter().collect(),
        _ => Vec::new(),
    }
}

fn group_repeat(d: Display) -> Option<RepeatKind> {
    match d {
        Display::TableHeaderGroup => Some(RepeatKind::Header),
        Display::TableFooterGroup => Some(RepeatKind::Footer),
        _ => None,
    }
}

fn row_ref<'a>(row: &'a LayoutBox, repeat: Option<RepeatKind>) -> RowRef<'a> {
    RowRef {
        node_id: row.node_id,
        cells: cells_of(row),
        min_height: row_min_height(row),
        repeat,
        #[cfg(feature = "pdf-ua")]
        ua_role: row.ua_role,
    }
}

/// A table row's explicit `height` in px (0 if unset/relative) — used as a floor
/// on the computed row height.
fn row_min_height(row: &LayoutBox) -> f32 {
    row.style
        .get("height")
        .and_then(|v| parse_px(v, DEFAULT_FONT_SIZE))
        .unwrap_or(0.0_f32)
        .max(0.0)
}

fn collect_rows<'a>(items: &'a [LayoutBox]) -> Vec<RowRef<'a>> {
    let mut out = Vec::new();
    for child in items {
        if matches!(child.display, Display::TableRow) {
            out.push(row_ref(child, None));
        } else {
            let repeat = group_repeat(child.display);
            out.extend(rows_of(child).into_iter().map(|r| row_ref(r, repeat)));
        }
    }
    out
}

// --------------------------------------------------------------------------
// grid placement (colspan / rowspan occupancy)
// --------------------------------------------------------------------------

struct Placed<'a> {
    lb: &'a LayoutBox,
    row: usize,
    col: usize,
    colspan: usize,
    rowspan: usize,
}

fn attr_usize(attrs: &[Attr], name: &str) -> usize {
    attrs
        .iter()
        .find(|a| a.name == name)
        .and_then(|a| a.value.trim().parse().ok())
        .filter(|n| *n >= 1)
        .unwrap_or(1)
}

fn ensure(occ: &mut Vec<Vec<bool>>, rows: usize, cols: usize) {
    while occ.len() < rows {
        occ.push(Vec::new());
    }
    for row in occ.iter_mut() {
        while row.len() < cols {
            row.push(false);
        }
    }
}

fn next_free(row: &[bool], start: usize) -> usize {
    let mut c = start;
    while c < row.len() && row[c] {
        c += 1;
    }
    c
}

fn mark(occ: &mut Vec<Vec<bool>>, r: usize, col: usize, cs: usize, rs: usize) {
    ensure(occ, r + rs, col + cs);
    for row in occ.iter_mut().take(r + rs).skip(r) {
        row[col..col + cs].fill(true);
    }
}

fn place_row<'a>(
    occ: &mut Vec<Vec<bool>>,
    row: &RowRef<'a>,
    r: usize,
    placed: &mut Vec<Placed<'a>>,
) -> usize {
    ensure(occ, r + 1, 0);
    let mut col = 0;
    let mut max_col = 0;
    for cell in &row.cells {
        col = next_free(&occ[r], col);
        let colspan = attr_usize(&cell.attrs, "colspan");
        let rowspan = attr_usize(&cell.attrs, "rowspan");
        mark(occ, r, col, colspan, rowspan);
        placed.push(Placed {
            lb: cell,
            row: r,
            col,
            colspan,
            rowspan,
        });
        col += colspan;
        max_col = max_col.max(col);
    }
    max_col
}

fn build_grid<'a>(rows: &[RowRef<'a>]) -> (Vec<Placed<'a>>, usize) {
    let mut occ: Vec<Vec<bool>> = Vec::new();
    let mut placed = Vec::new();
    let mut ncols = 0;
    for (r, row) in rows.iter().enumerate() {
        ncols = ncols.max(place_row(&mut occ, row, r, &mut placed));
    }
    (placed, ncols)
}

// --------------------------------------------------------------------------
// column widths
// --------------------------------------------------------------------------

fn is_fixed(style: &ComputedStyle) -> bool {
    style.get("table-layout").map(str::trim) == Some("fixed")
}

fn explicit_width(lb: &LayoutBox) -> Option<f32> {
    let bs = lb.resolved(ResolveCtx {
        parent_font_size: DEFAULT_FONT_SIZE,
        cb_width: 0.0,
    });
    match bs.width {
        LengthPct::Px(w) => Some(w + bs.padding.horizontal() + bs.border.widths().horizontal()),
        _ => None,
    }
}

fn cell_span_width(cols: &[f32], col: usize, span: usize) -> f32 {
    cols[col..col + span].iter().sum()
}

fn auto_columns(placed: &[Placed], ncols: usize, fonts: &FontRegistry) -> Vec<f32> {
    let mut w = vec![0.0_f32; ncols];
    for p in placed {
        if p.colspan == 1 {
            w[p.col] = w[p.col].max(natural_width(p.lb, fonts));
        }
    }
    for p in placed {
        fit_span(&mut w, p, fonts);
    }
    w
}

fn fit_span(w: &mut [f32], p: &Placed, fonts: &FontRegistry) {
    if p.colspan > 1 {
        let have = cell_span_width(w, p.col, p.colspan);
        let need = natural_width(p.lb, fonts);
        if need > have {
            w[p.col + p.colspan - 1] += need - have;
        }
    }
}

fn first_row_widths(placed: &[Placed], ncols: usize) -> Vec<Option<f32>> {
    let mut w = vec![None; ncols];
    for p in placed {
        if p.row == 0 && p.colspan == 1 && w[p.col].is_none() {
            w[p.col] = explicit_width(p.lb);
        }
    }
    w
}

fn share(remaining: f32, count: usize) -> f32 {
    if count == 0 {
        0.0
    } else {
        (remaining / count as f32).max(0.0)
    }
}

fn fixed_columns(placed: &[Placed], ncols: usize, table_width: f32) -> Vec<f32> {
    let w = first_row_widths(placed, ncols);
    let specified: f32 = w.iter().flatten().sum();
    let unspec = w.iter().filter(|x| x.is_none()).count();
    let each = share(table_width - specified, unspec);
    w.iter().map(|x| x.unwrap_or(each)).collect()
}

/// Each column's min-content width (widest single-column cell that can't shrink).
fn min_columns(placed: &[Placed], ncols: usize, fonts: &FontRegistry) -> Vec<f32> {
    let mut w = vec![0.0_f32; ncols];
    for p in placed {
        if p.colspan == 1 {
            w[p.col] = w[p.col].max(super::flex::min_content_width(p.lb, fonts));
        }
    }
    w
}

/// Fit `cols` to `target`. Growing scales up proportionally; shrinking takes only
/// from each column's slack above its min-content, so a column never clips its
/// content (the infobox label column kept clipping "Infraclass:" when the whole
/// row was scaled down uniformly). If even the minima don't fit, columns sit at
/// their min-content and the table overflows.
fn scale_to(cols: &mut [f32], target: f32, mins: &[f32]) {
    let sum: f32 = cols.iter().sum();
    if sum <= 0.0 || (sum - target).abs() <= f32::EPSILON {
        return;
    }
    if target >= sum {
        let k = target / sum;
        for c in cols.iter_mut() {
            *c *= k;
        }
        return;
    }
    let slack: f32 = cols.iter().zip(mins).map(|(c, m)| (c - m).max(0.0)).sum();
    let reduce = sum - target;
    for (c, m) in cols.iter_mut().zip(mins) {
        if slack > 0.0 {
            *c -= reduce * (*c - *m).max(0.0) / slack;
        }
        *c = c.max(*m);
    }
}

fn explicit_table_width(style: &ComputedStyle, cw: f32) -> Option<f32> {
    let bs = resolve_box_style(
        style,
        ResolveCtx {
            parent_font_size: DEFAULT_FONT_SIZE,
            cb_width: cw,
        },
    );
    bs.width.resolve(cw)
}

fn column_widths(
    style: &ComputedStyle,
    placed: &[Placed],
    ncols: usize,
    cw: f32,
    fonts: &FontRegistry,
) -> Vec<f32> {
    // `cw` is the table's already-resolved content width (its box was sized from
    // the `width` declaration upstream), so a table with an explicit width fills
    // `cw`. Re-resolving the `width` value here would apply a percentage twice
    // (e.g. an 85%-wide table's columns collapsing to 85% of 85%).
    let has_width = explicit_table_width(style, cw).is_some();
    if is_fixed(style) {
        return fixed_columns(placed, ncols, cw);
    }
    let mut cols = auto_columns(placed, ncols, fonts);
    let target = if has_width {
        cw
    } else {
        cols.iter().sum::<f32>().min(cw)
    };
    let mins = min_columns(placed, ncols, fonts);
    scale_to(&mut cols, target, &mins);
    cols
}

// --------------------------------------------------------------------------
// cell layout, row heights, placement
// --------------------------------------------------------------------------

struct LaidCell<'a> {
    p: &'a Placed<'a>,
    frag: Fragment,
    valign: VAlign,
    content_h: f32,
}

fn layout_one<'a>(p: &'a Placed<'a>, cols: &[f32], fs: f32, ctx: &mut Ctx) -> LaidCell<'a> {
    let w = cell_span_width(cols, p.col, p.colspan);
    let bs = p.lb.resolved(ResolveCtx {
        parent_font_size: fs,
        cb_width: w,
    });
    let frag = block::layout_box_sized_isolated(p.lb, &bs, 0.0, 0.0, w, ctx);
    let content_h = frag.height;
    LaidCell {
        p,
        frag,
        valign: bs.vertical_align,
        content_h,
    }
}

fn layout_cells<'a>(
    placed: &'a [Placed],
    cols: &[f32],
    fs: f32,
    ctx: &mut Ctx,
) -> Vec<LaidCell<'a>> {
    placed
        .iter()
        .map(|p| layout_one(p, cols, fs, ctx))
        .collect()
}

fn expand_rows(h: &mut [f32], c: &LaidCell) {
    let (start, span) = (c.p.row, c.p.rowspan);
    let have: f32 = h[start..start + span].iter().sum();
    if c.content_h > have {
        h[start + span - 1] += c.content_h - have;
    }
}

fn row_heights(laid: &[LaidCell], rows: &[RowRef]) -> Vec<f32> {
    // Seed each row at its explicit `height` floor (empty spacer rows), then grow
    // to fit content.
    let mut h: Vec<f32> = rows.iter().map(|r| r.min_height).collect();
    for c in laid {
        if c.p.rowspan == 1 {
            h[c.p.row] = h[c.p.row].max(c.content_h);
        }
    }
    for c in laid {
        if c.p.rowspan > 1 {
            expand_rows(&mut h, c);
        }
    }
    h
}

struct Geom {
    col_x: Vec<f32>,
    row_y: Vec<f32>,
    row_h: Vec<f32>,
    table_w: f32,
}

fn valign_offset(v: VAlign, cell_h: f32, content_h: f32) -> f32 {
    let slack = (cell_h - content_h).max(0.0);
    match v {
        VAlign::Middle | VAlign::Baseline => slack / 2.0,
        VAlign::Bottom => slack,
        _ => 0.0,
    }
}

fn spanned(row_h: &[f32], start: usize, span: usize) -> f32 {
    row_h[start..start + span].iter().sum()
}

fn position_cell(c: LaidCell, geom: &Geom, cx: f32, cy: f32) -> Fragment {
    let cell_h = spanned(&geom.row_h, c.p.row, c.p.rowspan);
    let dy = valign_offset(c.valign, cell_h, c.content_h);
    let mut frag = c.frag;
    frag.translate(cx + geom.col_x[c.p.col], cy + geom.row_y[c.p.row]);
    for child in &mut frag.children {
        child.translate(0.0, dy);
    }
    frag.height = cell_h;
    frag
}

fn make_row_frag(row: &RowRef, r: usize, geom: &Geom, cx: f32, cy: f32) -> Fragment {
    let content = FragmentContent::Box {
        background: None,
        border: BorderEdges::default(),
        border_radius: 0.0,
        shadow: None,
        gradient: None,
    };
    let mut f = Fragment::new(
        row.node_id,
        cx,
        cy + geom.row_y[r],
        geom.table_w,
        geom.row_h[r],
        content,
    );
    f.break_meta.repeatable = row.repeat;
    #[cfg(feature = "pdf-ua")]
    {
        f.role = row.ua_role;
    }
    f
}

fn finalize(rows: &[RowRef], laid: Vec<LaidCell>, geom: &Geom, cx: f32, cy: f32) -> Vec<Fragment> {
    let mut row_frags: Vec<Fragment> = rows
        .iter()
        .enumerate()
        .map(|(r, row)| make_row_frag(row, r, geom, cx, cy))
        .collect();
    for c in laid {
        let r = c.p.row;
        row_frags[r].children.push(position_cell(c, geom, cx, cy));
    }
    row_frags
}

/// Lay out a table's rows into the content box at `(cx, cy)` of width `cw`.
/// The table's min-content width: the sum of its columns' natural widths (each the
/// widest cell in the column, colspans distributed). A table never shrinks below
/// this — an explicit `width` narrower than the content is overridden (CSS auto
/// table layout), so a fixed-width infobox still grows to fit a wide image row
/// instead of overflowing it.
/// `(horizontal, vertical)` `border-spacing` in px — the gap around/between cells
/// in the default `separate` model (`2px` per CSS UA); `border-collapse:collapse`
/// removes it. Without it a table's cells butt together (the infobox label ran
/// straight into its value: "SpeciesF. catus").
fn border_spacing(style: &ComputedStyle) -> (f32, f32) {
    if style.get("border-collapse").map(str::trim) == Some("collapse") {
        return (0.0, 0.0);
    }
    let mut it = style
        .get("border-spacing")
        .map(str::split_whitespace)
        .into_iter()
        .flatten();
    let h = it.next().and_then(|t| parse_px(t, DEFAULT_FONT_SIZE));
    let v = it.next().and_then(|t| parse_px(t, DEFAULT_FONT_SIZE));
    (h.unwrap_or(2.0), v.or(h).unwrap_or(2.0))
}

/// Prefix offsets for `n` tracks separated (and bracketed) by `gap`: track `i`
/// starts at `gap*(i+1) + sum(tracks[..i])`.
fn spaced_offsets(tracks: &[f32], gap: f32) -> Vec<f32> {
    let mut out = Vec::with_capacity(tracks.len());
    let mut acc = gap;
    for t in tracks {
        out.push(acc);
        acc += t + gap;
    }
    out
}

pub(crate) fn min_content_width(
    items: &[LayoutBox],
    style: &ComputedStyle,
    fonts: &FontRegistry,
) -> f32 {
    let rows = collect_rows(items);
    let (placed, ncols) = build_grid(&rows);
    if ncols == 0 {
        return 0.0;
    }
    let (hs, _) = border_spacing(style);
    // Each column's min-content is the widest single-column cell (text wraps to its
    // longest word; a fixed-width cell keeps its size). The table is at least the sum
    // of those, and at least any spanning cell's own min-content (e.g. the infobox's
    // 267px cat-image row that spans both columns).
    let mut w = vec![0.0_f32; ncols];
    for p in &placed {
        if p.colspan == 1 {
            w[p.col] = w[p.col].max(super::flex::min_content_width(p.lb, fonts));
        }
    }
    let base: f32 = w.iter().sum();
    let span_max = placed
        .iter()
        .filter(|p| p.colspan > 1)
        .map(|p| super::flex::min_content_width(p.lb, fonts))
        .fold(0.0_f32, f32::max);
    base.max(span_max) + hs * (ncols as f32 + 1.0)
}

/// Returns the row fragments (galley-absolute) and the table content height.
pub(crate) fn layout_table(
    table: &LayoutBox,
    items: &[LayoutBox],
    cx: f32,
    cy: f32,
    cw: f32,
    fs: f32,
    ctx: &mut Ctx,
) -> (Vec<Fragment>, f32) {
    let rows = collect_rows(items);
    let (placed, ncols) = build_grid(&rows);
    if ncols == 0 {
        return (Vec::new(), 0.0);
    }
    let (hs, vs) = border_spacing(&table.style);
    // Reserve the horizontal spacing (gaps + edges) out of the content width so the
    // columns + gaps still fit the table box.
    let inner = (cw - hs * (ncols as f32 + 1.0)).max(1.0);
    let cols = column_widths(&table.style, &placed, ncols, inner, ctx.fonts);
    let laid = layout_cells(&placed, &cols, fs, ctx);
    let row_h = row_heights(&laid, &rows);
    let nrows = row_h.len();
    let geom = Geom {
        col_x: spaced_offsets(&cols, hs),
        row_y: spaced_offsets(&row_h, vs),
        table_w: cols.iter().sum::<f32>() + hs * (ncols as f32 + 1.0),
        row_h,
    };
    let height = geom.row_h.iter().sum::<f32>() + vs * (nrows as f32 + 1.0);
    (finalize(&rows, laid, &geom, cx, cy), height)
}
