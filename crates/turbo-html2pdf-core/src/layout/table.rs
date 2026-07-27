//! Table layout (§5.4, AC-5.7–5.10). Builds a cell grid (with `colspan`/`rowspan`
//! occupancy), resolves column widths by the `fixed` or `auto` algorithm, lays
//! each cell out at its column-span width, sizes rows to their tallest cell, and
//! applies per-cell `vertical-align`. `<thead>`/`<tfoot>` rows are marked
//! repeatable (`BreakMeta.repeatable`) so the fragmenter can re-emit them on each
//! page a table spans (§6.3, AC-5.8).
//!
//! A fixed-layout `border-collapse: collapse` table merges shared cell edges onto a
//! collapsed grid (each shared border counted once, half on each side, CSS 2.1
//! §17.6.2) so table/row/cell boxes size correctly across a `colspan`; see the
//! collapsed-border section below. Deferred in v1 (documented): `<caption>`,
//! `border-spacing`, and collapsed-border merging for AUTO-layout tables (those
//! still lay out with each cell's full borders); a cell taller than its row span
//! expands the last spanned row.

use crate::node::Attr;
use crate::style::ComputedStyle;
use crate::text::FontRegistry;

use super::block::{self, Ctx};
use super::boxgen::{BoxKind, LayoutBox};
use super::flex::natural_width;
use super::fragment::{Fragment, FragmentContent, NodeId, RepeatKind};
use super::value::{
    parse_px, resolve_box_style, BorderEdges, Display, Edges, LengthPct, ResolveCtx, VAlign,
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

fn collapsed(style: &ComputedStyle) -> bool {
    style.get("border-collapse").map(str::trim) == Some("collapse")
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
        grow_columns(cols, target / sum);
    } else {
        shrink_columns(cols, mins, sum - target);
    }
}

/// Scale every column up by factor `k` (grow-to-fit distributes slack evenly).
fn grow_columns(cols: &mut [f32], k: f32) {
    for c in cols.iter_mut() {
        *c *= k;
    }
}

