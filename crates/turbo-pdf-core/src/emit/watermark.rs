//! Page watermarks (§7, Phase 17): a faded mark painted *behind* the body on
//! every page, in one of two flavors.
//!
//! - [`TextWatermark`] shapes a word (default `DRAFT`, but any string —
//!   `BROUILLON`, `ENTWURF`, `WERSJA ROBOCZA`) with a caller-supplied
//!   [`FontFace`], registers its glyphs into the emitter's existing
//!   [`FontStore`] so they subset and embed exactly like body text, fades the
//!   fill through an `ExtGState` `/ca`, and rotates it about the page center by a
//!   fixed angle. The angle is a parameter, never the clock — output stays
//!   byte-deterministic (AC-7.6).
//! - [`ImageWatermark`] paints a raster (resolved by name through the *existing*
//!   [`ImageResolver`] / [`ImageStore`] raster path from Phase 9b) centered or
//!   tiled across the page, also faded by `/ca`.
//!
//! **Reuse, not reinvention.** This module owns no decode, no resolver, and no
//! image XObject writer: a text watermark rides the same [`FontStore`] glyph
//! subsetting as body text, and an image watermark rides the same
//! [`ImageStore`]/[`paint_image`] path as `<img>` content. It only adds the
//! placement (center / tile / rotate) and the fade.

use pdf_writer::{Content, Name};

use crate::image::ImageResolver;
use crate::layout::value::Rgba;
use crate::paginate::Page;
use crate::text::FontFace;

use super::color::device_rgb;
use super::fonts::FontStore;
use super::image::ImageStore;
use super::unit::px_to_pt;

/// The PDF resource name of the watermark's fade `ExtGState` (`/GSwm`). Distinct
/// from any name the rest of the emitter uses, so it never collides.
pub const FADE_GS_NAME: &str = "GSwm";

/// A page watermark: either shaped text or a resolved raster. Painted first in
/// every page's content stream, so the body draws on top of it.
#[derive(Debug, Clone)]
pub enum Watermark {
    /// A faded, rotated word (boxed: a [`FontFace`] is large relative to the
    /// image variant, so this keeps the enum small — see `clippy`'s
    /// `large_enum_variant`).
    Text(Box<TextWatermark>),
    /// A faded raster, centered or tiled.
    Image(ImageWatermark),
}

/// A faded diagonal word stamped behind the body (§7, Phase 17).
#[derive(Debug, Clone)]
pub struct TextWatermark {
    /// The word to stamp (default `DRAFT`; any string works).
    pub text: String,
    /// The face used to shape and embed the word.
    pub face: FontFace,
    /// Font size in CSS px (96 dpi), scaled to points like body text.
    pub font_size: f32,
    /// Fill color (the alpha channel is ignored here; fade is via `opacity`).
    pub color: Rgba,
    /// Fill opacity `0.0..=1.0`, applied through an `ExtGState` `/ca`.
    pub opacity: f32,
    /// Rotation about the page center, in degrees (counter-clockwise). A fixed
    /// parameter — never derived from the clock.
    pub angle_deg: f32,
}

impl TextWatermark {
    /// A `DRAFT` watermark in the given face: 25% opacity, gray, 64px, rotated
    /// 45° — the conventional faded-diagonal default.
    pub fn draft(face: FontFace) -> TextWatermark {
        TextWatermark {
            text: "DRAFT".to_string(),
            face,
            font_size: 64.0,
            color: Rgba::new(128, 128, 128, 255),
            opacity: 0.25,
            angle_deg: 45.0,
        }
    }
}

/// A raster watermark resolved by name through the shared [`ImageResolver`].
#[derive(Debug, Clone)]
pub struct ImageWatermark {
    /// The resolver name of the raster (PNG/JPEG bytes, decoded by the shared
    /// [`ImageStore`]).
    pub name: String,
    /// Fill opacity `0.0..=1.0`, applied through an `ExtGState` `/ca`.
    pub opacity: f32,
    /// When `true`, tile the raster across the whole page; otherwise center it.
    pub tiled: bool,
}

/// The fade opacity a watermark requests, if any. Drives whether the page
/// resources need the `/GSwm` `ExtGState`.
pub fn opacity(watermark: &Watermark) -> f32 {
    match watermark {
        Watermark::Text(t) => t.opacity,
        Watermark::Image(i) => i.opacity,
    }
}

/// Register a text watermark's shaped glyphs and an image watermark's raster
/// into the *existing* stores during the collect pass, so a text mark subsets +
/// embeds like body text and an image mark decodes via the shared raster path.
pub fn collect(
    watermark: &Watermark,
    fonts: &mut FontStore,
    images: &mut ImageStore,
    resolver: &dyn ImageResolver,
) {
    match watermark {
        Watermark::Text(text) => {
            let gids = shaped_gids(text);
            fonts.record_glyphs(&text.face, &gids);
        }
        Watermark::Image(image) => images.record(&image.name, resolver),
    }
}

