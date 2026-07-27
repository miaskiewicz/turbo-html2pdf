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
//! SVG support rides behind the off-by-default `svg` feature (Phase 15b). When
//! enabled, the gated arms in [`probe`]/[`decode`] (and the [`is_svg`] sniff)
//! rasterize `image/svg+xml` via `resvg`/`usvg` into a straight-alpha RGBA buffer
//! and slot a [`RasterImage`] in alongside the raster ones — the XObject path
//! already embeds RGBA via the alpha `SMask`, so no new emit code is needed (see
//! [`crate::svg`]). With the feature off, `resvg` is not a dependency and these
//! arms are not compiled, so the default decode path is byte-for-byte unchanged.

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
    /// GIF87a/GIF89a. Intrinsic-size only (header probe); not decoded for paint.
    Gif,
    /// RIFF/WebP (lossy `VP8 `, lossless `VP8L`, extended `VP8X`). Intrinsic-size
    /// only (header probe); not decoded for paint.
    WebP,
}

const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// GIF magic: either the `GIF87a` or `GIF89a` signature.
fn is_gif(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
}

/// RIFF/WebP magic: `RIFF` … `WEBP` with the FourCC at bytes 8..12.
fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP"
}

/// Sniff the encoded format from the leading magic bytes, or `None` if neither.
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(PNG_MAGIC) {
        return Some(Format::Png);
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(Format::Jpeg);
    }
    if is_gif(bytes) {
        return Some(Format::Gif);
    }
    if is_webp(bytes) {
        return Some(Format::WebP);
    }
    None
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
    // SVG has no fixed magic, so it is sniffed structurally (gated): an SVG box is
    // sized from its rasterized intrinsic dimensions before the raster sniff runs.
    #[cfg(feature = "svg")]
    if crate::svg::is_svg(bytes) {
        return crate::svg::probe_svg(bytes);
    }
    match sniff(bytes)? {
        Format::Png => probe_png(bytes),
        Format::Jpeg => probe_jpeg(bytes),
        Format::Gif => probe_gif(bytes),
        Format::WebP => probe_webp(bytes),
    }
}

/// GIF intrinsic size from the logical-screen descriptor: width/height are the two
/// little-endian `u16`s at bytes 6..10 (right after the 6-byte signature). No pixel
/// decode. `has_alpha` is left false — GIF transparency is per-frame and only
/// matters at emit, which this format doesn't take.
fn probe_gif(bytes: &[u8]) -> Option<Intrinsic> {
    if bytes.len() < 10 {
        return None;
    }
    let w = u16::from_le_bytes([bytes[6], bytes[7]]);
    let h = u16::from_le_bytes([bytes[8], bytes[9]]);
    (w > 0 && h > 0).then_some(Intrinsic {
        width: u32::from(w),
        height: u32::from(h),
        has_alpha: false,
    })
}

/// WebP intrinsic size across the three chunk layouts (lossy `VP8 `, lossless
/// `VP8L`, extended `VP8X`). Reads only the header dimensions — no pixel decode.
fn probe_webp(bytes: &[u8]) -> Option<Intrinsic> {
    // The chunk FourCC follows the 12-byte RIFF/WEBP header.
    let fourcc = bytes.get(12..16)?;
    let (w, h) = webp_size(fourcc, bytes)?;
    (w > 0 && h > 0).then_some(Intrinsic {
        width: w,
        height: h,
        has_alpha: fourcc != b"VP8 ", // lossless/extended may carry alpha
    })
}

/// Dispatch to the per-chunk size reader for a WebP FourCC; unknown chunks yield
/// `None`.
fn webp_size(fourcc: &[u8], bytes: &[u8]) -> Option<(u32, u32)> {
    match fourcc {
        b"VP8 " => webp_lossy_size(bytes),
        b"VP8L" => webp_lossless_size(bytes),
        b"VP8X" => webp_extended_size(bytes),
        _ => None,
    }
}

/// Lossy WebP: after the `VP8 ` chunk header (8 bytes) the VP8 key-frame carries a
/// 3-byte start code `9d 01 2a`, then 14-bit width and height (little-endian) at
/// bytes 26..30 of the file.
fn webp_lossy_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let d = bytes.get(20..30)?; // frame tag (3) + start code (3) + w/h (4)
    if d[3..6] != [0x9d, 0x01, 0x2a] {
        return None;
    }
    let w = u16::from_le_bytes([d[6], d[7]]) & 0x3fff;
    let h = u16::from_le_bytes([d[8], d[9]]) & 0x3fff;
    Some((u32::from(w), u32::from(h)))
}

