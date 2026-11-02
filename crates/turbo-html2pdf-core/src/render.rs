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
//! that has the document, the cascade, and pagination together). We collect the
//! note sources in document order, move each body into a synthetic
//! `<p class="footnote">` (built in the document) after its mark, lay it out into
//! its own small galley, tag the body galley's markers with their note index, and
//! hand the notes to `paginate_with_footnotes`, which runs the fixpoint (§6.4) so
//! each page reserves the area of the notes it actually references. `page`-reset
//! numbering, which depends on which page a note lands on, is resolved in a second
//! relabel pass once placement is known.
//!
//! ## Node ids and semantics
//!
//! Every flow (body, notes, regions) is laid out with its own disjoint
//! [`NodeId`](crate::NodeId) range; with the `pdf-ua`/`xref` features each layout
//! also records its box sources and the derived [`Semantics`] (structure roles,
//! links, anchors) ride on every returned [`Page`].
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
use turbo_dom::rtdom::tree::NodeType;
use turbo_dom::{Handle, Tree};
use turbo_dom_layout::TreeView;

use crate::error::{Diagnostics, LintCode, RenderError, Span};
use crate::flow::Flow;
use crate::image::ImageResolver;
use crate::layout::fragment::{Fragment, NodeId};
use crate::layout::ImageCtx;
use crate::markers::{directive_of, TKind};
use crate::paginate::{
    paginate_document, resolve_geometry, FlowBoxes, Galley, Note, Page, PageGeometry,
};
use crate::style::{AtRule, Cascade};
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
    /// Caller-supplied image source (§7.4). Defaults to a no-op resolver via
    /// [`crate::NoImages`]; supply one to embed `<img>`/`background-image`.
    pub images: &'a dyn ImageResolver,
    pub now: Option<i64>,
}

/// Per-render state: the diagnostics sink, the next free box id, and (with the
/// `pdf-ua`/`xref` features) the semantics recorded from every flow laid out.
struct Run<'d> {
    diags: &'d mut Diagnostics,
    #[cfg(any(feature = "pdf-ua", feature = "xref"))]
    next_id: u32,
    #[cfg(any(feature = "pdf-ua", feature = "xref"))]
    derived: crate::semantics::derive::Derived,
}

/// No image resolver, no page height: the region/footnote layout.
const NO_IMAGES: ImageCtx<'static> = ImageCtx {
    resolver: &crate::NoImages,
    body_height: None,
};

