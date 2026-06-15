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
/// FontDescriptor, font program): 4 by default. A `pdf-ua` render adds a
/// per-face `/ToUnicode` CMap stream, so each face occupies one more object
/// (AC-11.1) — a *runtime* count, decided per render by `opts.pdf_ua`, so a
/// flag-off render under the `pdf-ua` build keeps the default 4-object plan.
fn objects_per_font(pdf_ua: bool) -> i32 {
    if pdf_ua {
        5
    } else {
        4
    }
}

/// Whether this render emits PDF/A-2b objects: `opts.pdf_a` under the `pdf-a`
/// feature, else a compile-time `false` so the default build folds the branch
/// away and the gated objects never enter the plan.
#[cfg(feature = "pdf-a")]
fn pdf_a_on(opts: &EmitOptions) -> bool {
    opts.pdf_a
}
#[cfg(not(feature = "pdf-a"))]
fn pdf_a_on(_opts: &EmitOptions) -> bool {
    false
}

/// Whether this render emits PDF/UA tagged-PDF objects: `opts.pdf_ua` under the
/// `pdf-ua` feature, else a compile-time `false`.
#[cfg(feature = "pdf-ua")]
fn pdf_ua_on(opts: &EmitOptions) -> bool {
    opts.pdf_ua
}
#[cfg(not(feature = "pdf-ua"))]
fn pdf_ua_on(_opts: &EmitOptions) -> bool {
    false
}

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
        opts,
        #[cfg(feature = "xref")]
        &xref,
    );
    // The `pdf-ua` structure tree's objects start after every object the plan
    // already allocated (info, plus any `xref`/`pdf-a` objects), so the three
    // features never claim the same object id. Built only when the render
    // actually emits tagged PDF (`opts.pdf_ua`); otherwise the plan reserves no
    // struct-tree ids and the catalog/pages take their untagged path.
    #[cfg(feature = "pdf-ua")]
    let ua = pdf_ua_on(opts).then(|| ua::UaPlan::build(pages, plan.next_free_id()).0);
    let mut pdf = Pdf::new();
    pdf.set_version(1, 7);
    write_catalog(
        &mut pdf,
        &plan,
        opts,
        #[cfg(feature = "pdf-ua")]
        ua.as_ref(),
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
        ua.as_ref(),
    );
    fonts.write(&mut pdf, &mut plan.font_alloc(), opts);
    images.write(&mut pdf, &mut plan.image_alloc());
    write_info(&mut pdf, plan.info, opts);
    #[cfg(feature = "xref")]
    write_xref(&mut pdf, &plan, &xref);
    #[cfg(feature = "pdf-a")]
    if pdf_a_on(opts) {
        write_pdfa_objects(&mut pdf, &plan, opts);
    }
    #[cfg(feature = "pdf-ua")]
    if let Some(ua) = &ua {
        ua.write(&mut pdf, &plan.page_refs, opts);
    }
    finish(pdf, opts)
}

/// Serialise the finished `pdf`, applying AES-256 password encryption when the
/// caller set [`EmitOptions::encryption`] (the `encrypt` feature). Without the
/// feature — or with `encryption: None` — this is exactly `pdf.finish()`, so the
/// default path is byte-for-byte unchanged.
#[cfg(feature = "encrypt")]
fn finish(pdf: pdf_writer::Pdf, opts: &EmitOptions) -> Vec<u8> {
    let bytes = pdf.finish();
    match &opts.encryption {
        Some(enc) => super::encrypt::encrypt_pdf(&bytes, enc),
        None => bytes,
    }
}

