//! Raster image ingestion (§7.4, Phase 9b): the caller-supplied [`ImageResolver`]
//! and the decode that turns its PNG/JPEG bytes into the pixel data the PDF
//! emitter embeds as image XObjects.
//!
//! **No I/O (§0.2).** This module never touches the network or the filesystem.
//! Every image is named in the template (`<img src>` / `background-image`) and
//! the bytes for that name are produced by a caller-supplied resolver. A render
//! with no resolver simply paints no images (the [`NoImages`] default).
//!
//! **Layout vs. emit.** Layout needs only the *intrinsic* pixel size to size the
//! box and apply the overflow caps; the emitter needs the full pixel payload. The
//! same resolver answers both: [`probe`] reads the header for `(w, h)` cheaply,
//! [`decode`] produces the embeddable [`RasterImage`]. PNG is decoded to 8-bit
//! RGB(A) (alpha split off as an SMask); JPEG is passed through verbatim as
//! `DCTDecode` (§7.4: "JPEG passed through where possible").
//!
//! SVG is out of scope here. // TODO(phase15): a `feature = "svg"` arm rasterizes
//! `image/svg+xml` via `resvg` and slots a [`RasterImage`] in alongside these.

use std::io::Cursor;

/// Caller-supplied image source (§0.2): maps a template image name (an `<img>`
/// `src` or a `background-image` `url(...)`) to its encoded bytes. The engine
/// never fetches; everything the document shows comes from here.
///
/// Implemented by the host (a `HashMap`, a CMS lookup, an embedded asset table).
/// A render that supplies no resolver uses [`NoImages`], which resolves nothing.
pub trait ImageResolver {
    /// The encoded bytes (PNG or JPEG) for `name`, or `None` if unknown. A
    /// `None` result lets the image lay out at zero intrinsic size and emit
    /// nothing, so an unresolved reference degrades gracefully.
    fn resolve(&self, name: &str) -> Option<&[u8]>;
}

/// The default resolver: every lookup misses, so no images are embedded. Lets
/// the layout and emit entry points keep a zero-config signature.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoImages;

impl ImageResolver for NoImages {
    fn resolve(&self, _name: &str) -> Option<&[u8]> {
        None
    }
}

/// The container format of an encoded image, decided by its magic bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
}

/// Sniff the encoded format from the leading magic bytes, or `None` if neither.
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some(Format::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Format::Jpeg)
    } else {
        None
    }
}

/// The intrinsic pixel size of an encoded image plus whether it has an alpha
/// channel, all read from the header without decoding pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Intrinsic {
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
}

/// Read just the intrinsic size + alpha flag from encoded bytes, without
/// decoding pixels. Used at layout time to size the image box and apply the
/// overflow caps.
pub fn probe(bytes: &[u8]) -> Option<Intrinsic> {
    match sniff(bytes)? {
        Format::Png => probe_png(bytes),
        Format::Jpeg => probe_jpeg(bytes),
    }
}

fn probe_png(bytes: &[u8]) -> Option<Intrinsic> {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let reader = decoder.read_info().ok()?;
    let info = reader.info();
    Some(Intrinsic {
        width: info.width,
        height: info.height,
        has_alpha: png_has_alpha(info.color_type, info.trns.is_some()),
    })
}

/// Whether a PNG carries transparency: a color type with an alpha channel, or a
/// `tRNS` chunk (which `normalize_to_color8` expands into one on decode).
fn png_has_alpha(color: png::ColorType, has_trns: bool) -> bool {
    matches!(color, png::ColorType::GrayscaleAlpha | png::ColorType::Rgba) || has_trns
}

fn probe_jpeg(bytes: &[u8]) -> Option<Intrinsic> {
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    decoder.read_info().ok()?;
    let info = decoder.info()?;
    Some(Intrinsic {
        width: u32::from(info.width),
        height: u32::from(info.height),
        has_alpha: false,
    })
}

/// The PDF color space an image's samples live in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    /// 1 sample/pixel grayscale (`DeviceGray`).
    Gray,
    /// 3 samples/pixel (`DeviceRGB`).
    Rgb,
}

impl ColorSpace {
    /// Samples per pixel in this color space.
    pub fn components(self) -> usize {
        match self {
            ColorSpace::Gray => 1,
            ColorSpace::Rgb => 3,
        }
    }
}