impl<'d> Run<'d> {
    fn new(diags: &'d mut Diagnostics) -> Run<'d> {
        Run {
            diags,
            // Id 0 stays free for the fragmenter's synthetic fragments (the
            // footnote separator rule), so it never aliases a laid-out box.
            #[cfg(any(feature = "pdf-ua", feature = "xref"))]
            next_id: 1,
            #[cfg(any(feature = "pdf-ua", feature = "xref"))]
            derived: Default::default(),
        }
    }
}

// The two layout strategies live in gated-only sibling files (the workspace coverage
// build unifies the binding crates' features, so only one of them is compiled there).
#[cfg(any(feature = "pdf-ua", feature = "xref"))]
#[path = "render_semantics.rs"]
mod lay;
#[cfg(not(any(feature = "pdf-ua", feature = "xref")))]
#[path = "render_plain.rs"]
mod lay;

/// Drive a compiled [`Program`] all the way to paginated [`Page`]s with running
/// header/footer regions filled and page-number field codes late-evaluated.
///
/// This is the Phase 7 public entry point: a caller compiles a template, builds
/// a cascade, and hands both here to get the page list the PDF emitter consumes.
pub fn render_pages<T: Serialize>(
    inputs: &RenderInputs<T>,
    diags: &mut Diagnostics,
) -> Result<Vec<Page>, RenderError> {
    let mut run = Run::new(diags);
    let (mut body, mut flow, sources, reset, document) = lay_out_body(inputs, &mut run)?;
    // Everything after the body renders against the document's own sheets too (its
    // `@page` rules, its `<style>` rules in notes and regions) and its root font size.
    let inputs = &RenderInputs {
        cascade: &document.cascade,
        at_rules: &document.at_rules,
        ..*inputs
    };
    let _root_font = turbo_dom_layout::root_font_scope(document.root_font_px);
    let base = resolve_geometry(inputs.at_rules, PageGeometry::a4())?;
    let geometry = reserve_bands(inputs, base, &mut run)?;
    let bodies = note_bodies(&mut flow, &sources);
    let marks = continuous_marks(&sources);
    let notes = flow_notes(inputs, &mut flow, &bodies, &marks, &geometry, &mut run);
    tag_markers(&mut body, notes.len());
    let paged = PagedBody {
        root: &body,
        wrappers: &document.wrappers,
        boxes: &document.boxes,
    };
    let mut pages = paged.paginate(geometry, &notes, run.diags);
    let notes = NoteFlow {
        flow: &mut flow,
        bodies: &bodies,
        sources: &sources,
    };
    relabel_page_reset(
        inputs, notes, reset, &geometry, &paged, &mut pages, &mut run,
    );
    fill_regions(inputs, &mut pages, &mut run)?;
    run.finish(&mut pages);
    pages
        .iter_mut()
        .for_each(|page| page.canvas = document.canvas);
    Ok(pages)
}

/// What the body document contributes to the rest of the render
/// (miaskiewicz/turbo-html2pdf#29): the cascade and `@page` rules with the document's
/// own `<style>` sheets (before the caller's CSS), the root element's font size `rem`
/// resolves against, and the canvas colour propagated from `<html>`/`<body>`.
struct DocumentStyle {
    cascade: Cascade,
    at_rules: Vec<AtRule>,
    root_font_px: f32,
    canvas: Option<crate::layout::value::Rgba>,
    /// The dissolved `<html>`/`<body>` boxes (outermost first, children removed).
    wrappers: Vec<Fragment>,
    /// The body flow's used box values, for the fragmenter (miaskiewicz/turbo-html2pdf#32).
    boxes: FlowBoxes,
}

/// The body galley the fragmenter walks, with the `<html>`/`<body>` boxes dissolved
/// out of it (their paint is sliced onto every page).
struct PagedBody<'a> {
    root: &'a Fragment,
    wrappers: &'a [Fragment],
    boxes: &'a FlowBoxes,
}

impl PagedBody<'_> {
    fn paginate(
        &self,
        geometry: PageGeometry,
        notes: &[Note],
        diags: &mut Diagnostics,
    ) -> Vec<Page> {
        let galley = Galley {
            root: self.root,
            wrappers: self.wrappers,
            boxes: self.boxes,
        };
        paginate_document(galley, geometry, notes, diags)
    }
}

impl DocumentStyle {
    fn of<T: Serialize>(inputs: &RenderInputs<T>, flow: &Flow) -> DocumentStyle {
        let css = flow.document_css();
        // The geometry reads the first `@page` rule: the caller's, else the document's.
        let mut at_rules = inputs.at_rules.to_vec();
        at_rules.extend(crate::style::parse_stylesheet(&css).at_rules);
        DocumentStyle {
            cascade: inputs.cascade.for_document(&css),
            at_rules,
            root_font_px: 16.0,
            canvas: None,
            wrappers: Vec::new(),
            boxes: FlowBoxes::default(),
        }
    }
}

/// Dissolve the `<html>`/`<body>` boxes of a laid-out document into their content:
/// every fragment whose box came from one of `wrappers` is replaced by its children
/// (already in absolute galley coordinates), so the fragmenter walks the body content
/// as the page flow, exactly as it walks a detached flow. Their own paint is kept in
/// `boxes` (outermost first, children removed): the canvas colour is painted per page
/// and the fragmenter slices their backgrounds/borders onto every page
/// (miaskiewicz/turbo-html2pdf#31).
fn dissolve_wrappers(
    children: Vec<Fragment>,
    wrappers: &[NodeId],
    boxes: &mut Vec<Fragment>,
) -> Vec<Fragment> {
    let mut out = Vec::with_capacity(children.len());
    for mut frag in children {
        if wrappers.contains(&frag.node_id) {
            let children = std::mem::take(&mut frag.children);
            boxes.push(frag);
            out.extend(dissolve_wrappers(children, wrappers, boxes));
        } else {
            out.push(frag);
        }
    }
    out
}

