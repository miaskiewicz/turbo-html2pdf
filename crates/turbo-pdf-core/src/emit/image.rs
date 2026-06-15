//! Image XObject embedding (§7.4, Phase 9b, AC-7.4). Every `Image` fragment names
//! a raster the caller's [`ImageResolver`] supplies; this module decodes each
//! distinct name once (PNG to raw RGB/gray samples, JPEG passed through as
//! `DCTDecode`), embeds it as an image XObject, attaches an `SMask` when the
//! source had alpha, and paints it with a placement transform.
//!
//! **Resources.** Like fonts, images are collected across all pages in
//! first-encounter order and written into each page's `/XObject` resource dict
//! under a stable name (`Im0`, `Im1`, …), so output stays deterministic
//! (AC-7.6). A name the resolver can't supply (or can't decode) is dropped: it
//! occupies no object and paints nothing.

use pdf_writer::{Chunk, Filter, Finish, Name, Ref};

use crate::image::{decode, ColorSpace, ImageResolver, Payload, RasterImage};
use crate::layout::fragment::{Fragment, FragmentContent, ImagePlacement};

use super::fonts::RefAlloc;
use super::unit::{flip_y, px_to_pt};

/// One embedded image: the resolver name it was collected under and its decoded
/// pixels. Kept in first-encounter order for deterministic resource layout.
struct UsedImage {
    name: String,
    image: RasterImage,
}

/// Every distinct raster embedded across the document, decoded once. Built by
/// walking each page's `Image` fragments through the resolver; consumed by
/// [`ImageStore::write`] to emit the XObjects and by the painter to resolve a
/// placement to its resource name.
#[derive(Default)]
pub struct ImageStore {
    images: Vec<UsedImage>,
}

impl ImageStore {
    /// Collect and decode every resolvable image used across `pages`.
    pub fn collect(pages: &[crate::paginate::Page], resolver: &dyn ImageResolver) -> ImageStore {
        let mut store = ImageStore::default();
        for page in pages {
            for band in bands(page) {
                for frag in band {
                    store.collect_fragment(frag, resolver);
                }
            }
        }
        store
    }

    fn collect_fragment(&mut self, frag: &Fragment, resolver: &dyn ImageResolver) {
        if let FragmentContent::Image(placement) = &frag.content {
            self.record(&placement.name, resolver);
        }
        for child in &frag.children {
            self.collect_fragment(child, resolver);
        }
    }

    /// Decode and store `name` if it resolves, is decodable, and is new. Public
    /// so watermark collection can register a raster the same way (§7, Phase 17).
    pub fn record(&mut self, name: &str, resolver: &dyn ImageResolver) {
        if self.images.iter().any(|u| u.name == name) {
            return;
        }
        let Some(bytes) = resolver.resolve(name) else {
            return;
        };
        let Some(image) = decode(bytes) else {
            return;
        };
        self.images.push(UsedImage {
            name: name.to_string(),
            image,
        });
    }

    /// The index of a placement's image, if it was collected (resolved + decoded).
    fn index_of(&self, name: &str) -> Option<usize> {
        self.images.iter().position(|u| u.name == name)
    }

    /// The PDF resource name for the `n`th image (`Im0`, `Im1`, …).
    pub fn resource_name(n: usize) -> String {
        format!("Im{n}")
    }

    /// A resolved image's `(resource index, pixel width, pixel height)`, or
    /// `None` if the name was never collected. Used to place a watermark raster
    /// at its decoded size (§7, Phase 17).
    pub fn placement(&self, name: &str) -> Option<(usize, u32, u32)> {
        let index = self.index_of(name)?;
        let image = &self.images[index].image;
        Some((index, image.width, image.height))
    }

    /// The number of distinct embedded images.
    pub fn len(&self) -> usize {
        self.images.len()
    }

