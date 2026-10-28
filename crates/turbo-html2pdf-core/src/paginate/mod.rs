//! The fragmenter (§6, Stage 4): turns the continuous galley into a sequence of
//! fixed-size [`Page`]s. The number of pages is an *output* of walking the
//! content against a page's body capacity, never an input (AC-6.0): the same
//! template over more data simply yields more pages.
//!
//! This phase delivers the structural spine — geometry resolution and the break
//! walk — plus the footnote area: each page reserves the measured height of the
//! notes its body references, via the body/footnote fixpoint in [`footnotes`]
//! (§6.4). Running headers/footers and page-number late-evaluation
//! (`{{ page.number }}`, `<t:page/>`) are layered on by the `render` orchestrator,
//! which also resolves the footnote *content*; the `header`/`footer`/`footnotes`
//! page bands it paints into are exposed here. Page masters remain TODO(phase7b).
//!
//! [`paginate_document`] walks a whole document's [`Galley`]: with the used box values
//! of its boxes ([`FlowBoxes`]) it keeps margins, paddings and borders at breaks as
//! Chrome 156 prints them and places out-of-flow boxes (fixed boxes on every page,
//! absolute boxes from their containing block) — miaskiewicz/turbo-html2pdf#32.

mod boxes;
mod decorate;
mod footnotes;
mod geometry;
mod out_of_flow;
mod shifted;
mod walk;

use crate::error::{Diagnostics, RenderError};
use crate::layout::fragment::Fragment;
use crate::style::AtRule;

pub use boxes::{BoxFacts, FlowBoxes};
pub use footnotes::{FootnoteBand, Note};
pub use geometry::{resolve_geometry, PageGeometry};
pub use shifted::CARRIED;

/// Which master/variant a page resolves to (§3). In this phase the kind is
/// derived from the page number; `Blank` and explicit master variants arrive
/// with page masters in Phase 7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageKind {
    First,
    Left,
    Right,
    Blank,
}

/// One paginated page: its geometry plus the fragments painted in each band.
/// `header`/`footer` (Phase 7) and `footnotes` (Phase 8) are empty for now.
#[derive(Debug, Clone)]
pub struct Page {
    pub geometry: PageGeometry,
    pub kind: PageKind,
    /// 1-based page number (also the source of `{{ page.number }}` in Phase 7).
    pub number: u32,
    pub body: Vec<Fragment>,
    pub header: Vec<Fragment>,
    pub footer: Vec<Fragment>,
    pub footnotes: Vec<Fragment>,
    /// The render's fragment semantics (PDF/UA roles, internal links, anchors —
    /// see [`crate::semantics`]); empty unless built by [`crate::render_pages`]
    /// with the `pdf-ua`/`xref` features.
    pub semantics: crate::semantics::Semantics,
    /// The canvas colour (the document's root/`<body>` background, propagated as in
    /// a browser; miaskiewicz/turbo-html2pdf#29), painted over the page area behind
    /// everything else. `None` = no fill (white paper).
    pub canvas: Option<crate::layout::value::Rgba>,
}

/// Derive a page's kind from its 1-based number: page 1 is the first page, then
/// even/odd pages are the left/right (verso/recto) of a duplex spread.
fn page_kind(number: u32) -> PageKind {
    if number == 1 {
        PageKind::First
    } else if number.is_multiple_of(2) {
        PageKind::Left
    } else {
        PageKind::Right
    }
}

/// Assemble one [`Page`] from a walked body, shifting the body fragments from
/// body-local coordinates into absolute page coordinates.
fn assemble(geometry: PageGeometry, number: u32, mut body: Vec<Fragment>) -> Page {
    let (ox, oy) = geometry.body_origin();
    for frag in &mut body {
        frag.translate(ox, oy);
    }
    Page {
        geometry,
        kind: page_kind(number),
        number,
        body,
        header: Vec::new(),
        footer: Vec::new(),
        footnotes: Vec::new(),
        semantics: Default::default(),
        canvas: None,
    }
}

/// Paginate the galley `root` into pages against the geometry resolved from the
/// stylesheet's at-rules (§6.1–6.2). `diags` collects overflow lints.
pub fn paginate(
    root: &Fragment,
    at_rules: &[AtRule],
    diags: &mut Diagnostics,
) -> Result<Vec<Page>, RenderError> {
    let geometry = resolve_geometry(at_rules, PageGeometry::a4())?;
    Ok(paginate_with_geometry(root, geometry, diags))
}