/// Expand `<t:endnote>`/`<t:endnotes/>` when the `endnotes` feature is on; an
/// identity otherwise. Wrapping the gate in a function keeps the default body
/// free of any conditionally-compiled statement line.
#[cfg(not(feature = "endnotes"))]
fn expand_endnotes(_flow: &mut Flow) {}

/// Expand `<t:endnote>`/`<t:endnotes/>` into inline markers plus the rendered
/// note list (`endnotes` feature, §3).
#[cfg(feature = "endnotes")]
fn expand_endnotes(flow: &mut Flow) {
    crate::endnotes::expand(flow);
}

/// Render → style → lay out the body flow into one continuous galley, returning
/// the galley, the flow, plus the footnote sources found in it in document order
/// (their bodies are flowed separately into the footnote area).
fn lay_out_body<T: Serialize>(
    inputs: &RenderInputs<T>,
    run: &mut Run,
) -> Result<(Fragment, Flow, Vec<FootnoteSrc>, ResetMode, DocumentStyle), RenderError> {
    let (mut flow, rdiags) = inputs.program.render_nodes(inputs.data, inputs.now)?;
    run.diags.lints.extend(rdiags.lints);
    expand_endnotes(&mut flow);
    let sources = collect_footnotes(&flow);
    let reset = reset_mode(&flow);
    let mut document = DocumentStyle::of(inputs, &flow);
    let mut styled = flow.style_document(&document.cascade);
    document.canvas = turbo_dom_layout::html_layout::propagate_canvas_background(&mut styled);
    document.root_font_px = turbo_dom_layout::document_root_font_px(&styled);
    let _root_font = turbo_dom_layout::root_font_scope(document.root_font_px);
    // Size images against the base geometry's body height (the 60% cap basis).
    // Bands reserved later only shrink it slightly, so the base height is a
    // stable approximation and keeps body layout single-pass.
    let geo = resolve_geometry(&document.at_rules, PageGeometry::a4())?;
    let image_ctx = ImageCtx {
        resolver: inputs.images,
        body_height: Some(geo.body_height()),
    };
    // The page area is the print initial containing block: fixed boxes and absolute
    // boxes of the initial containing block resolve `%` / `bottom` insets and `%`
    // heights against its height (turbo-html2pdf#39).
    let laid = run.lay_recorded(
        &flow,
        &styled,
        (geo.content_width(), geo.body_height()),
        inputs.fonts,
        &image_ctx,
    );
    let mut galley = laid.galley;
    document.boxes =
        crate::flow_boxes::flow_boxes(&styled, &laid.sources, &galley, geo.content_width());
    let wrappers = wrapper_ids(&flow, &laid.sources);
    galley.children = dissolve_wrappers(
        std::mem::take(&mut galley.children),
        &wrappers,
        &mut document.wrappers,
    );
    Ok((galley, flow, sources, reset, document))
}