/// Lossless WebP: after the `VP8L` chunk header the signature byte `0x2f` precedes
/// 14-bit `width-1` and `height-1` packed little-endian across the next 4 bytes.
fn webp_lossless_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let d = bytes.get(20..25)?; // signature (1) + 4 packed bytes
    if d[0] != 0x2f {
        return None;
    }
    let bits = u32::from_le_bytes([d[1], d[2], d[3], d[4]]);
    let w = (bits & 0x3fff) + 1;
    let h = ((bits >> 14) & 0x3fff) + 1;
    Some((w, h))
}

/// Extended WebP (`VP8X`): the canvas size is two 24-bit `value-1` little-endian
/// fields at bytes 24..30 (after the 4-byte flags of the VP8X chunk body).
fn webp_extended_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let d = bytes.get(24..30)?;
    let w = u32::from_le_bytes([d[0], d[1], d[2], 0]) + 1;
    let h = u32::from_le_bytes([d[3], d[4], d[5], 0]) + 1;
    Some((w, h))
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
    // SVG (gated): rasterize to RGBA before the raster sniff. The resulting
    // [`RasterImage`] feeds the same XObject/SMask path as a decoded RGBA PNG.
    #[cfg(feature = "svg")]
    if crate::svg::is_svg(bytes) {
        return crate::svg::decode_svg(bytes);
    }
    match sniff(bytes)? {
        Format::Png => decode_png(bytes),
        Format::Jpeg => decode_jpeg(bytes),
        // GIF/WebP are probed for intrinsic size (layout) but not yet pixel-decoded
        // for paint — an unsupported decode emits nothing rather than a wrong image.
        Format::Gif | Format::WebP => None,
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

#[cfg(test)]
mod format_tests {
    use super::*;

    #[test]
    fn sniff_recognizes_gif_and_webp() {
        assert_eq!(sniff(b"GIF89a\x00\x00\x00\x00\x00\x00"), Some(Format::Gif));
        assert_eq!(sniff(b"GIF87a\x00\x00\x00\x00\x00\x00"), Some(Format::Gif));
        assert_eq!(sniff(b"RIFF\x00\x00\x00\x00WEBPVP8 "), Some(Format::WebP));
        assert_eq!(sniff(b"not an image"), None);
    }

    #[test]
    fn probe_gif_reads_logical_screen_size() {
        // GIF89a header: signature, then width=120 (0x0078) and height=60 (0x003C) LE.
        let gif = b"GIF89a\x78\x00\x3c\x00\xf0\x00\x00";
        let intrinsic = probe(gif).expect("gif intrinsic");
        assert_eq!((intrinsic.width, intrinsic.height), (120, 60));
    }

    #[test]
    fn probe_webp_lossy_reads_frame_size() {
        // RIFF/WEBP + `VP8 ` chunk; 14-bit width=100, height=50 after the start code.
        let mut b = Vec::from(*b"RIFF\x00\x00\x00\x00WEBPVP8 \x00\x00\x00\x00");
        b.extend_from_slice(&[0x00, 0x00, 0x00]); // frame tag
        b.extend_from_slice(&[0x9d, 0x01, 0x2a]); // start code
        b.extend_from_slice(&[0x64, 0x00, 0x32, 0x00]); // w=100, h=50 (LE, 14-bit)
        let intrinsic = probe(&b).expect("webp intrinsic");
        assert_eq!((intrinsic.width, intrinsic.height), (100, 50));
    }

    #[test]
    fn probe_webp_lossless_reads_packed_size() {
        // `VP8L`: signature 0x2f then packed (width-1)|((height-1)<<14). w=8, h=6.
        let bits: u32 = (8 - 1) | ((6 - 1) << 14);
        let mut b = Vec::from(*b"RIFF\x00\x00\x00\x00WEBPVP8L\x00\x00\x00\x00");
        b.push(0x2f);
        b.extend_from_slice(&bits.to_le_bytes());
        let intrinsic = probe(&b).expect("webp lossless intrinsic");
        assert_eq!((intrinsic.width, intrinsic.height), (8, 6));
    }

    #[test]
    fn gif_is_probed_but_not_decoded() {
        // Intrinsic sizing works (layout) but the pixel decode is intentionally
        // unsupported (paint) — an unsupported decode emits nothing.
        let gif = b"GIF89a\x08\x00\x06\x00\xf0\x00\x00";
        assert!(probe(gif).is_some());
        assert!(decode(gif).is_none());
    }
}
