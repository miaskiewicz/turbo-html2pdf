//! Internal links & cross-references (`xref` feature, AC-3.25). Two-pass over the
//! paginated pages: pass 1 ([`Xref::collect`]) walks every fragment to record
//! each `<t:anchor name>`'s page + position (a named GoTo destination) and each
//! `<a href="#name">`'s page + rectangle (a Link annotation); pass 2 writes a
//! `/Dests` dictionary on the catalog plus a per-page `/Annots` array of Link
//! annotations whose GoTo action targets the matching destination.
//!
//! Coordinates flip from the galley (px, y-down) into PDF user space (pt, y-up)
//! exactly as the painter does, so a link rect and its dest land where the
//! fragment was positioned; each anchor/link captures its host page height (pt)
//! at collection time so the flip needs no later geometry lookup. Unmatched
//! anchors or links are harmless. Determinism (AC-7.6) holds: destinations sort
//! by name and annotations follow page/document order.

use std::collections::BTreeMap;

use pdf_writer::{Finish, Name, Pdf, Rect, Ref};

use crate::layout::fragment::Fragment;
use crate::paginate::Page;
use crate::semantics::Semantics;

use super::unit::{flip_y, px_to_pt};

/// A named destination: the page object index plus the galley point (px) of the
/// anchor fragment's top-left and the host page height (pt) for the y-flip.
struct Dest {
    page: usize,
    x: f32,
    y: f32,
    page_height_pt: f32,
}

/// One internal link: the destination name it targets, its galley rectangle
/// (px), and the host page height (pt) for the y-flip.
struct Link {
    target: String,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    page_height_pt: f32,
}

/// The cross-reference data collected from a document's pages.
pub(crate) struct Xref {
    /// Destination name → its target, sorted by name for stable output.
    dests: BTreeMap<String, Dest>,
    /// Links per page, in page then document order (parallel to the pages).
    links: Vec<Vec<Link>>,
}

impl Xref {
    /// Pass 1: walk every fragment of every page, recording anchors and links.
    pub(crate) fn collect(pages: &[Page]) -> Xref {
        let mut dests = BTreeMap::new();
        let mut links = Vec::with_capacity(pages.len());
        for (page_idx, page) in pages.iter().enumerate() {
            let h = px_to_pt(page.geometry.height);
            let mut page_links = Vec::new();
            for band in bands(page) {
                for frag in band {
                    walk(
                        frag,
                        &page.semantics,
                        (page_idx, h),
                        &mut dests,
                        &mut page_links,
                    );
                }
            }
            links.push(page_links);
        }
        Xref { dests, links }
    }

    /// Whether the document defines any named destinations.
    pub(crate) fn has_dests(&self) -> bool {
        !self.dests.is_empty()
    }

    /// The number of link annotation objects, summed over pages.
    pub(crate) fn link_count(&self) -> usize {
        self.links.iter().map(Vec::len).sum()
    }

    /// The annotation refs for the links on `page_idx`, sliced from the document
    /// order `link_refs` (parallel to [`Self::links`]).
    pub(crate) fn page_annots<'a>(&self, page_idx: usize, link_refs: &'a [Ref]) -> &'a [Ref] {
        let start: usize = self.links[..page_idx].iter().map(Vec::len).sum();
        &link_refs[start..start + self.links[page_idx].len()]
    }

    /// Write the `/Dests` dictionary mapping each destination name to a
    /// `[page /XYZ left top]` array (scroll to the anchor's top, native zoom).
    pub(crate) fn write_dests(&self, pdf: &mut Pdf, dests_ref: Ref, page_refs: &[(Ref, Ref)]) {
        let mut dict = pdf.indirect(dests_ref).dict();
        for (name, dest) in &self.dests {
            let (page_obj, _) = page_refs[dest.page];
            dict.insert(Name(name.as_bytes()))
                .start::<pdf_writer::writers::Destination>()
                .page(page_obj)
                .xyz(px_to_pt(dest.x), flip_y(dest.y, dest.page_height_pt), None);
        }
        dict.finish();
    }

    /// Write one Link annotation object per link (page then document order), each
    /// a `GoTo` action to its named destination over the fragment's rectangle.
    pub(crate) fn write_links(&self, pdf: &mut Pdf, link_refs: &[Ref]) {
        let mut refs = link_refs.iter();
        for page in &self.links {
            for link in page {
                let id = *refs.next().expect("one ref per link");
                write_link(pdf, id, link);
            }
        }
    }
}

/// The four paint bands of a page, matching the per-page painter's order.
fn bands(page: &Page) -> [&[Fragment]; 4] {
    [&page.body, &page.header, &page.footer, &page.footnotes]
}

/// Record a fragment's anchor and/or link, then recurse into its children.
fn walk(
    frag: &Fragment,
    semantics: &Semantics,
    (page_idx, page_height_pt): (usize, f32),
    dests: &mut BTreeMap<String, Dest>,
    links: &mut Vec<Link>,
) {
    if let Some(name) = semantics.anchor(frag) {
        dests.entry(name.to_string()).or_insert(Dest {
            page: page_idx,
            x: frag.x,
            y: frag.y,
            page_height_pt,
        });
    }
    if let Some(target) = semantics.link_href(frag) {
        links.push(Link {
            target: target.to_string(),
            x: frag.x,
            y: frag.y,
            width: frag.width,
            height: frag.height,
            page_height_pt,
        });
    }
    for child in &frag.children {
        walk(child, semantics, (page_idx, page_height_pt), dests, links);
    }
}

/// Write a single Link annotation: its rectangle plus a `GoTo` action to the
/// named destination, with no visible border so it doesn't paint a box.
fn write_link(pdf: &mut Pdf, id: Ref, link: &Link) {
    let mut annot = pdf.annotation(id);
    annot.subtype(pdf_writer::types::AnnotationType::Link);
    annot.rect(link_rect(link));
    annot.border(0.0, 0.0, 0.0, None);
    annot
        .action()
        .action_type(pdf_writer::types::ActionType::GoTo)
        .destination_named(Name(link.target.as_bytes()));
    annot.finish();
}

/// The PDF-space rectangle (pt, y-up) covering a link's galley box.
fn link_rect(link: &Link) -> Rect {
    let x1 = px_to_pt(link.x);
    let x2 = px_to_pt(link.x + link.width);
    let y_top = flip_y(link.y, link.page_height_pt);
    let y_bot = flip_y(link.y + link.height, link.page_height_pt);
    Rect::new(x1, y_bot, x2, y_top)
}
