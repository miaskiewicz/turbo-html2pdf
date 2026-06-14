//! Page orchestration (§3.0, §3.6, §6.4–6.8): the higher layer that drives the
//! body through render → style → layout → `paginate`, resolves footnote content
//! into per-page bands via the body/footnote fixpoint, then renders the running
//! header/footer regions per page with late-evaluated page-number context and
//! paints them into each page's `header`/`footer` band.
//!
//! ## Footnotes (§3.6, §6.4)
//!
//! Footnotes are content-driven: a `<t:footnote>` leaves an inline marker in the
//! body and its note body is flowed *here* (the orchestrator is the one layer
//! that has the node tree, the cascade, and pagination together). We collect the
//! note sources in document order, lay each body out into its own small galley
//! with its mark, tag the body galley's markers with their note index, and hand
//! the notes to `paginate_with_footnotes`, which runs the fixpoint (§6.4) so each
//! page reserves the area of the notes it actually references. `page`-reset
//! numbering, which depends on which page a note lands on, is resolved in a second
//! relabel pass once placement is known.
//!
//! Layering: `crate::paginate` stays free of template/style deps — it only walks
//! a laid-out galley against geometry. This module is the one place that knows
//! about all of `Program`, `Cascade`, and `paginate` at once, so the late
//! evaluation that needs every layer lives here and nowhere lower.
//!
//! ## Band sizing (the chicken-and-egg)
//!
//! A region's height reduces body capacity, but the region itself is rendered
//! against a page count that the body's pagination produces. We resolve it in
//! one pass, no fixpoint:
//!
//! 1. Render + lay out each region once against a *representative* page-1
//!    context (`number = 1`, `total = 1`) and measure its laid-out height.
//! 2. Reserve that measured height as the band extent (capped at the margin so a
//!    region can never eat past the page edge), which lowers body capacity.
//! 3. Paginate the body against the reduced capacity to get the real page count.
//! 4. Re-render each region per page with the true `{number, total, is_first,
//!    is_last}` and paint it into the reserved band.
//!
//! The one-pass approximation: the measured extent uses the page-1 context, so a
//! region whose *height* changes with the page number (rare — e.g. a footer that
//! wraps to two lines only on the last page) reserves the page-1 height for every
//! page. Per-page content taller than the reserved band is clipped + linted
//! (AC-6.8), so the body is never overlapped. A full fixpoint over band height is
//! TODO(phase7b) alongside masters.
//!
//! TODO(phase7b): page masters, `t:counter`, leaders, mirrored-margin duplex —
//! this slice handles only the master-less running header/footer on the default
//! geometry, with `page.number`/`page.total` late evaluation.

use serde::Serialize;

use crate::error::{Diagnostics, LintCode, RenderError, Span};
use crate::layout::fragment::{Fragment, FragmentContent};
use crate::layout::layout;
use crate::node::{Element, Node, TKind, Tag};
use crate::paginate::{paginate_with_footnotes, resolve_geometry, Note, Page, PageGeometry};
use crate::style::{style_tree, AtRule, Cascade};
use crate::template::{Program, FOOTER, HEADER};
use crate::text::FontRegistry;

/// The per-page context exposed to a running header/footer region (§3.3 subset).
/// Serialized under the `page` key so a region writes `{{ page.number }}` etc.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PageContext {
    /// 1-based page number — the value of `{{ page.number }}` / `<t:page/>`.
    pub number: u32,
    /// Total page count — the value of `{{ page.total }}` / `<t:pages/>`.
    pub total: u32,
    /// True on the first page.
    pub is_first: bool,
    /// True on the last page.
    pub is_last: bool,
}

impl PageContext {
    /// Build the context for page `number` of `total`.
    fn new(number: u32, total: u32) -> PageContext {
        PageContext {
            number,
            total,
            is_first: number == 1,
            is_last: number == total,
        }
    }
}

/// The full render context handed to a region template: the per-page `page`
/// state plus the caller's original `data` (so a footer can interpolate both
/// `{{ page.number }}` and document fields).
#[derive(Serialize)]
struct RegionCtx<'a, T: Serialize> {
    page: PageContext,
    data: &'a T,
}

/// Inputs the orchestrator needs that the body pipeline doesn't already carry.
pub struct RenderInputs<'a, T: Serialize> {
    pub program: &'a Program,
    pub data: &'a T,
    pub cascade: &'a Cascade,
    pub at_rules: &'a [AtRule],
    pub fonts: &'a FontRegistry,
    pub now: Option<i64>,
}