/// Remove `reduce` px total, taking only from each column's slack above its
/// min-content so no column clips its content; a column never drops below `mins`.
fn shrink_columns(cols: &mut [f32], mins: &[f32], reduce: f32) {
    let slack: f32 = cols.iter().zip(mins).map(|(c, m)| (c - m).max(0.0)).sum();
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
// collapsed-border grid (`border-collapse: collapse`)
// --------------------------------------------------------------------------
//
// In the collapse model a shared edge between two cells is a SINGLE border, drawn
// at the max width of the abutting borders and centred on the grid line, with half
// on each side (CSS 2.1 §17.6.2). So a cell's border box runs from the centre of
// its left grid line to the centre of its right one, and the table's width is the
// column content+padding tracks plus one border per grid line — NOT each cell's
// full borders summed (which double-counts every internal edge and overshoots,
// especially under a `colspan`). We model it explicitly: per grid-line border
// widths, content+padding column tracks, and grid-line centre positions.

/// A cell's resolved border widths (px). Cheap wrapper so callers read
/// `.left`/`.right`/`.top`/`.bottom` without re-plumbing a `ResolveCtx`.
fn cell_border(lb: &LayoutBox) -> Edges {
    lb.resolved(ResolveCtx {
        parent_font_size: DEFAULT_FONT_SIZE,
        cb_width: 0.0,
    })
    .border
    .widths()
}

/// A cell's specified content+padding width (px) when it has a `width: <px>` — the
/// column's preferred track width in the collapse model (borders live on the grid
/// lines, not inside the track). `None` for an auto width.
fn explicit_cp(lb: &LayoutBox) -> Option<f32> {
    let bs = lb.resolved(ResolveCtx {
        parent_font_size: DEFAULT_FONT_SIZE,
        cb_width: 0.0,
    });
    match bs.width {
        LengthPct::Px(w) => Some(w + bs.padding.horizontal()),
        _ => None,
    }
}

/// Collapsed border widths at the `ncols + 1` vertical grid lines. Line `j` is the
/// max of the table's left/right border (outer lines only) and every cell edging it
/// (its left border if it starts at `j`, its right if it ends at `j`). A `colspan`
/// cell contributes only at its two outer edges, so the line under it still takes
/// its width from cells in other rows that really border there.
fn vborders(placed: &[Placed], ncols: usize, style: &ComputedStyle) -> Vec<f32> {
    let table = resolve_box_style(
        style,
        ResolveCtx {
            parent_font_size: DEFAULT_FONT_SIZE,
            cb_width: 0.0,
        },
    )
    .border
    .widths();
    let mut v = vec![0.0_f32; ncols + 1];
    v[0] = table.left;
    v[ncols] = v[ncols].max(table.right);
    for p in placed {
        let b = cell_border(p.lb);
        v[p.col] = v[p.col].max(b.left);
        v[p.col + p.colspan] = v[p.col + p.colspan].max(b.right);
    }
    v
}

/// Collapsed border widths at the `nrows + 1` horizontal grid lines (the row-axis
/// analogue of [`vborders`], keyed on `rowspan` and top/bottom borders).
fn hborders(placed: &[Placed], nrows: usize, style: &ComputedStyle) -> Vec<f32> {
    let table = resolve_box_style(
        style,
        ResolveCtx {
            parent_font_size: DEFAULT_FONT_SIZE,
            cb_width: 0.0,
        },
    )
    .border
    .widths();
    let mut h = vec![0.0_f32; nrows + 1];
    h[0] = table.top;
    h[nrows] = h[nrows].max(table.bottom);
    for p in placed {
        let b = cell_border(p.lb);
        h[p.row] = h[p.row].max(b.top);
        h[p.row + p.rowspan] = h[p.row + p.rowspan].max(b.bottom);
    }
    h
}

/// Centre positions of the `n + 1` grid lines bounding `n` content+padding tracks,
/// where line `i` has collapsed border width `borders[i]`. Track `i` sits between
/// the full widths of lines `i` and `i+1`; a cell's border box runs centre-to-centre
/// so it owns half of each bounding border.
fn line_centers(tracks: &[f32], borders: &[f32]) -> Vec<f32> {
    let n = tracks.len();
    let mut out = Vec::with_capacity(n + 1);
    let mut pos = 0.0_f32;
    for i in 0..n {
        out.push(pos + borders[i] / 2.0);
        pos += borders[i] + tracks[i];
    }
    out.push(pos + borders[n] / 2.0);
    out
}

/// Per-column specified content+padding width from the FIRST row (fixed layout's
/// column authority), a `colspan` cell's width divided over the columns it covers.
/// `None` = auto (shares the leftover). Later rows never widen a column here — they
/// just flow into it (fixed layout, CSS 2.1 §17.5.2.1).
fn first_row_cp(placed: &[Placed], ncols: usize) -> Vec<Option<f32>> {
    let mut w = vec![None; ncols];
    for p in placed.iter().filter(|p| p.row == 0) {
        if let Some(total) = explicit_cp(p.lb) {
            let per = total / p.colspan as f32;
            for slot in w.iter_mut().skip(p.col).take(p.colspan) {
                slot.get_or_insert(per);
            }
        }
    }
    w
}

/// Content+padding widths per column for a fixed-layout collapsed table, given the
/// available content width (`table width − all grid-line borders`). Specified
/// columns keep their width and auto columns share the rest; when every column is
/// specified, the whole set scales proportionally to fill (or fit) `avail` — CSS
/// distributes the table's surplus/deficit over the columns.
fn fixed_collapse_columns(placed: &[Placed], ncols: usize, avail: f32) -> Vec<f32> {
    let spec = first_row_cp(placed, ncols);
    let specified: f32 = spec.iter().flatten().sum();
    let n_auto = spec.iter().filter(|x| x.is_none()).count();
    if n_auto > 0 {
        let each = share(avail - specified, n_auto);
        return spec.iter().map(|x| x.unwrap_or(each)).collect();
    }
    let k = if specified > 0.0 {
        avail / specified
    } else {
        0.0
    };
    spec.iter().map(|x| x.unwrap_or(0.0) * k).collect()
}

/// The fixed-collapse table's minimum content+padding per column: each first-row
/// cell's specified width, else its text min-content (colspan-distributed). Summed
/// with the grid-line borders this is the table's used-width floor — so a declared
/// width narrower than the columns grows (07), while one wider than them is honoured
/// (29's `colspan` first row asks for less than its `width:200px`).
fn fixed_min_cp(placed: &[Placed], ncols: usize, fonts: &FontRegistry) -> Vec<f32> {
    let mut w = vec![0.0_f32; ncols];
    let mut fixed = vec![false; ncols];
    for p in placed.iter().filter(|p| p.row == 0) {
        let explicit = explicit_cp(p.lb);
        let val = explicit.unwrap_or_else(|| {
            (super::flex::min_content_width(p.lb, fonts) - cell_border(p.lb).horizontal()).max(0.0)
        });
        let per = val / p.colspan as f32;
        for j in p.col..p.col + p.colspan {
            if explicit.is_some() && !fixed[j] {
                w[j] = per;
                fixed[j] = true;
            } else if !fixed[j] {
                w[j] = w[j].max(per);
            }
        }
    }
    w
}

/// Lay out the cells of a collapsed-border table into the grid defined by the
/// vertical grid-line centres `line_x` and their border widths `vert`. Each cell's
/// content is laid at the width its content region truly gets — the collapsed
/// border box minus the half-borders it shares, plus the cell's own full border
/// (which the block layout subtracts back out) — then its reported box width is
/// overwritten to the collapsed border box (centre-to-centre).
fn layout_cells_collapsed<'a>(
    placed: &'a [Placed],
    line_x: &[f32],
    vert: &[f32],
    fs: f32,
    ctx: &mut Ctx,
) -> Vec<LaidCell<'a>> {
    placed
        .iter()
        .map(|p| {
            let extent = line_x[p.col + p.colspan] - line_x[p.col];
            let half = vert[p.col] / 2.0 + vert[p.col + p.colspan] / 2.0;
            let bs = p.lb.resolved(ResolveCtx {
                parent_font_size: fs,
                cb_width: extent,
            });
            let own = bs.border.widths().horizontal();
            let layout_w = (extent - half + own).max(0.0);
            let mut frag = block::layout_box_sized_isolated(p.lb, &bs, 0.0, 0.0, layout_w, ctx);
            let content_h = frag.height;
            frag.width = extent;
            LaidCell {
                p,
                frag,
                valign: bs.vertical_align,
                content_h,
            }
        })
        .collect()
}

