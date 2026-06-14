//! Document assembly (§7): the catalog, page tree, per-page objects, font
//! objects and info dict, written in a fixed order so output is deterministic.
//!
//! Object layout (1-based ids): `1` catalog, `2` page tree, then for each page a
//! page object and its content stream, then all font objects (4 per face), then
//! the info dict last. `pdf-writer` serializes objects in id order, so this
//! layout is stable across runs. The font object ids are known from the layout
//! up front, so each page object is written exactly once with its resources.

use pdf_writer::{Finish, Name, Pdf, Rect, Ref};

use crate::paginate::Page;

use super::fonts::{FontStore, RefAlloc};
use super::meta::write_info;
use super::page::content_stream;
use super::unit::px_to_pt;
use super::EmitOptions;

/// The number of PDF objects each embedded face occupies (Type0, CIDFont,
/// FontDescriptor, font program).
const OBJECTS_PER_FONT: i32 = 4;

/// Build the whole PDF document from the paginated pages.
pub fn build(pages: &[Page], opts: &EmitOptions) -> Vec<u8> {
    let fonts = FontStore::collect(pages);
    let plan = Plan::new(pages, &fonts);
    let mut pdf = Pdf::new();
    pdf.set_version(1, 7);
    write_catalog(&mut pdf, &plan);
    write_page_tree(&mut pdf, pages, &plan);
    write_pages(&mut pdf, pages, &plan, &fonts);
    fonts.write(&mut pdf, &mut plan.font_alloc());
    write_info(&mut pdf, plan.info, opts);
    pdf.finish()
}

/// The fixed reference layout for one build.
struct Plan {
    catalog: Ref,
    page_tree: Ref,
    /// `(page_obj, content_obj)` for each page, in page order.
    page_refs: Vec<(Ref, Ref)>,
    /// The first font object id (4 objects per face follow contiguously).
    fonts_start: i32,
    /// The Type0 font object for each face, in resource order.
    font_refs: Vec<Ref>,
    info: Ref,
}

impl Plan {
    fn new(pages: &[Page], fonts: &FontStore) -> Plan {
        let mut next = 3;
        let page_refs = page_ref_pairs(pages.len(), &mut next);
        let fonts_start = next;
        let font_refs = type0_refs(fonts_start, fonts.len());
        let info = Ref::new(fonts_start + OBJECTS_PER_FONT * fonts.len() as i32);
        Plan {
            catalog: Ref::new(1),
            page_tree: Ref::new(2),
            page_refs,
            fonts_start,
            font_refs,
            info,
        }
    }

    /// A fresh allocator positioned at the first font object.
    fn font_alloc(&self) -> RefAlloc {
        RefAlloc::new(self.fonts_start)
    }
}

/// Allocate the `(page, content)` ref pair for each page, advancing `next`.
fn page_ref_pairs(count: usize, next: &mut i32) -> Vec<(Ref, Ref)> {
    (0..count)
        .map(|_| {
            let pair = (Ref::new(*next), Ref::new(*next + 1));
            *next += 2;
            pair
        })
        .collect()
}

/// The Type0 font ref of each face: the first of its four contiguous objects.
fn type0_refs(start: i32, count: usize) -> Vec<Ref> {
    (0..count as i32)
        .map(|i| Ref::new(start + OBJECTS_PER_FONT * i))
        .collect()
}

fn write_catalog(pdf: &mut Pdf, plan: &Plan) {
    pdf.catalog(plan.catalog).pages(plan.page_tree);
}

fn write_page_tree(pdf: &mut Pdf, pages: &[Page], plan: &Plan) {
    let kids = plan.page_refs.iter().map(|(p, _)| *p);
    pdf.pages(plan.page_tree)
        .kids(kids)
        .count(pages.len() as i32);
}

/// Write each page object (with resources) and its content stream.
fn write_pages(pdf: &mut Pdf, pages: &[Page], plan: &Plan, fonts: &FontStore) {
    for (page, (page_ref, content_ref)) in pages.iter().zip(&plan.page_refs) {
        write_page_object(pdf, page, plan, *page_ref, *content_ref);
        let bytes = content_stream(page, fonts);
        pdf.stream(*content_ref, &bytes);
    }
}

/// The page's `MediaBox` in points, from its geometry.
fn media_box(page: &Page) -> Rect {
    Rect::new(
        0.0,
        0.0,
        px_to_pt(page.geometry.width),
        px_to_pt(page.geometry.height),
    )
}

fn write_page_object(pdf: &mut Pdf, page: &Page, plan: &Plan, page_ref: Ref, content_ref: Ref) {
    let mut obj = pdf.page(page_ref);
    obj.parent(plan.page_tree);
    obj.media_box(media_box(page));
    obj.contents(content_ref);
    write_font_resources(&mut obj, &plan.font_refs);
    obj.finish();
}

/// Map each font resource name (`F0`, …) to its Type0 font object.
fn write_font_resources(obj: &mut pdf_writer::writers::Page, font_refs: &[Ref]) {
    let mut resources = obj.resources();
    let mut dict = resources.fonts();
    for (i, font_ref) in font_refs.iter().enumerate() {
        let name = FontStore::resource_name(i);
        dict.pair(Name(name.as_bytes()), *font_ref);
    }
}