/// The original glyph ids the word shapes to, in the watermark's face.
fn shaped_gids(text: &TextWatermark) -> Vec<u16> {
    text.face
        .shape(&text.text)
        .iter()
        .map(|g| g.glyph_id)
        .collect()
}

/// Paint the watermark into a page's content stream *first* (behind the body).
/// The fade `ExtGState` is set once around the whole mark.
pub fn paint(
    content: &mut Content,
    watermark: &Watermark,
    page: &Page,
    fonts: &FontStore,
    images: &ImageStore,
) {
    let page_w = px_to_pt(page.geometry.width);
    let page_h = px_to_pt(page.geometry.height);
    content.save_state();
    content.set_parameters(Name(FADE_GS_NAME.as_bytes()));
    match watermark {
        Watermark::Text(text) => paint_text(content, text, fonts, page_w, page_h),
        Watermark::Image(image) => paint_image(content, image, images, page_w, page_h),
    }
    content.restore_state();
}

// --------------------------------------------------------------------------
// text watermark
// --------------------------------------------------------------------------

/// Stamp the shaped word, rotated by `angle_deg` about the page center and
/// horizontally centered on its own advance width.
fn paint_text(
    content: &mut Content,
    text: &TextWatermark,
    fonts: &FontStore,
    page_w: f32,
    page_h: f32,
) {
    let glyphs = text.face.shape(&text.text);
    let face_index = fonts.index_of(&text.face);
    let size_pt = px_to_pt(text.font_size);
    let scale = size_pt / f32::from(text.face.units_per_em());
    let advance: f32 = glyphs.iter().map(|g| advance_pt(g.x_advance, scale)).sum();
    let rgb = device_rgb(text.color);

    // Rotate about the page center: translate to center, rotate, then place the
    // word's baseline so its advance is centered on the origin.
    content.transform(rotation_about(page_w / 2.0, page_h / 2.0, text.angle_deg));
    content.begin_text();
    content.set_font(
        Name(FontStore::resource_name(face_index).as_bytes()),
        size_pt,
    );
    content.set_fill_rgb(rgb.r, rgb.g, rgb.b);
    let mut pen = -advance / 2.0;
    for glyph in &glyphs {
        content.set_text_matrix([1.0, 0.0, 0.0, 1.0, pen, 0.0]);
        let code = fonts.remap(face_index, glyph.glyph_id);
        content.show(pdf_writer::Str(&code.to_be_bytes()));
        pen += advance_pt(glyph.x_advance, scale);
    }
    content.end_text();
}

/// One glyph's advance in points: design-unit advance times the units→points
/// scale (`x_advance` is `i32` design units, so it can't ride `f32::from`).
fn advance_pt(x_advance: i32, scale: f32) -> f32 {
    x_advance as f32 * scale
}

/// The affine matrix that rotates `angle_deg` degrees counter-clockwise about
/// the point `(cx, cy)` in PDF user space.
fn rotation_about(cx: f32, cy: f32, angle_deg: f32) -> [f32; 6] {
    let theta = angle_deg.to_radians();
    let (s, c) = theta.sin_cos();
    // T(cx,cy) * R(theta) * T(-cx,-cy), composed into one matrix.
    [c, s, -s, c, cx - c * cx + s * cy, cy - s * cx - c * cy]
}

// --------------------------------------------------------------------------
// image watermark
// --------------------------------------------------------------------------

/// Paint the raster watermark: centered at its decoded pixel size, or tiled to
/// cover the page. Unresolvable/undecodable names paint nothing.
fn paint_image(
    content: &mut Content,
    image: &ImageWatermark,
    images: &ImageStore,
    page_w: f32,
    page_h: f32,
) {
    let Some((index, w, h)) = images.placement(&image.name) else {
        return;
    };
    let resource = ImageStore::resource_name(index);
    let (w_pt, h_pt) = (px_to_pt(w as f32), px_to_pt(h as f32));
    if image.tiled {
        paint_tiles(content, &resource, w_pt, h_pt, page_w, page_h);
    } else {
        let x = (page_w - w_pt) / 2.0;
        let y = (page_h - h_pt) / 2.0;
        draw_xobject(content, &resource, w_pt, h_pt, x, y);
    }
}

/// Tile the raster from the bottom-left, covering the whole page (the final
/// row/column may overhang; the page box clips it).
fn paint_tiles(
    content: &mut Content,
    resource: &str,
    w_pt: f32,
    h_pt: f32,
    page_w: f32,
    page_h: f32,
) {
    let mut y = 0.0;
    while y < page_h {
        let mut x = 0.0;
        while x < page_w {
            draw_xobject(content, resource, w_pt, h_pt, x, y);
            x += w_pt;
        }
        y += h_pt;
    }
}

/// Draw one image XObject scaled to `(w_pt, h_pt)` with its bottom-left at
/// `(x, y)`, isolating the placement transform in its own state.
fn draw_xobject(content: &mut Content, resource: &str, w_pt: f32, h_pt: f32, x: f32, y: f32) {
    content.save_state();
    content.transform([w_pt, 0.0, 0.0, h_pt, x, y]);
    content.x_object(Name(resource.as_bytes()));
    content.restore_state();
}
