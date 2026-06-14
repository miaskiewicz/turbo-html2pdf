//! The fragmenter (§6, Stage 4): turns the continuous galley into a sequence of
//! fixed-size [`Page`]s. The number of pages is an *output* of walking the
//! content against a page's body capacity, never an input (AC-6.0): the same
//! template over more data simply yields more pages.
//!
//! This phase delivers the structural spine — geometry resolution and the break
//! walk. Running headers/footers, page masters, page-number late-evaluation
//! (`{{ page.number }}`, `<t:page/>`), and footnote reservation are layered on in
//! Phases 7 and 8; the hooks they need (the `header`/`footer`/`footnotes` page
//! bands and the footnote-area capacity term) are already exposed here.

mod geometry;
mod walk;

use crate::error::{Diagnostics, RenderError};
use crate::layout::fragment::Fragment;
use crate::style::AtRule;

pub use geometry::{resolve_geometry, PageGeometry};

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
}

/// Derive a page's kind from its 1-based number: page 1 is the first page, then
/// even/odd pages are the left/right (verso/recto) of a duplex spread.
fn page_kind(number: u32) -> PageKind {
    if number == 1 {
        PageKind::First
    } else if number % 2 == 0 {
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
    }
}

/// Drop a single trailing empty page (left by a `break-after:page` on the last
/// block), keeping at least one page so an empty document still yields a page.
fn trim_trailing_empty(pages: &mut Vec<Vec<Fragment>>) {
    if pages.len() > 1 && pages.last().is_some_and(Vec::is_empty) {
        pages.pop();
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
    // Footnote reservation (Phase 8) will subtract its measured area here; until
    // then the whole body height is available to the walk.
    let footnote_area_height = 0.0;
    let capacity = geometry.body_height() - footnote_area_height;
    let mut bodies = walk::walk(root, capacity, diags);
    trim_trailing_empty(&mut bodies);
    bodies
        .into_iter()
        .enumerate()
        .map(|(i, body)| assemble(geometry, i as u32 + 1, body))
        .collect()
}