    /// Whether no image was embedded.
    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }

    /// The number of PDF objects each image occupies: the XObject plus an SMask
    /// when it has alpha.
    fn object_count(image: &RasterImage) -> i32 {
        if image.alpha.is_some() {
            2
        } else {
            1
        }
    }

    /// Total PDF objects all images occupy (used to lay out the object plan).
    pub fn total_objects(&self) -> i32 {
        self.images
            .iter()
            .map(|u| Self::object_count(&u.image))
            .sum()
    }

    /// The main XObject ref of each image, in resource order, given the first
    /// image object id. Mirrors the allocation order [`ImageStore::write`] uses.
    pub fn xobject_refs(&self, start: i32) -> Vec<Ref> {
        let mut alloc = RefAlloc::new(start);
        self.images
            .iter()
            .map(|u| {
                let main = alloc.bump();
                if u.image.alpha.is_some() {
                    alloc.bump();
                }
                main
            })
            .collect()
    }

    /// Write every collected image (and its SMask) into `chunk`, allocating
    /// object ids from `alloc` in resource order.
    pub fn write(&self, chunk: &mut Chunk, alloc: &mut RefAlloc) {
        for used in &self.images {
            write_image(chunk, &used.image, alloc);
        }
    }
}

/// The four paint bands of a page, back to front.
fn bands(page: &crate::paginate::Page) -> [&[Fragment]; 4] {
    [&page.body, &page.header, &page.footer, &page.footnotes]
}

/// Write one image: its main XObject (with an `SMask` reference when present),
/// then the SMask stream itself.
fn write_image(chunk: &mut Chunk, image: &RasterImage, alloc: &mut RefAlloc) {
    let main = alloc.bump();
    let smask = image.alpha.as_ref().map(|_| alloc.bump());
    write_xobject(chunk, main, image, smask);
    if let (Some(smask_ref), Some(alpha)) = (smask, image.alpha.as_ref()) {
        write_smask(chunk, smask_ref, image, alpha);
    }
}

/// Write the main color XObject. PNG samples ride a `FlateDecode`-free raw
/// stream; JPEG bytes ride through as `DCTDecode`.
fn write_xobject(chunk: &mut Chunk, id: Ref, image: &RasterImage, smask: Option<Ref>) {
    let (data, filter) = stream_data(image);
    let mut xobj = chunk.image_xobject(id, data);
    xobj.width(image.width as i32);
    xobj.height(image.height as i32);
    if let Some(f) = filter {
        xobj.filter(f);
    }
    set_color_space(&mut xobj, image.color());
    xobj.bits_per_component(8);
    if let Some(s) = smask {
        xobj.s_mask(s);
    }
    xobj.finish();
}

/// The encoded stream bytes and optional filter for an image's color payload.
fn stream_data(image: &RasterImage) -> (&[u8], Option<Filter>) {
    match &image.payload {
        Payload::Raw { samples, .. } => (samples, None),
        Payload::Jpeg { bytes, .. } => (bytes, Some(Filter::DctDecode)),
    }
}

fn set_color_space(xobj: &mut pdf_writer::writers::ImageXObject, color: ColorSpace) {
    match color {
        ColorSpace::Gray => xobj.color_space().device_gray(),
        ColorSpace::Rgb => xobj.color_space().device_rgb(),
    }
}

/// Write the alpha plane as a `DeviceGray`, 8-bit image XObject that the color
/// image references through `/SMask` (§7.4 AC-7.4).
fn write_smask(chunk: &mut Chunk, id: Ref, image: &RasterImage, alpha: &[u8]) {
    let mut mask = chunk.image_xobject(id, alpha);
    mask.width(image.width as i32);
    mask.height(image.height as i32);
    mask.color_space().device_gray();
    mask.bits_per_component(8);
    mask.finish();
}

// --------------------------------------------------------------------------
// painting
// --------------------------------------------------------------------------

use pdf_writer::Content;

/// Paint an `Image` fragment: select its XObject resource and draw it under a
/// placement transform that maps the unit image square to the fragment's box
/// (respecting the PDF y-flip, like text and graphics).
pub fn paint_image(
    content: &mut Content,
    frag: &Fragment,
    placement: &ImagePlacement,
    store: &ImageStore,
    page_height_pt: f32,
) {
    let Some(index) = store.index_of(&placement.name) else {
        return;
    };
    let resource = ImageStore::resource_name(index);
    let w = px_to_pt(frag.width);
    let h = px_to_pt(frag.height);
    let x = px_to_pt(frag.x);
    // The image's unit square has its origin at the bottom-left; the fragment's
    // top edge in galley space is `frag.y`, so its PDF bottom is the flipped
    // bottom edge.
    let y = flip_y(frag.y + frag.height, page_height_pt);
    content.save_state();
    content.transform([w, 0.0, 0.0, h, x, y]);
    content.x_object(Name(resource.as_bytes()));
    content.restore_state();
}