/// How an image's pixels reach the PDF: PNG is re-encoded as raw samples (the
/// emitter Flate-compresses the stream); JPEG rides through untouched as a
/// `DCTDecode` stream (§7.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    /// Raw 8-bit samples in `color`, row-major, no padding.
    Raw { samples: Vec<u8>, color: ColorSpace },
    /// The original JPEG bytes, embedded as a `DCTDecode` stream.
    Jpeg { bytes: Vec<u8>, color: ColorSpace },
}

/// A decoded image ready to embed: its pixel size, the color payload, and the
/// optional 8-bit alpha plane that becomes the XObject's `SMask` (§7.4 AC-7.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterImage {
    pub width: u32,
    pub height: u32,
    pub payload: Payload,
    /// One alpha byte per pixel, row-major, when the source had transparency.
    pub alpha: Option<Vec<u8>>,
}

impl RasterImage {
    /// The image's color space (RGB or gray), regardless of payload encoding.
    pub fn color(&self) -> ColorSpace {
        match &self.payload {
            Payload::Raw { color, .. } | Payload::Jpeg { color, .. } => *color,
        }
    }
}

/// Decode encoded bytes into an embeddable [`RasterImage`], or `None` if the
/// format is unrecognized or the data is malformed.
pub fn decode(bytes: &[u8]) -> Option<RasterImage> {
    match sniff(bytes)? {
        Format::Png => decode_png(bytes),
        Format::Jpeg => decode_jpeg(bytes),
    }
}

fn decode_png(bytes: &[u8]) -> Option<RasterImage> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let out = reader.next_frame(&mut buf).ok()?;
    buf.truncate(out.buffer_size());
    Some(png_to_image(out.width, out.height, out.color_type, &buf))
}

/// Split a normalized 8-bit PNG frame into an RGB/gray payload plus an optional
/// alpha plane. `normalize_to_color8` already expanded palette/low-bit/16-bit
/// inputs, so only the four 8-bit `ColorType`s reach here.
fn png_to_image(width: u32, height: u32, color: png::ColorType, buf: &[u8]) -> RasterImage {
    let (color_space, channels, alpha_idx) = png_layout(color);
    let pixels = (width * height) as usize;
    let comps = color_space.components();
    let mut samples = Vec::with_capacity(pixels * comps);
    let mut alpha = alpha_idx.map(|_| Vec::with_capacity(pixels));
    for px in buf.chunks_exact(channels) {
        samples.extend_from_slice(&px[..comps]);
        if let (Some(a), Some(i)) = (alpha.as_mut(), alpha_idx) {
            a.push(px[i]);
        }
    }
    RasterImage {
        width,
        height,
        payload: Payload::Raw {
            samples,
            color: color_space,
        },
        alpha,
    }
}

/// The `(color space, channels per pixel, alpha-channel index)` for one
/// normalized PNG color type.
fn png_layout(color: png::ColorType) -> (ColorSpace, usize, Option<usize>) {
    match color {
        png::ColorType::Grayscale => (ColorSpace::Gray, 1, None),
        png::ColorType::GrayscaleAlpha => (ColorSpace::Gray, 2, Some(1)),
        png::ColorType::Rgb => (ColorSpace::Rgb, 3, None),
        // `normalize_to_color8` expands Indexed to Rgb/Rgba, so any remaining
        // case is RGBA.
        _ => (ColorSpace::Rgb, 4, Some(3)),
    }
}

fn decode_jpeg(bytes: &[u8]) -> Option<RasterImage> {
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    decoder.read_info().ok()?;
    let info = decoder.info()?;
    let color = jpeg_color(info.pixel_format)?;
    Some(RasterImage {
        width: u32::from(info.width),
        height: u32::from(info.height),
        payload: Payload::Jpeg {
            bytes: bytes.to_vec(),
            color,
        },
        alpha: None,
    })
}

/// Map a baseline-JPEG pixel format to a PDF color space. CMYK JPEGs are not
/// passed through in v1 (they need an `/Decode` inversion); they decode to
/// `None` and emit nothing.
fn jpeg_color(format: jpeg_decoder::PixelFormat) -> Option<ColorSpace> {
    match format {
        jpeg_decoder::PixelFormat::L8 | jpeg_decoder::PixelFormat::L16 => Some(ColorSpace::Gray),
        jpeg_decoder::PixelFormat::RGB24 => Some(ColorSpace::Rgb),
        jpeg_decoder::PixelFormat::CMYK32 => None,
    }
}
