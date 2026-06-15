//! Document assembly (§7): the catalog, page tree, per-page objects, font
//! objects and info dict, written in a fixed order so output is deterministic.
//!
//! Object layout (1-based ids): `1` catalog, `2` page tree, then for each page a
//! page object and its content stream, then all font objects (4 per face), then
//! all image objects (1 per image, +1 for each alpha SMask), then the info dict
//! last. `pdf-writer` serializes objects in id order, so this layout is stable
//! across runs. The font and image object ids are known from the layout up
//! front, so each page object is written exactly once with its resources.
//!
//! Phase 15 features and this object plan: `endnotes` and `print-color` needed
//! no change to the plan, so they ship without touching this file. The `pdf-a`
//! feature (AC-11.2) DOES extend it — under `#[cfg(feature = "pdf-a")]` two
//! objects (an sRGB ICC profile stream and an XMP `/Metadata` stream) follow the
//! info dict, an `OutputIntent` is attached to the catalog, and a trailer `/ID`
//! is set; all in [`super::pdfa`]. It is off by default, so the default object
//! plan and bytes are unchanged. The remaining two stay deferred:
//!
//! TODO(phase15b, feature `xref`, AC-3.25): named GoTo destinations for
//! `<t:anchor name>` plus internal-link annotations for `<a href="#name">`. A
//! two-pass emit (collect each anchor's positioned fragment, then write a Dests
//! name tree and per-page Annots link arrays). DEFERRED because positioned
//! fragments do not yet carry the anchor name / link href through layout and
//! pagination, so the annotation rect is unavailable at emit time; wiring that
//! through fragment.rs/boxgen.rs to 100% coverage is a larger change than this
//! slice could land cleanly.
//!
//! TODO(phase15b, feature `pdf-ua`, AC-11.1): a tagged StructTreeRoot built from
//! semantic HTML (headings/lists/tables), Alt text from `<img alt>`, and reading
//! order. The heaviest item; DEFERRED because the marked-content plumbing (BDC and
//! EMC around every painted run, plus the structure-element tree) reaches into the
//! per-page painter and would not reach 100% coverage in this slice.

use pdf_writer::{Finish, Name, Pdf, Rect, Ref};

use crate::image::ImageResolver;
use crate::paginate::Page;

use super::fonts::{FontStore, RefAlloc};
use super::image::ImageStore;
use super::meta::write_info;
use super::page::content_stream;
#[cfg(feature = "pdf-a")]
use super::pdfa;
#[cfg(feature = "pdf-ua")]
use super::ua;
use super::unit::px_to_pt;
use super::watermark;
#[cfg(feature = "xref")]
use super::xref::Xref;
use super::EmitOptions;

/// The number of PDF objects each embedded face occupies (Type0, CIDFont,
/// FontDescriptor, font program). Under `pdf-ua` each face also carries a
/// `/ToUnicode` CMap stream, so it occupies one more object (AC-11.1).
#[cfg(not(feature = "pdf-ua"))]
const OBJECTS_PER_FONT: i32 = 4;
#[cfg(feature = "pdf-ua")]
const OBJECTS_PER_FONT: i32 = 5;