/// Drive a compiled [`Program`] all the way to paginated [`Page`]s with running
/// header/footer regions filled and page-number field codes late-evaluated.
///
/// This is the Phase 7 public entry point: a caller compiles a template, builds
/// a cascade, and hands both here to get the page list the PDF emitter consumes.
pub fn render_pages<T: Serialize>(
    inputs: &RenderInputs<T>,
    diags: &mut Diagnostics,
) -> Result<Vec<Page>, RenderError> {
    let (mut body, sources, reset) = lay_out_body(inputs, diags)?;
    let base = resolve_geometry(inputs.at_rules, PageGeometry::a4())?;
    let geometry = reserve_bands(inputs, base, diags)?;
    let notes = lay_out_notes(inputs, &sources, &geometry, diags);
    tag_markers(&mut body, notes.len());
    let mut pages = paginate_with_footnotes(&body, geometry, &notes, diags);
    relabel_page_reset(inputs, &sources, reset, &geometry, &body, &mut pages, diags);
    fill_regions(inputs, &mut pages, diags)?;
    Ok(pages)
}

/// Render → style → lay out the body flow into one continuous galley, returning
/// the galley plus the footnote sources collected from the rendered node tree in
/// document order (their bodies are flowed separately into the footnote area).
fn lay_out_body<T: Serialize>(
    inputs: &RenderInputs<T>,
    diags: &mut Diagnostics,
) -> Result<(Fragment, Vec<FootnoteSrc>, ResetMode), RenderError> {
    let (nodes, rdiags) = inputs.program.render_nodes(inputs.data, inputs.now)?;
    diags.lints.extend(rdiags.lints);
    let sources = collect_footnotes(&nodes);
    let reset = reset_mode(&nodes);
    let styled = style_tree(&nodes, inputs.cascade);
    let width = resolve_geometry(inputs.at_rules, PageGeometry::a4())?.content_width();
    Ok((layout(&styled, width, inputs.fonts, diags), sources, reset))
}

// --------------------------------------------------------------------------
// footnotes (§3.6, §6.4)
// --------------------------------------------------------------------------

/// How footnote numbering resets across the document (§3.6, AC-3.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResetMode {
    /// Document-continuous (the default, and `none`).
    Continuous,
    /// Restart at 1 on each page.
    Page,
}

/// One footnote as found in the rendered node tree, before its body is flowed
/// into the footnote area.
struct FootnoteSrc {
    /// The note body's child nodes (markup + text).
    children: Vec<Node>,
    /// A manual mark (`<t:footnote mark="*">`), independent of auto-numbering.
    manual: Option<String>,
}

/// Collect every `<t:footnote>` in `nodes` in document (pre-order) order, the
/// same order the layout assigns the inline marker fragments.
fn collect_footnotes(nodes: &[Node]) -> Vec<FootnoteSrc> {
    let mut out = Vec::new();
    for node in nodes {
        collect_footnotes_node(node, &mut out);
    }
    out
}

fn collect_footnotes_node(node: &Node, out: &mut Vec<FootnoteSrc>) {
    let Some(el) = node.as_element() else {
        return;
    };
    if matches!(el.tag, Tag::Directive(TKind::Footnote)) {
        out.push(FootnoteSrc {
            children: el.children.clone(),
            manual: el.attr("mark").map(str::to_string),
        });
        return;
    }
    for child in &el.children {
        collect_footnotes_node(child, out);
    }
}

/// The document's footnote-reset policy: the first `footnote-reset` attribute
/// declared on any footnote wins; absent or `none`/`section` is continuous.
/// TODO(phase15): `section` reset awaits section anchors; treated as continuous.
fn reset_mode(nodes: &[Node]) -> ResetMode {
    find_reset(nodes)
        .filter(|v| v == "page")
        .map_or(ResetMode::Continuous, |_| ResetMode::Page)
}

fn find_reset(nodes: &[Node]) -> Option<String> {
    nodes.iter().find_map(find_reset_node)
}

fn find_reset_node(node: &Node) -> Option<String> {
    let el = node.as_element()?;
    if matches!(el.tag, Tag::Directive(TKind::Footnote)) {
        if let Some(v) = el.attr("footnote-reset") {
            return Some(v.to_string());
        }
    }
    el.children.iter().find_map(find_reset_node)
}

/// Lay out each footnote body into its own galley with a continuous mark prefix
/// (the placement-driving pass; page-reset relabeling happens afterward).
fn lay_out_notes<T: Serialize>(
    inputs: &RenderInputs<T>,
    sources: &[FootnoteSrc],
    geometry: &PageGeometry,
    diags: &mut Diagnostics,
) -> Vec<Note> {
    let marks = continuous_marks(sources);
    flow_notes(inputs, sources, &marks, geometry, diags)
}