/// The box ids of the document's `<html>` and `<body>` elements.
fn wrapper_ids(flow: &Flow, sources: &turbo_dom_layout::NodeSources) -> Vec<NodeId> {
    let view = TreeView::new(flow.tree());
    let elements = [view.find_element("html"), view.find_element("body")];
    let mut ids: Vec<NodeId> = sources
        .elements
        .iter()
        .filter(|(_, h)| elements.contains(&Some(**h)))
        .map(|(&id, _)| id)
        .collect();
    ids.sort_by_key(|id| id.0);
    ids
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

/// One footnote as found in the body flow, before its body is flowed into the
/// footnote area.
pub(crate) struct FootnoteSrc {
    /// The `<t:footnote>` element.
    pub(crate) element: Handle,
    /// A manual mark (`<t:footnote mark="*">`), independent of auto-numbering.
    pub(crate) manual: Option<String>,
}

/// The content children (elements and text) of `h`.
fn kept_children(tree: &Tree, h: Handle) -> Vec<Handle> {
    TreeView::new(tree).children(h)
}

/// Whether `h` is a `<t:footnote>` element.
fn is_footnote(tree: &Tree, h: Handle) -> bool {
    tree.node_type(h) == NodeType::Element
        && tree.local_name(h).and_then(directive_of) == Some(TKind::Footnote)
}

/// Collect every `<t:footnote>` in the flow in document (pre-order) order, the
/// same order the layout assigns the inline marker fragments. A footnote's own
/// body is not searched (a nested note belongs to its note).
pub(crate) fn collect_footnotes(flow: &Flow) -> Vec<FootnoteSrc> {
    let mut out = Vec::new();
    collect_footnotes_in(flow.tree(), flow.roots(), &mut out);
    out
}

fn collect_footnotes_in(tree: &Tree, nodes: &[Handle], out: &mut Vec<FootnoteSrc>) {
    for &node in nodes {
        if is_footnote(tree, node) {
            out.push(FootnoteSrc {
                element: node,
                manual: tree.get_attribute_local(node, "mark").map(str::to_string),
            });
        } else {
            collect_footnotes_in(tree, &kept_children(tree, node), out);
        }
    }
}

/// The document's footnote-reset policy: the first `footnote-reset` attribute
/// declared on any footnote wins; absent or `none`/`section` is continuous.
/// TODO(phase15): `section` reset awaits section anchors; treated as continuous.
fn reset_mode(flow: &Flow) -> ResetMode {
    find_reset(flow.tree(), flow.roots())
        .filter(|v| v == "page")
        .map_or(ResetMode::Continuous, |_| ResetMode::Page)
}

fn find_reset(tree: &Tree, nodes: &[Handle]) -> Option<String> {
    nodes.iter().find_map(|&node| find_reset_node(tree, node))
}

fn find_reset_node(tree: &Tree, node: Handle) -> Option<String> {
    if is_footnote(tree, node) {
        if let Some(v) = tree.get_attribute_local(node, "footnote-reset") {
            return Some(v.to_string());
        }
    }
    find_reset(tree, &kept_children(tree, node))
}

/// One footnote's synthetic `<p class="footnote">` (its body moved after a mark
/// text node) in the body document.
pub(crate) struct NoteBody {
    pub(crate) paragraph: Handle,
    pub(crate) mark: Handle,
}

/// Build each note's `<p class="footnote">`: a mark text node followed by the
/// footnote's body, moved out of the (already laid out) `<t:footnote>` marker, so
/// it flows as an ordinary small paragraph through the existing layout path.
pub(crate) fn note_bodies(flow: &mut Flow, sources: &[FootnoteSrc]) -> Vec<NoteBody> {
    let tree = flow.tree_mut();
    sources
        .iter()
        .map(|src| {
            let paragraph = tree.create_element("p");
            tree.set_attribute(paragraph, "class", "footnote");
            let mark = tree.create_text_node("");
            tree.append_child(paragraph, mark);
            for child in kept_children(tree, src.element) {
                tree.append_child(paragraph, child);
            }
            NoteBody { paragraph, mark }
        })
        .collect()
}

/// Document-continuous marks: manual marks pass through; auto notes take the next
/// integer in their own sequence (AC-3.20, independent sequences).
pub(crate) fn continuous_marks(sources: &[FootnoteSrc]) -> Vec<String> {
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
    flow: &mut Flow,
    bodies: &[NoteBody],
    marks: &[String],
    geometry: &PageGeometry,
    run: &mut Run,
) -> Vec<Note> {
    let width = geometry.content_width();
    let mut out = Vec::with_capacity(bodies.len());
    for (i, body) in bodies.iter().enumerate() {
        flow.tree_mut()
            .set_node_value(body.mark, &format!("{} ", marks[i]));
        let styled = flow.style_roots(&[body.paragraph], inputs.cascade);
        let galley = run.lay(flow, &styled, width, inputs.fonts, &NO_IMAGES);
        out.push(Note::new(i, galley));
    }
    out
}

/// Tag the body galley's footnote markers, in document order, with their note
/// index so the fragmenter knows which notes a page references (§6.4).
fn tag_markers(body: &mut Fragment, count: usize) {
    let mut next = 0usize;
    tag_markers_in(body, count, &mut next);
}

fn tag_markers_in(frag: &mut Fragment, count: usize, next: &mut usize) {
    if TKind::of_fragment(&frag.content) == Some(TKind::Footnote) && *next < count {
        frag.break_meta.footnotes.push(*next);
        *next += 1;
    }
    for child in &mut frag.children {
        tag_markers_in(child, count, next);
    }
}

/// The body flow plus its notes, for the page-reset relabel pass.
struct NoteFlow<'a> {
    flow: &'a mut Flow,
    bodies: &'a [NoteBody],
    sources: &'a [FootnoteSrc],
}