/// Build the whole PDF document from the paginated pages.
pub fn build(pages: &[Page], opts: &EmitOptions, resolver: &dyn ImageResolver) -> Vec<u8> {
    let mut fonts = FontStore::collect(pages);
    let mut images = ImageStore::collect(pages, resolver);
    // A watermark's word glyphs / raster aren't carried by any fragment, so they
    // must be registered into the shared stores before the object plan is laid
    // out — text subsets like body text, an image rides the Phase 9b raster path.
    if let Some(mark) = &opts.watermark {
        watermark::collect(mark, &mut fonts, &mut images, resolver);
    }
    #[cfg(feature = "xref")]
    let xref = Xref::collect(pages);
    let plan = Plan::new(
        pages,
        &fonts,
        &images,
        #[cfg(feature = "xref")]
        &xref,
    );
    // The `pdf-ua` structure tree's objects start after every object the fixed
    // plan already allocated (info, plus any `xref`/`pdf-a` objects), so the
    // three features never claim the same object id.
    #[cfg(feature = "pdf-ua")]
    let (ua, _) = ua::UaPlan::build(pages, plan.next_free_id());
    let mut pdf = Pdf::new();
    pdf.set_version(1, 7);
    write_catalog(
        &mut pdf,
        &plan,
        #[cfg(feature = "pdf-ua")]
        &ua,
        #[cfg(feature = "pdf-ua")]
        opts,
    );
    write_page_tree(&mut pdf, pages, &plan);
    write_pages(
        &mut pdf,
        pages,
        &plan,
        &fonts,
        &images,
        opts,
        #[cfg(feature = "pdf-ua")]
        &ua,
    );
    fonts.write(&mut pdf, &mut plan.font_alloc());
    images.write(&mut pdf, &mut plan.image_alloc());
    write_info(&mut pdf, plan.info, opts);
    #[cfg(feature = "xref")]
    write_xref(&mut pdf, &plan, &xref);
    #[cfg(feature = "pdf-a")]
    write_pdfa_objects(&mut pdf, &plan, opts);
    #[cfg(feature = "pdf-ua")]
    ua.write(&mut pdf, &plan.page_refs, opts);
    pdf.finish()
}

/// Write the cross-reference objects (`xref` feature): the `/Dests` dictionary
/// (when any anchor exists) and the per-page Link annotation objects.
#[cfg(feature = "xref")]
fn write_xref(pdf: &mut Pdf, plan: &Plan, xref: &Xref) {
    if xref.has_dests() {
        xref.write_dests(pdf, plan.dests, &plan.page_refs);
    }
    xref.write_links(pdf, &plan.link_refs);
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
    /// The first image object id (images follow the fonts contiguously).
    images_start: i32,
    /// The main XObject ref of each image, in resource order.
    image_refs: Vec<Ref>,
    info: Ref,
    /// The `/Dests` dictionary object (`xref` feature). Meaningful only when
    /// `has_dests` is set; the catalog references it only then.
    #[cfg(feature = "xref")]
    dests: Ref,
    /// Whether the document defines any named destinations (`xref` feature).
    #[cfg(feature = "xref")]
    has_dests: bool,
    /// The Link annotation objects in page/document order (`xref` feature).
    #[cfg(feature = "xref")]
    link_refs: Vec<Ref>,
    /// The Link annotation refs grouped per page (`xref` feature), parallel to
    /// the pages, so each page object can write its `/Annots` array by index.
    #[cfg(feature = "xref")]
    page_annot_refs: Vec<Vec<Ref>>,
    /// The embedded sRGB ICC profile stream (`pdf-a` only): the `OutputIntent`'s
    /// `DestOutputProfile`. Laid out after the info dict (and after any `xref`
    /// objects) so the default object plan is untouched when the feature is off.
    #[cfg(feature = "pdf-a")]
    icc: Ref,
    /// The XMP `/Metadata` stream (`pdf-a` only), declaring PDF/A-2b.
    #[cfg(feature = "pdf-a")]
    xmp: Ref,
}