/// Without the `encrypt` feature, finishing is just `pdf.finish()`.
#[cfg(not(feature = "encrypt"))]
fn finish(pdf: pdf_writer::Pdf, _opts: &EmitOptions) -> Vec<u8> {
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
    /// objects) so the default object plan is untouched when the render does not
    /// emit PDF/A. Only meaningful — and only an allocated object — when
    /// `pdf_a` is set; otherwise no ICC/XMP ids are reserved at all.
    #[cfg(feature = "pdf-a")]
    icc: Ref,
    /// The XMP `/Metadata` stream (`pdf-a` only), declaring PDF/A-2b.
    #[cfg(feature = "pdf-a")]
    xmp: Ref,
    /// Whether this render emits PDF/A objects (resolved from `opts.pdf_a`). When
    /// `false`, `icc`/`xmp` reserve no ids and the catalog skips the OutputIntent.
    #[cfg(feature = "pdf-a")]
    pdf_a: bool,
    /// Whether this render emits PDF/UA tagged-PDF objects (resolved from
    /// `opts.pdf_ua`). Drives the struct-tree allocation and each page's
    /// `/StructParents` key, so a flag-off render keeps the default object plan.
    /// (The font-object count is decided in [`Plan::new`] from the same flag.)
    #[cfg(feature = "pdf-ua")]
    pdf_ua: bool,
}