/// Under `page` reset, renumber each note from its position within the page it
/// landed on (AC-3.16), re-flow the note galleys with the new marks, and
/// re-paginate once so the relabeled bands are placed. A no-op for continuous.
fn relabel_page_reset<T: Serialize>(
    inputs: &RenderInputs<T>,
    notes: NoteFlow,
    reset: ResetMode,
    geometry: &PageGeometry,
    body: &PagedBody,
    pages: &mut Vec<Page>,
    run: &mut Run,
) {
    if reset != ResetMode::Page {
        return;
    }
    let marks = page_reset_marks(notes.sources, pages);
    let flowed = flow_notes(inputs, notes.flow, notes.bodies, &marks, geometry, run);
    *pages = body.paginate(*geometry, &flowed, run.diags);
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
    run: &mut Run,
) -> Result<PageGeometry, RenderError> {
    let mut geo = base;
    let probe = PageContext::new(1, 1);
    if let Some(galley) = render_region(inputs, HEADER, probe, run)? {
        geo.header_extent = band_extent(&galley, base.margin.top);
    }
    if let Some(galley) = render_region(inputs, FOOTER, probe, run)? {
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
    run: &mut Run,
) -> Result<Option<Fragment>, RenderError> {
    let region_ctx = RegionCtx {
        page: ctx,
        data: inputs.data,
    };
    let Some(result) = inputs.program.render_region(name, &region_ctx, inputs.now) else {
        return Ok(None);
    };
    let (flow, rdiags) = result?;
    run.diags.lints.extend(rdiags.lints);
    let styled = flow.style(inputs.cascade);
    let width = resolve_geometry(inputs.at_rules, PageGeometry::a4())?.content_width();
    Ok(Some(run.lay(
        &flow,
        &styled,
        width,
        inputs.fonts,
        &NO_IMAGES,
    )))
}

/// Re-render every page's regions with that page's real `{number, total}` and
/// paint them into the reserved bands, clipping + linting any overflow.
fn fill_regions<T: Serialize>(
    inputs: &RenderInputs<T>,
    pages: &mut [Page],
    run: &mut Run,
) -> Result<(), RenderError> {
    let total = pages.len() as u32;
    for page in pages.iter_mut() {
        let ctx = PageContext::new(page.number, total);
        place_band(inputs, page, ctx, HEADER, run)?;
        place_band(inputs, page, ctx, FOOTER, run)?;
    }
    Ok(())
}

/// Render one region for `page`, translate it into its band, and store it.
fn place_band<T: Serialize>(
    inputs: &RenderInputs<T>,
    page: &mut Page,
    ctx: PageContext,
    name: &str,
    run: &mut Run,
) -> Result<(), RenderError> {
    let Some(mut galley) = render_region(inputs, name, ctx, run)? else {
        return Ok(());
    };
    let (extent, dy) = band_placement(name, &page.geometry);
    clip_region(&mut galley, extent, run.diags);
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