impl Plan {
    fn new(
        pages: &[Page],
        fonts: &FontStore,
        images: &ImageStore,
        #[cfg(feature = "xref")] xref: &Xref,
    ) -> Plan {
        let mut next = 3;
        let page_refs = page_ref_pairs(pages.len(), &mut next);
        let fonts_start = next;
        let font_refs = type0_refs(fonts_start, fonts.len());
        let images_start = fonts_start + OBJECTS_PER_FONT * fonts.len() as i32;
        let image_refs = images.xobject_refs(images_start);
        let info_id = images_start + images.total_objects();
        let info = Ref::new(info_id);
        // Cross-reference objects follow the info dict: an optional `/Dests`
        // dictionary, then one object per Link annotation.
        #[cfg(feature = "xref")]
        let (dests, link_refs) = xref_refs(info_id + 1, xref);
        #[cfg(feature = "xref")]
        let page_annot_refs = (0..pages.len())
            .map(|i| xref.page_annots(i, &link_refs).to_vec())
            .collect();
        // The optional PDF/A objects (ICC + XMP) go after the info dict and any
        // `xref` objects, so the two features never claim the same object id.
        #[cfg(all(feature = "pdf-a", feature = "xref"))]
        let pdfa_start = info_id + 1 + xref_object_count(xref);
        #[cfg(all(feature = "pdf-a", not(feature = "xref")))]
        let pdfa_start = info_id + 1;
        Plan {
            catalog: Ref::new(1),
            page_tree: Ref::new(2),
            page_refs,
            fonts_start,
            font_refs,
            images_start,
            image_refs,
            info,
            #[cfg(feature = "xref")]
            dests,
            #[cfg(feature = "xref")]
            has_dests: xref.has_dests(),
            #[cfg(feature = "xref")]
            link_refs,
            #[cfg(feature = "xref")]
            page_annot_refs,
            // The two PDF/A objects (ICC + XMP) follow the info dict (and any
            // xref objects).
            #[cfg(feature = "pdf-a")]
            icc: Ref::new(pdfa_start),
            #[cfg(feature = "pdf-a")]
            xmp: Ref::new(pdfa_start + 1),
        }
    }

    /// A fresh allocator positioned at the first font object.
    fn font_alloc(&self) -> RefAlloc {
        RefAlloc::new(self.fonts_start)
    }

    /// A fresh allocator positioned at the first image object.
    fn image_alloc(&self) -> RefAlloc {
        RefAlloc::new(self.images_start)
    }