/// Document-continuous marks: manual marks pass through; auto notes take the next
/// integer in their own sequence (AC-3.20, independent sequences).
fn continuous_marks(sources: &[FootnoteSrc]) -> Vec<String> {
    let mut auto = 0u32;
    sources
        .iter()
        .map(|s| match &s.manual {
            Some(m) => m.clone(),
            None => {
                auto += 1;
                auto.to_string()
            }
        })
        .collect()
}

/// Flow each note body (mark prefix + children) through style + layout into a
/// galley fragment, returning the resolved [`Note`] list.
fn flow_notes<T: Serialize>(
    inputs: &RenderInputs<T>,
    sources: &[FootnoteSrc],
    marks: &[String],
    geometry: &PageGeometry,
    diags: &mut Diagnostics,
) -> Vec<Note> {
    let width = geometry.content_width();
    let mut out = Vec::with_capacity(sources.len());
    for (i, src) in sources.iter().enumerate() {
        let nodes = note_nodes(&marks[i], &src.children);
        let styled = style_tree(&nodes, inputs.cascade);
        let galley = layout(&styled, width, inputs.fonts, diags);
        out.push(Note::new(i, galley));
    }
    out
}

/// Wrap a note's mark and body into a `<p class="footnote">` node so it flows as
/// an ordinary small paragraph through the existing layout path.
fn note_nodes(mark: &str, body: &[Node]) -> Vec<Node> {
    let mut children = vec![Node::Text(format!("{mark} "))];
    children.extend(body.iter().cloned());
    vec![Node::Element(Element {
        tag: Tag::Html("p".to_string()),
        attrs: vec![crate::node::Attr {
            name: "class".to_string(),
            value: "footnote".to_string(),
        }],
        children,
    })]
}

/// Tag the body galley's footnote markers, in document order, with their note
/// index so the fragmenter knows which notes a page references (§6.4).
fn tag_markers(body: &mut Fragment, count: usize) {
    let mut next = 0usize;
    tag_markers_in(body, count, &mut next);
}

fn tag_markers_in(frag: &mut Fragment, count: usize, next: &mut usize) {
    if matches!(frag.content, FragmentContent::Directive(TKind::Footnote)) && *next < count {
        frag.break_meta.footnotes.push(*next);
        *next += 1;
    }
    for child in &mut frag.children {
        tag_markers_in(child, count, next);
    }
}

/// Under `page` reset, renumber each note from its position within the page it
/// landed on (AC-3.16), re-flow the note galleys with the new marks, and
/// re-paginate once so the relabeled bands are placed. A no-op for continuous.
fn relabel_page_reset<T: Serialize>(
    inputs: &RenderInputs<T>,
    sources: &[FootnoteSrc],
    reset: ResetMode,
    geometry: &PageGeometry,
    body: &Fragment,
    pages: &mut Vec<Page>,
    diags: &mut Diagnostics,
) {
    if reset != ResetMode::Page {
        return;
    }
    let marks = page_reset_marks(sources, pages);
    let notes = flow_notes(inputs, sources, &marks, geometry, diags);
    *pages = paginate_with_footnotes(body, *geometry, &notes, diags);
}

/// Per-page marks: each page restarts auto-numbering at 1 in note-index order;
/// manual marks still pass through unchanged.
fn page_reset_marks(sources: &[FootnoteSrc], pages: &[Page]) -> Vec<String> {
    let mut marks = vec![String::new(); sources.len()];
    for page in pages {
        number_page(sources, page, &mut marks);
    }
    marks
}

/// Number the notes that landed on one page, restarting auto-numbering at 1.
fn number_page(sources: &[FootnoteSrc], page: &Page, marks: &mut [String]) {
    let mut auto = 0u32;
    for idx in page_note_indices(page) {
        marks[idx] = match &sources[idx].manual {
            Some(m) => m.clone(),
            None => {
                auto += 1;
                auto.to_string()
            }
        };
    }
}

/// The note indices a page references, in document order, read off the marker
/// fragments' tags.
fn page_note_indices(page: &Page) -> Vec<usize> {
    let mut out = Vec::new();
    for frag in &page.body {
        collect_marker_indices(frag, &mut out);
    }
    out
}

fn collect_marker_indices(frag: &Fragment, out: &mut Vec<usize>) {
    out.extend(&frag.break_meta.footnotes);
    for child in &frag.children {
        collect_marker_indices(child, out);
    }
}