/// Content+padding height per row for a collapsed table: the tallest cell's own
/// content+padding (its laid border box minus its vertical borders), floored by the
/// row's explicit `height`. A `rowspan` cell that overflows its rows grows the last.
fn collapse_row_heights(laid: &[LaidCell], rows: &[RowRef]) -> Vec<f32> {
    let cp = |c: &LaidCell| (c.frag.height - cell_border(c.p.lb).vertical()).max(0.0);
    let mut h: Vec<f32> = rows.iter().map(|r| r.min_height).collect();
    for c in laid.iter().filter(|c| c.p.rowspan == 1) {
        h[c.p.row] = h[c.p.row].max(cp(c));
    }
    for c in laid.iter().filter(|c| c.p.rowspan > 1) {
        let (start, span) = (c.p.row, c.p.rowspan);
        let have: f32 = h[start..start + span].iter().sum();
        let need = cp(c);
        if need > have {
            h[start + span - 1] += need - have;
        }
    }
    h
}

/// A placed table grid: the collected rows and their cells positioned on the
/// `ncols`-wide occupancy grid (colspan/rowspan resolved). Bundled so the collapse
/// layout doesn't thread three coupled values through its signature.
struct Grid<'a> {
    rows: &'a [RowRef<'a>],
    placed: &'a [Placed<'a>],
    ncols: usize,
}

/// Lay out a fixed-layout `border-collapse: collapse` table (§17.6.2): merge shared
/// cell edges so the table/row/cell boxes are sized against the collapsed grid, not
/// each cell's full borders. Returns the row fragments and the table content height.
fn layout_table_collapsed(
    table: &LayoutBox,
    grid: &Grid,
    cx: f32,
    cy: f32,
    cw: f32,
    fs: f32,
    ctx: &mut Ctx,
) -> (Vec<Fragment>, f32) {
    let &Grid {
        rows,
        placed,
        ncols,
    } = grid;
    let nrows = rows.len();
    let vert = vborders(placed, ncols, &table.style);
    let vert_total: f32 = vert.iter().sum();
    let col_cp = fixed_collapse_columns(placed, ncols, (cw - vert_total).max(1.0));
    let line_x = line_centers(&col_cp, &vert);
    let laid = layout_cells_collapsed(placed, &line_x, &vert, fs, ctx);
    let horiz = hborders(placed, nrows, &table.style);
    let row_cp = collapse_row_heights(&laid, rows);
    let line_y = line_centers(&row_cp, &horiz);
    let geom = Geom {
        col_x: line_x[..ncols].to_vec(),
        row_y: line_y[..nrows].to_vec(),
        row_h: line_y.windows(2).map(|w| w[1] - w[0]).collect(),
        table_w: col_cp.iter().sum::<f32>() + vert_total,
    };
    let height = row_cp.iter().sum::<f32>() + horiz.iter().sum::<f32>();
    (finalize(rows, laid, &geom, cx, cy), height)
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
        transform: None,
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
    if collapsed(style) {
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
    // Fixed collapsed table: floor = first-row columns (colspan-distributed) plus one
    // border per grid line — the used-width basis, not each cell's full borders.
    if is_fixed(style) && collapsed(style) {
        let vert: f32 = vborders(&placed, ncols, style).iter().sum();
        return fixed_min_cp(&placed, ncols, fonts).iter().sum::<f32>() + vert;
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
    // Fixed-layout collapsed borders need the merged-grid geometry (shared edges
    // counted once). Auto/separate tables keep the track+spacing model below.
    if is_fixed(&table.style) && collapsed(&table.style) {
        let grid = Grid {
            rows: &rows,
            placed: &placed,
            ncols,
        };
        return layout_table_collapsed(table, &grid, cx, cy, cw, fs, ctx);
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