    /// The first object id not used by the fixed plan — where further feature
    /// objects (the `pdf-ua` structure tree) begin. Accounts for the optional
    /// `xref` objects (a `/Dests` dict + one per Link) and `pdf-a` objects (ICC +
    /// XMP), so the three features never claim the same id when co-enabled.
    #[cfg(feature = "pdf-ua")]
    fn next_free_id(&self) -> i32 {
        // `mut` is only exercised when `xref`/`pdf-a` are also on; with neither,
        // the blocks below are cfg'd out and the binding is never reassigned.
        #[allow(unused_mut)]
        let mut next = self.info.get() + 1;
        #[cfg(feature = "xref")]
        {
            next += i32::from(self.has_dests) + self.link_refs.len() as i32;
        }
        #[cfg(feature = "pdf-a")]
        {
            next += 2;
        }
        next
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

/// Allocate the cross-reference object refs starting at `start` (`xref`
/// feature): a `/Dests` dictionary first (when any anchor exists), then one ref
/// per Link annotation. The dests ref is unused — and never referenced by the
/// catalog — when the document defines no destinations.
#[cfg(feature = "xref")]
fn xref_refs(start: i32, xref: &Xref) -> (Ref, Vec<Ref>) {
    let mut next = start;
    let dests = Ref::new(next);
    if xref.has_dests() {
        next += 1;
    }
    let link_refs = (0..xref.link_count() as i32)
        .map(|i| Ref::new(next + i))
        .collect();
    (dests, link_refs)
}

/// The number of object ids the `xref` feature consumes after the info dict: an
/// optional `/Dests` dictionary plus one object per Link annotation. Used to
/// offset the `pdf-a` objects so the two features never collide when both are on.
#[cfg(all(feature = "xref", feature = "pdf-a"))]
fn xref_object_count(xref: &Xref) -> i32 {
    i32::from(xref.has_dests()) + xref.link_count() as i32
}

#[cfg(not(feature = "pdf-ua"))]
fn write_catalog(pdf: &mut Pdf, plan: &Plan) {
    let mut catalog = pdf.catalog(plan.catalog);
    catalog.pages(plan.page_tree);
    #[cfg(feature = "xref")]
    if plan.has_dests {
        catalog.destinations(plan.dests);
    }
    // PDF/A-2b: attach the OutputIntent (sRGB) and the XMP `/Metadata` stream to
    // the catalog. Off by default, so the default catalog is byte-for-byte the
    // single `/Pages` entry.
    #[cfg(feature = "pdf-a")]
    pdfa::write_catalog_entries(&mut catalog, plan.icc, plan.xmp);
    catalog.finish();
}

/// Write the PDF/A objects (ICC profile + XMP packet) and set the trailer `/ID`
/// PDF/A requires. Called only under `#[cfg(feature = "pdf-a")]`.
#[cfg(feature = "pdf-a")]
fn write_pdfa_objects(pdf: &mut Pdf, plan: &Plan, opts: &EmitOptions) {
    pdfa::write_icc_profile(pdf, plan.icc);
    pdfa::write_metadata(pdf, plan.xmp, opts);
    pdf.set_file_id(pdfa::file_id(opts));
}

/// `pdf-ua` catalog: the page tree plus the tagged-PDF wiring (`StructTreeRoot`,
/// `MarkInfo`, `/Lang`, the XMP metadata stream and `DisplayDocTitle`).
#[cfg(feature = "pdf-ua")]
fn write_catalog(pdf: &mut Pdf, plan: &Plan, ua: &ua::UaPlan, opts: &EmitOptions) {
    use pdf_writer::TextStr;
    let mut cat = pdf.catalog(plan.catalog);
    cat.pages(plan.page_tree);
    // When co-enabled with `xref`/`pdf-a`, their catalog entries ride along too,
    // so enabling accessibility never drops links or the archival OutputIntent.
    #[cfg(feature = "xref")]
    if plan.has_dests {
        cat.destinations(plan.dests);
    }
    #[cfg(feature = "pdf-a")]
    pdfa::write_catalog_entries(&mut cat, plan.icc, plan.xmp);
    cat.pair(Name(b"StructTreeRoot"), ua.root_ref());
    cat.mark_info().marked(true);
    cat.metadata(ua.metadata_ref());
    let lang = opts.lang.as_deref().unwrap_or("en-US");
    cat.lang(TextStr(lang));
    cat.viewer_preferences().display_doc_title(true);
}

fn write_page_tree(pdf: &mut Pdf, pages: &[Page], plan: &Plan) {
    let kids = plan.page_refs.iter().map(|(p, _)| *p);
    pdf.pages(plan.page_tree)
        .kids(kids)
        .count(pages.len() as i32);
}

/// Write each page object (with resources) and its content stream.
#[cfg(not(feature = "pdf-ua"))]
fn write_pages(
    pdf: &mut Pdf,
    pages: &[Page],
    plan: &Plan,
    fonts: &FontStore,
    images: &ImageStore,
    opts: &EmitOptions,
) {
    for (i, (page, (page_ref, content_ref))) in pages.iter().zip(&plan.page_refs).enumerate() {
        write_page_object(pdf, page, plan, (*page_ref, *content_ref), opts, i);
        let bytes = content_stream(page, fonts, images, opts.watermark.as_ref());
        pdf.stream(*content_ref, &bytes);
    }
}

/// `pdf-ua` variant: each page also carries its `/StructParents` key and its
/// content stream's marked-content tags (the page index threads both).
#[cfg(feature = "pdf-ua")]
fn write_pages(
    pdf: &mut Pdf,
    pages: &[Page],
    plan: &Plan,
    fonts: &FontStore,
    images: &ImageStore,
    opts: &EmitOptions,
    ua: &ua::UaPlan,
) {
    for (i, (page, (page_ref, content_ref))) in pages.iter().zip(&plan.page_refs).enumerate() {
        write_page_object(pdf, page, plan, (*page_ref, *content_ref), opts, i);
        let bytes = content_stream(
            page,
            fonts,
            images,
            opts.watermark.as_ref(),
            &ua.page_tags(i),
        );
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

fn write_page_object(
    pdf: &mut Pdf,
    page: &Page,
    plan: &Plan,
    refs: (Ref, Ref),
    opts: &EmitOptions,
    page_idx: usize,
) {
    let (page_ref, content_ref) = refs;
    let mut obj = pdf.page(page_ref);
    obj.parent(plan.page_tree);
    obj.media_box(media_box(page));
    obj.contents(content_ref);
    #[cfg(feature = "pdf-ua")]
    obj.struct_parents(page_idx as i32);
    write_resources(&mut obj, plan, opts);
    write_page_annots(&mut obj, plan, page_idx);
    obj.finish();
}

/// Write the page's `/Annots` array of Link annotations, when the `xref` feature
/// is on and this page carries any internal links. A no-op otherwise, so the
/// default page object is byte-for-byte unchanged.
#[cfg(feature = "xref")]
fn write_page_annots(obj: &mut pdf_writer::writers::Page, plan: &Plan, page_idx: usize) {
    let annots = &plan.page_annot_refs[page_idx];
    if !annots.is_empty() {
        obj.annotations(annots.iter().copied());
    }
}

#[cfg(not(feature = "xref"))]
fn write_page_annots(_obj: &mut pdf_writer::writers::Page, _plan: &Plan, _page_idx: usize) {}

/// Write the page's resource dictionary: fonts, image XObjects, then the
/// watermark fade `ExtGState` when a watermark is present. The font and image
/// dictionaries are written even when empty, which conformant viewers accept.
fn write_resources(obj: &mut pdf_writer::writers::Page, plan: &Plan, opts: &EmitOptions) {
    let mut resources = obj.resources();
    write_font_dict(&mut resources, &plan.font_refs);
    write_image_dict(&mut resources, &plan.image_refs);
    // PDF/A-2b forbids transparency, so the watermark's `/ca` fade `ExtGState`
    // is not emitted under `pdf-a` (the mark prints at full opacity instead).
    #[cfg(not(feature = "pdf-a"))]
    if let Some(mark) = &opts.watermark {
        write_fade_gs(&mut resources, watermark::opacity(mark));
    }
    #[cfg(feature = "pdf-a")]
    let _ = opts;
}

/// Write the watermark's `/GSwm` fade `ExtGState` inline (a simple `/ca` dict
/// needs no indirect object), referenced by the content stream's `gs` operator.
/// Not compiled under `pdf-a`, which forbids the `/ca` transparency entirely.
#[cfg(not(feature = "pdf-a"))]
fn write_fade_gs(resources: &mut pdf_writer::writers::Resources, opacity: f32) {
    let mut states = resources.ext_g_states();
    states
        .insert(Name(watermark::FADE_GS_NAME.as_bytes()))
        .start::<pdf_writer::writers::ExtGraphicsState>()
        .non_stroking_alpha(opacity);
    states.finish();
}

/// Map each font resource name (`F0`, …) to its Type0 font object.
fn write_font_dict(resources: &mut pdf_writer::writers::Resources, font_refs: &[Ref]) {
    let mut dict = resources.fonts();
    for (i, font_ref) in font_refs.iter().enumerate() {
        let name = FontStore::resource_name(i);
        dict.pair(Name(name.as_bytes()), *font_ref);
    }
    dict.finish();
}

/// Map each image resource name (`Im0`, …) to its main XObject.
fn write_image_dict(resources: &mut pdf_writer::writers::Resources, image_refs: &[Ref]) {
    let mut dict = resources.x_objects();
    for (i, image_ref) in image_refs.iter().enumerate() {
        let name = ImageStore::resource_name(i);
        dict.pair(Name(name.as_bytes()), *image_ref);
    }
    dict.finish();
}