/// Paginate the galley `root` against an already-resolved `geometry` (§6.1–6.2).
/// The Phase 7 orchestrator reserves the running header/footer bands into the
/// geometry first, so this entry takes the geometry directly rather than the
/// at-rules — its `body_height()` already nets out the reserved bands.
pub fn paginate_with_geometry(
    root: &Fragment,
    geometry: PageGeometry,
    diags: &mut Diagnostics,
) -> Vec<Page> {
    paginate_with_footnotes(root, geometry, &[], diags)
}

/// Paginate `root` while reserving each page's referenced footnotes via the
/// body/footnote fixpoint (§6.4). `notes` are the document's resolved footnotes,
/// each tagged inline on its marker fragment's `BreakMeta.footnotes`; with an
/// empty slice this is exactly the footnote-free walk.
pub fn paginate_with_footnotes(
    root: &Fragment,
    geometry: PageGeometry,
    notes: &[Note],
    diags: &mut Diagnostics,
) -> Vec<Page> {
    let galley = Galley {
        root,
        wrappers: &[],
        boxes: &FlowBoxes::default(),
    };
    paginate_document(galley, geometry, notes, diags)
}

/// A laid-out body flow as the fragmenter walks it.
#[derive(Debug, Clone, Copy)]
pub struct Galley<'a> {
    /// The galley root; its children are the page flow.
    pub root: &'a Fragment,
    /// The document's `<html>`/`<body>` boxes when they were dissolved into `root`
    /// before pagination (outermost first, in galley coordinates; their children are
    /// ignored): each one that paints a background or border is sliced onto every page
    /// (miaskiewicz/turbo-html2pdf#31, `box-decoration-break: slice`).
    pub wrappers: &'a [Fragment],
    /// The used box values of the flow's boxes (miaskiewicz/turbo-html2pdf#32): margins,
    /// paddings, `position`, insets. Empty = every box in flow, margins unknown.
    pub boxes: &'a FlowBoxes,
}

/// [`paginate_with_footnotes`] for a whole document: its [`Galley`] with the dissolved
/// `<html>`/`<body>` boxes and the boxes' used values.
pub fn paginate_document(
    galley: Galley,
    geometry: PageGeometry,
    notes: &[Note],
    diags: &mut Diagnostics,
) -> Vec<Page> {
    let capacity = geometry.body_height();
    let extracted = out_of_flow::extract(galley.root, galley.boxes, capacity);
    let flow = Galley {
        root: extracted.as_ref().map_or(galley.root, |e| &e.root),
        ..galley
    };
    let footnotes::Resolved {
        mut pages,
        bands,
        mut spans,
        anchors,
        deferred,
    } = footnotes::resolve(flow, geometry, notes, diags);
    let (mut bodies, mut bands): (Vec<_>, Vec<_>) =
        trim_trailing_pages(&mut pages, bands).into_iter().unzip();
    spans.truncate(bodies.len());
    let pages = out_of_flow::Paged {
        bodies: &mut bodies,
        spans: &mut spans,
        bands: &mut bands,
        capacity,
    };
    out_of_flow::place(extracted.as_ref(), &deferred, &anchors, pages);
    decorate::decorate(&mut bodies, &spans, galley.wrappers, flow.root.y);
    bodies
        .into_iter()
        .zip(bands)
        .enumerate()
        .map(|(i, (body, band))| assemble_page(geometry, i as u32 + 1, body, band))
        .collect()
}

/// Drop a single trailing empty page with no footnotes, pairing each surviving
/// body with its footnote band.
fn trim_trailing_pages(
    pages: &mut Vec<Vec<Fragment>>,
    mut bands: Vec<FootnoteBand>,
) -> Vec<(Vec<Fragment>, FootnoteBand)> {
    if pages.len() > 1
        && pages.last().is_some_and(Vec::is_empty)
        && bands.last().is_some_and(|b| b.fragments.is_empty())
    {
        pages.pop();
        bands.pop();
    }
    std::mem::take(pages).into_iter().zip(bands).collect()
}

/// Assemble one [`Page`] from a walked body plus its footnote band, shifting both
/// from local coordinates into absolute page coordinates.
fn assemble_page(
    geometry: PageGeometry,
    number: u32,
    body: Vec<Fragment>,
    band: FootnoteBand,
) -> Page {
    let mut page = assemble(geometry, number, body);
    page.footnotes = place_band(geometry, band);
    page
}

/// Translate a footnote band's fragments into their page position: the band top
/// sits just above the bottom margin and the footer band.
fn place_band(geometry: PageGeometry, band: FootnoteBand) -> Vec<Fragment> {
    let top = geometry.height - geometry.margin.bottom - geometry.footer_extent - band.height;
    band.fragments
        .into_iter()
        .map(|mut f| {
            f.translate(geometry.margin.left, top);
            f
        })
        .collect()
}