impl Plan {
    fn new(
        pages: &[Page],
        fonts: &FontStore,
        images: &ImageStore,
        opts: &EmitOptions,
        #[cfg(feature = "xref")] xref: &Xref,
    ) -> Plan {
        let pdf_ua = pdf_ua_on(opts);
        let per_font = objects_per_font(pdf_ua);
        let mut next = 3;
        let page_refs = page_ref_pairs(pages.len(), &mut next);
        let fonts_start = next;
        let font_refs = type0_refs(fonts_start, fonts.len(), per_font);
        let images_start = fonts_start + per_font * fonts.len() as i32;
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
        // `xref` objects, so the two never claim the same id. Reserved only when
        // this render emits PDF/A; otherwise the ids stay free for whatever
        // follows (the `pdf-ua` struct tree), keeping a flag-off plan unchanged.
        #[cfg(feature = "pdf-a")]
        let pdf_a = pdf_a_on(opts);
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
            // xref objects) — only when this render emits PDF/A.
            #[cfg(feature = "pdf-a")]
            icc: Ref::new(pdfa_start),
            #[cfg(feature = "pdf-a")]
            xmp: Ref::new(pdfa_start + 1),
            #[cfg(feature = "pdf-a")]
            pdf_a,
            #[cfg(feature = "pdf-ua")]
            pdf_ua,
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

    /// The first object id not used by the plan — where the `pdf-ua` structure
    /// tree begins. Accounts for the optional `xref` objects (a `/Dests` dict +
    /// one per Link) and the `pdf-a` objects (ICC + XMP), at runtime: the two
    /// PDF/A ids are counted only when this render emits PDF/A, so the three
    /// features never claim the same id when co-enabled and a flag-off render
    /// starts the struct tree exactly where the default plan would.
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
        if self.pdf_a {
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

/// The Type0 font ref of each face: the first of its `per_font` contiguous
/// objects (4 normally, 5 under a `pdf-ua` render).
fn type0_refs(start: i32, count: usize, per_font: i32) -> Vec<Ref> {
    (0..count as i32)
        .map(|i| Ref::new(start + per_font * i))
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

/// Write the document catalog. The default catalog is byte-for-byte the single
/// `/Pages` entry; each feature adds its entries *at runtime* so a flag-off
/// render under a feature build emits exactly the default catalog:
///
/// * `xref` (`plan.has_dests`): a `/Dests` name-tree reference.
/// * `pdf-a` (`plan.pdf_a`): the sRGB `OutputIntent` and the XMP `/Metadata`.
/// * `pdf-ua` (`ua` present): the `StructTreeRoot`, `/MarkInfo`, the tagged XMP
///   `/Metadata`, `/Lang` and the `DisplayDocTitle` viewer preference.
///
/// Entry order is fixed (pages → dests → pdf-a → pdf-ua) so co-enabling never
/// reshuffles the bytes a smaller feature set produces.
fn write_catalog(
    pdf: &mut Pdf,
    plan: &Plan,
    opts: &EmitOptions,
    #[cfg(feature = "pdf-ua")] ua: Option<&ua::UaPlan>,
) {
    let mut cat = pdf.catalog(plan.catalog);
    cat.pages(plan.page_tree);
    #[cfg(feature = "xref")]
    if plan.has_dests {
        cat.destinations(plan.dests);
    }
    // PDF/A-2b: attach the OutputIntent (sRGB) and the XMP `/Metadata` stream,
    // only when this render emits PDF/A.
    #[cfg(feature = "pdf-a")]
    if plan.pdf_a {
        pdfa::write_catalog_entries(&mut cat, plan.icc, plan.xmp);
    }
    // PDF/UA: the tagged-PDF wiring, only when this render emits tagged PDF.
    #[cfg(feature = "pdf-ua")]
    if let Some(ua) = ua {
        use pdf_writer::TextStr;
        cat.pair(Name(b"StructTreeRoot"), ua.root_ref());
        cat.mark_info().marked(true);
        // The catalog carries a single `/Metadata`. A PDF/A render already wrote
        // one above (its XMP packet), so we don't add a second key here; a
        // pure-UA render writes the tagged XMP packet as the catalog metadata.
        #[cfg(feature = "pdf-a")]
        let pdfa_wrote_metadata = plan.pdf_a;
        #[cfg(not(feature = "pdf-a"))]
        let pdfa_wrote_metadata = false;
        if !pdfa_wrote_metadata {
            cat.metadata(ua.metadata_ref());
        }
        let lang = opts.lang.as_deref().unwrap_or("en-US");
        cat.lang(TextStr(lang));
        cat.viewer_preferences().display_doc_title(true);
    }
    let _ = opts;
    cat.finish();
}

/// Write the PDF/A objects (ICC profile + XMP packet) and set the trailer `/ID`
/// PDF/A requires. Called only when this render emits PDF/A.
#[cfg(feature = "pdf-a")]
fn write_pdfa_objects(pdf: &mut Pdf, plan: &Plan, opts: &EmitOptions) {
    pdfa::write_icc_profile(pdf, plan.icc);
    pdfa::write_metadata(pdf, plan.xmp, opts);
    pdf.set_file_id(pdfa::file_id(opts));
}

fn write_page_tree(pdf: &mut Pdf, pages: &[Page], plan: &Plan) {
    let kids = plan.page_refs.iter().map(|(p, _)| *p);
    pdf.pages(plan.page_tree)
        .kids(kids)
        .count(pages.len() as i32);
}

/// Write each page object (with resources) and its content stream. A tagged
/// render (`ua` present) additionally writes each page's `/StructParents` key and
/// its content stream's marked-content tags; a flag-off render writes the plain
/// page object and stream, byte-for-byte the untagged output.
fn write_pages(
    pdf: &mut Pdf,
    pages: &[Page],
    plan: &Plan,
    fonts: &FontStore,
    images: &ImageStore,
    opts: &EmitOptions,
    #[cfg(feature = "pdf-ua")] ua: Option<&ua::UaPlan>,
) {
    let cmyk = opts.cmyk;
    let fade = !pdf_a_on(opts);
    for (i, (page, (page_ref, content_ref))) in pages.iter().zip(&plan.page_refs).enumerate() {
        write_page_object(pdf, page, plan, (*page_ref, *content_ref), opts, i);
        #[cfg(feature = "pdf-ua")]
        let tags = ua.map(|ua| ua.page_tags(i));
        let bytes = content_stream(
            page,
            fonts,
            images,
            opts.watermark.as_ref(),
            cmyk,
            fade,
            #[cfg(feature = "pdf-ua")]
            tags.as_ref(),
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
    // A tagged render declares each page's `/StructParents` key; a flag-off
    // render omits it, so the default page object is byte-for-byte unchanged.
    #[cfg(feature = "pdf-ua")]
    if plan.pdf_ua {
        obj.struct_parents(page_idx as i32);
    }
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
    // PDF/A-2b forbids transparency, so a PDF/A render omits the watermark's
    // `/ca` fade `ExtGState` (the mark prints at full opacity instead). A
    // non-PDF/A render emits it exactly as before.
    if !pdf_a_on(opts) {
        if let Some(mark) = &opts.watermark {
            write_fade_gs(&mut resources, watermark::opacity(mark));
        }
    }
}

/// Write the watermark's `/GSwm` fade `ExtGState` inline (a simple `/ca` dict
/// needs no indirect object), referenced by the content stream's `gs` operator.
/// Skipped for a PDF/A render, which forbids the `/ca` transparency entirely.
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
