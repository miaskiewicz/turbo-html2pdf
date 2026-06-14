//! Page orchestration (§3.0, §6.5–6.8): the higher layer that drives the body
//! through render → style → layout → `paginate`, then renders the running
//! header/footer regions per page with late-evaluated page-number context and
//! paints them into each page's `header`/`footer` band.
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
use crate::layout::fragment::Fragment;
use crate::layout::layout;
use crate::paginate::{paginate_with_geometry, resolve_geometry, Page, PageGeometry};
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
    let body = lay_out_body(inputs, diags)?;
    let base = resolve_geometry(inputs.at_rules, PageGeometry::a4())?;
    let geometry = reserve_bands(inputs, base, diags)?;
    let mut pages = paginate_with_geometry(&body, geometry, diags);
    fill_regions(inputs, &mut pages, diags)?;
    Ok(pages)
}

/// Render → style → lay out the body flow into one continuous galley, exactly as
/// the canonical full-pipeline wiring does.
fn lay_out_body<T: Serialize>(
    inputs: &RenderInputs<T>,
    diags: &mut Diagnostics,
) -> Result<Fragment, RenderError> {
    let (nodes, rdiags) = inputs.program.render_nodes(inputs.data, inputs.now)?;
    diags.lints.extend(rdiags.lints);
    let styled = style_tree(&nodes, inputs.cascade);
    let width = resolve_geometry(inputs.at_rules, PageGeometry::a4())?.content_width();
    Ok(layout(&styled, width, inputs.fonts, diags))
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