/// Measure each present region once (page-1 context) and reserve its height as
/// the corresponding band extent, capped at the available margin so a region can
/// never push past the page edge (AC-3.0.3).
fn reserve_bands<T: Serialize>(
    inputs: &RenderInputs<T>,
    base: PageGeometry,
    diags: &mut Diagnostics,
) -> Result<PageGeometry, RenderError> {
    let mut geo = base;
    let probe = PageContext::new(1, 1);
    if let Some(galley) = render_region(inputs, HEADER, probe, diags)? {
        geo.header_extent = band_extent(&galley, base.margin.top);
    }
    if let Some(galley) = render_region(inputs, FOOTER, probe, diags)? {
        geo.footer_extent = band_extent(&galley, base.margin.bottom);
    }
    Ok(geo)
}

/// The reserved band height: the region's laid-out height, never more than the
/// margin it sits in.
fn band_extent(galley: &Fragment, margin: f32) -> f32 {
    galley.height.min(margin)
}

/// Render + style + lay out one region against `ctx`, returning its galley, or
/// `None` if that region was not declared.
fn render_region<T: Serialize>(
    inputs: &RenderInputs<T>,
    name: &str,
    ctx: PageContext,
    diags: &mut Diagnostics,
) -> Result<Option<Fragment>, RenderError> {
    let region_ctx = RegionCtx {
        page: ctx,
        data: inputs.data,
    };
    let Some(result) = inputs.program.render_region(name, &region_ctx, inputs.now) else {
        return Ok(None);
    };
    let (nodes, rdiags) = result?;
    diags.lints.extend(rdiags.lints);
    let styled = style_tree(&nodes, inputs.cascade);
    let width = resolve_geometry(inputs.at_rules, PageGeometry::a4())?.content_width();
    Ok(Some(layout(&styled, width, inputs.fonts, diags)))
}

/// Re-render every page's regions with that page's real `{number, total}` and
/// paint them into the reserved bands, clipping + linting any overflow.
fn fill_regions<T: Serialize>(
    inputs: &RenderInputs<T>,
    pages: &mut [Page],
    diags: &mut Diagnostics,
) -> Result<(), RenderError> {
    let total = pages.len() as u32;
    for page in pages.iter_mut() {
        let ctx = PageContext::new(page.number, total);
        place_band(inputs, page, ctx, HEADER, diags)?;
        place_band(inputs, page, ctx, FOOTER, diags)?;
    }
    Ok(())
}

/// Render one region for `page`, translate it into its band, and store it.
fn place_band<T: Serialize>(
    inputs: &RenderInputs<T>,
    page: &mut Page,
    ctx: PageContext,
    name: &str,
    diags: &mut Diagnostics,
) -> Result<(), RenderError> {
    let Some(mut galley) = render_region(inputs, name, ctx, diags)? else {
        return Ok(());
    };
    let (extent, dy) = band_placement(name, &page.geometry);
    clip_region(&mut galley, extent, diags);
    galley.translate(page.geometry.margin.left, dy);
    let frags = std::mem::take(&mut galley.children);
    store_band(page, name, frags);
    Ok(())
}

/// The band's reserved extent and the `y` its top sits at: the header rides at
/// the top margin, the footer just above the bottom margin.
fn band_placement(name: &str, geo: &PageGeometry) -> (f32, f32) {
    if name == HEADER {
        (geo.header_extent, geo.margin.top)
    } else {
        let top = geo.height - geo.margin.bottom - geo.footer_extent;
        (geo.footer_extent, top)
    }
}

/// Clip + lint a region taller than its band (AC-6.8): drop any laid-out line
/// whose top already sits past the reserved extent so the region never overlaps
/// the body, and flag the overflow. The band is region-local (`y = 0` at its
/// top), so a fragment is out of bounds once `y >= extent`.
fn clip_region(galley: &mut Fragment, extent: f32, diags: &mut Diagnostics) {
    let before = galley.children.len();
    galley.children.retain(|c| c.y < extent);
    if galley.height > extent + 0.5 || galley.children.len() < before {
        diags.push(
            LintCode::RegionOverflow,
            "running region content taller than its band was clipped",
            Span::default(),
        );
    }
}

/// Store the laid-out band fragments in the page's header or footer slot.
fn store_band(page: &mut Page, name: &str, frags: Vec<Fragment>) {
    if name == HEADER {
        page.header = frags;
    } else {
        page.footer = frags;
    }
}
