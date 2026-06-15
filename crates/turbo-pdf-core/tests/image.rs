//! Phase 9b raster-image tests (§7.4, AC-7.4). Covers the decode layer
//! (`image::{sniff, probe, decode}` for PNG RGB/RGBA/gray + JPEG, plus the
//! malformed-input paths), the layout-time size caps (100% width, 60% body
//! height, aspect-ratio preservation), and the full layout → emit path (image
//! XObject, alpha → SMask, JPEG → DCTDecode, the no-image default, determinism,
//! and a `qpdf --check` when available).
//!
//! All test images are generated in-process (PNG via the `png` encoder; one tiny
//! committed baseline-JPEG byte array) so no external/system files are touched.

mod common;

use std::collections::HashMap;
use std::io::Cursor;

use turbo_pdf_core::image::{decode, probe, sniff, ColorSpace, Format, Payload};
use turbo_pdf_core::layout::boxgen::build_box_tree;
use turbo_pdf_core::layout::imgsize::{size_replaced, SizeCtx};
use turbo_pdf_core::layout::value::{resolve_box_style, ResolveCtx};
use turbo_pdf_core::paginate::{paginate, Page, PageGeometry};
use turbo_pdf_core::style::TokenSet;
use turbo_pdf_core::{
    build_cascade, emit_pdf, emit_pdf_with_images, layout_with_images, style_tree, ComputedStyle,
    Diagnostics, EmitOptions, ImageResolver, Node, StyledElement, StyledNode, Tag,
};
use turbo_pdf_core::{Attr, FragmentContent, ImageCtx, NoImages};

// --------------------------------------------------------------------------
// test image generation
// --------------------------------------------------------------------------

/// Encode an RGB (or RGBA, or gray) PNG of the given size from raw samples.
fn make_png(width: u32, height: u32, color: png::ColorType, samples: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut out), width, height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(samples).expect("png data");
    }
    out
}

/// A 2x2 solid-red RGB PNG.
fn png_rgb_2x2() -> Vec<u8> {
    let px = [255u8, 0, 0];
    let samples: Vec<u8> = px.iter().cycle().take(2 * 2 * 3).copied().collect();
    make_png(2, 2, png::ColorType::Rgb, &samples)
}

/// A 2x2 RGBA PNG with a varying alpha channel (so it needs an SMask).
fn png_rgba_2x2() -> Vec<u8> {
    let samples = [
        10, 20, 30, 0, // top-left, fully transparent
        40, 50, 60, 128, // top-right, half
        70, 80, 90, 200, //
        100, 110, 120, 255, // opaque
    ];
    make_png(2, 2, png::ColorType::Rgba, &samples)
}

/// A 2x2 grayscale PNG.
fn png_gray_2x2() -> Vec<u8> {
    make_png(2, 2, png::ColorType::Grayscale, &[10, 20, 30, 40])
}

/// A 2x2 grayscale+alpha PNG (gray sample then alpha sample per pixel).
fn png_gray_alpha_2x2() -> Vec<u8> {
    make_png(
        2,
        2,
        png::ColorType::GrayscaleAlpha,
        &[10, 0, 20, 128, 30, 200, 40, 255],
    )
}

/// A wide PNG: 400x100 RGB (used to exercise the width cap).
fn png_wide() -> Vec<u8> {
    let samples = vec![128u8; (400 * 100 * 3) as usize];
    make_png(400, 100, png::ColorType::Rgb, &samples)
}

/// A tall PNG: 100x4000 RGB (used to exercise the 60% height cap).
fn png_tall() -> Vec<u8> {
    let samples = vec![64u8; (100 * 4000 * 3) as usize];
    make_png(100, 4000, png::ColorType::Rgb, &samples)
}

/// A tiny valid 2x2 baseline RGB JPEG (committed bytes; see module docs).
const JPEG_RGB_2X2: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x06, 0x04, 0x05, 0x06, 0x05, 0x04, 0x06,
    0x06, 0x05, 0x06, 0x07, 0x07, 0x06, 0x08, 0x0A, 0x10, 0x0A, 0x0A, 0x09, 0x09, 0x0A, 0x14, 0x0E,
    0x0F, 0x0C, 0x10, 0x17, 0x14, 0x18, 0x18, 0x17, 0x14, 0x16, 0x16, 0x1A, 0x1D, 0x25, 0x1F, 0x1A,
    0x1B, 0x23, 0x1C, 0x16, 0x16, 0x20, 0x2C, 0x20, 0x23, 0x26, 0x27, 0x29, 0x2A, 0x29, 0x19, 0x1F,
    0x2D, 0x30, 0x2D, 0x28, 0x30, 0x25, 0x28, 0x29, 0x28, 0xFF, 0xDB, 0x00, 0x43, 0x01, 0x07, 0x07,
    0x07, 0x0A, 0x08, 0x0A, 0x13, 0x0A, 0x0A, 0x13, 0x28, 0x1A, 0x16, 0x1A, 0x28, 0x28, 0x28, 0x28,
    0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28,
    0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28,
    0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0x28, 0xFF, 0xC0,
    0x00, 0x11, 0x08, 0x00, 0x02, 0x00, 0x02, 0x03, 0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11,
    0x01, 0xFF, 0xC4, 0x00, 0x1F, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09,
    0x0A, 0x0B, 0xFF, 0xC4, 0x00, 0xB5, 0x10, 0x00, 0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05,
    0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7D, 0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21,
    0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08, 0x23,
    0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16, 0x17,
    0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A,
    0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A,
    0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A,
    0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99,
    0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7,
    0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5,
    0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1,
    0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFF, 0xC4, 0x00, 0x1F, 0x01, 0x00, 0x03,
    0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
    0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0xFF, 0xC4, 0x00, 0xB5, 0x11, 0x00,
    0x02, 0x01, 0x02, 0x04, 0x04, 0x03, 0x04, 0x07, 0x05, 0x04, 0x04, 0x00, 0x01, 0x02, 0x77, 0x00,
    0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13,
    0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xA1, 0xB1, 0xC1, 0x09, 0x23, 0x33, 0x52, 0xF0, 0x15,
    0x62, 0x72, 0xD1, 0x0A, 0x16, 0x24, 0x34, 0xE1, 0x25, 0xF1, 0x17, 0x18, 0x19, 0x1A, 0x26, 0x27,
    0x28, 0x29, 0x2A, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88,
    0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6,
    0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4,
    0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE2,
    0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9,
    0xFA, 0xFF, 0xDA, 0x00, 0x0C, 0x03, 0x01, 0x00, 0x02, 0x11, 0x03, 0x11, 0x00, 0x3F, 0x00, 0xF1,
    0x8D, 0x6F, 0x55, 0xD4, 0x60, 0xD6, 0x6F, 0xE2, 0x86, 0xFE, 0xEE, 0x38, 0xA3, 0xB8, 0x91, 0x51,
    0x12, 0x66, 0x0A, 0xA0, 0x31, 0x00, 0x00, 0x0F, 0x02, 0x8A, 0x28, 0xAF, 0xAB, 0xC3, 0xFF, 0x00,
    0x0A, 0x1E, 0x8B, 0xF2, 0x3D, 0x8A, 0xFF, 0x00, 0xC5, 0x97, 0xAB, 0xFC, 0xCF, 0xFF, 0xD9,
];

/// A tiny valid 2x2 baseline grayscale JPEG (committed bytes).
const JPEG_GRAY_2X2: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x0A, 0x07, 0x07, 0x08, 0x07, 0x06, 0x0A,
    0x08, 0x08, 0x08, 0x0B, 0x0A, 0x0A, 0x0B, 0x0E, 0x18, 0x10, 0x0E, 0x0D, 0x0D, 0x0E, 0x1D, 0x15,
    0x16, 0x11, 0x18, 0x23, 0x1F, 0x25, 0x24, 0x22, 0x1F, 0x22, 0x21, 0x26, 0x2B, 0x37, 0x2F, 0x26,
    0x29, 0x34, 0x29, 0x21, 0x22, 0x30, 0x41, 0x31, 0x34, 0x39, 0x3B, 0x3E, 0x3E, 0x3E, 0x25, 0x2E,
    0x44, 0x49, 0x43, 0x3C, 0x48, 0x37, 0x3D, 0x3E, 0x3B, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x02,
    0x00, 0x02, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xC4, 0x00, 0x1F, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01,
    0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04,
    0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0xFF, 0xC4, 0x00, 0xB5, 0x10, 0x00, 0x02, 0x01, 0x03,
    0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7D, 0x01, 0x02, 0x03, 0x00,
    0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32,
    0x81, 0x91, 0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72,
    0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35,
    0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55,
    0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75,
    0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94,
    0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2,
    0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9,
    0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6,
    0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFF, 0xDA,
    0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0x2B, 0xFF, 0xD9,
];

/// A tiny valid 2x2 CMYK (Adobe) JPEG. Not supported for pass-through (v1 has no
/// CMYK inversion), so it must decode to `None` and embed nothing.
const JPEG_CMYK_2X2: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xEE, 0x00, 0x0E, 0x41, 0x64, 0x6F, 0x62, 0x65, 0x00, 0x64, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x0A, 0x07, 0x07, 0x08, 0x07, 0x06, 0x0A, 0x08, 0x08,
    0x08, 0x0B, 0x0A, 0x0A, 0x0B, 0x0E, 0x18, 0x10, 0x0E, 0x0D, 0x0D, 0x0E, 0x1D, 0x15, 0x16, 0x11,
    0x18, 0x23, 0x1F, 0x25, 0x24, 0x22, 0x1F, 0x22, 0x21, 0x26, 0x2B, 0x37, 0x2F, 0x26, 0x29, 0x34,
    0x29, 0x21, 0x22, 0x30, 0x41, 0x31, 0x34, 0x39, 0x3B, 0x3E, 0x3E, 0x3E, 0x25, 0x2E, 0x44, 0x49,
    0x43, 0x3C, 0x48, 0x37, 0x3D, 0x3E, 0x3B, 0xFF, 0xC0, 0x00, 0x14, 0x08, 0x00, 0x02, 0x00, 0x02,
    0x04, 0x43, 0x11, 0x00, 0x4D, 0x11, 0x00, 0x59, 0x11, 0x00, 0x4B, 0x11, 0x00, 0xFF, 0xC4, 0x00,
    0x1F, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0xFF, 0xC4,
    0x00, 0xB5, 0x10, 0x00, 0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, 0x00,
    0x00, 0x01, 0x7D, 0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13,
    0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, 0x15,
    0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25,
    0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46,
    0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66,
    0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86,
    0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4,
    0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2,
    0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9,
    0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5,
    0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFF, 0xDA, 0x00, 0x0E, 0x04, 0x43, 0x00, 0x4D, 0x00, 0x59, 0x00,
    0x4B, 0x00, 0x00, 0x3F, 0x00, 0xF5, 0xEA, 0xF5, 0x6A, 0xF4, 0xEA, 0xF4, 0x6A, 0xFF, 0xD9,
];

/// A by-name image resolver backed by a map.
struct MapResolver {
    map: HashMap<String, Vec<u8>>,
}

impl MapResolver {
    fn new(pairs: Vec<(&str, Vec<u8>)>) -> MapResolver {
        MapResolver {
            map: pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        }
    }
}

impl ImageResolver for MapResolver {
    fn resolve(&self, name: &str) -> Option<&[u8]> {
        self.map.get(name).map(Vec::as_slice)
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

// --------------------------------------------------------------------------
// decode-layer tests
// --------------------------------------------------------------------------

#[test]
fn sniff_recognizes_png_jpeg_and_rejects_others() {
    assert_eq!(sniff(&png_rgb_2x2()), Some(Format::Png));
    assert_eq!(sniff(JPEG_RGB_2X2), Some(Format::Jpeg));
    assert_eq!(sniff(b"not an image"), None);
    assert_eq!(sniff(&[]), None);
}

#[test]
fn probe_reads_dimensions_and_alpha() {
    let rgb = probe(&png_rgb_2x2()).expect("probe rgb");
    assert_eq!((rgb.width, rgb.height, rgb.has_alpha), (2, 2, false));

    let rgba = probe(&png_rgba_2x2()).expect("probe rgba");
    assert!(rgba.has_alpha, "rgba png must report alpha");

    let gray = probe(&png_gray_2x2()).expect("probe gray");
    assert_eq!((gray.width, gray.height, gray.has_alpha), (2, 2, false));

    let jpeg = probe(JPEG_RGB_2X2).expect("probe jpeg");
    assert_eq!((jpeg.width, jpeg.height, jpeg.has_alpha), (2, 2, false));

    assert!(probe(b"garbage").is_none());
}

#[test]
fn decode_png_rgb_to_raw_samples() {
    let img = decode(&png_rgb_2x2()).expect("decode rgb");
    assert_eq!((img.width, img.height), (2, 2));
    assert_eq!(img.color(), ColorSpace::Rgb);
    assert!(img.alpha.is_none());
    match img.payload {
        Payload::Raw { samples, color } => {
            assert_eq!(color, ColorSpace::Rgb);
            assert_eq!(samples.len(), 2 * 2 * 3);
            assert_eq!(&samples[0..3], &[255, 0, 0]);
        }
        Payload::Jpeg { .. } => panic!("png must decode to raw samples"),
    }
}

#[test]
fn decode_png_rgba_splits_alpha_into_smask_plane() {
    let img = decode(&png_rgba_2x2()).expect("decode rgba");
    assert_eq!(img.color(), ColorSpace::Rgb);
    let alpha = img.alpha.expect("rgba must yield an alpha plane");
    assert_eq!(alpha, vec![0, 128, 200, 255]);
    match img.payload {
        Payload::Raw { samples, .. } => {
            // Color samples have alpha stripped: 3 components per pixel.
            assert_eq!(samples.len(), 2 * 2 * 3);
            assert_eq!(&samples[0..3], &[10, 20, 30]);
        }
        Payload::Jpeg { .. } => panic!("png raw expected"),
    }
}

#[test]
fn decode_png_grayscale_keeps_one_component() {
    let img = decode(&png_gray_2x2()).expect("decode gray");
    assert_eq!(img.color(), ColorSpace::Gray);
    match img.payload {
        Payload::Raw { samples, color } => {
            assert_eq!(color, ColorSpace::Gray);
            assert_eq!(samples, vec![10, 20, 30, 40]);
        }
        Payload::Jpeg { .. } => panic!("png raw expected"),
    }
}

#[test]
fn decode_jpeg_passes_through_as_dct() {
    let img = decode(JPEG_RGB_2X2).expect("decode jpeg");
    assert_eq!((img.width, img.height), (2, 2));
    assert_eq!(img.color(), ColorSpace::Rgb);
    assert!(img.alpha.is_none());
    match img.payload {
        Payload::Jpeg { bytes, color } => {
            assert_eq!(color, ColorSpace::Rgb);
            assert_eq!(bytes, JPEG_RGB_2X2, "jpeg bytes pass through unchanged");
        }
        Payload::Raw { .. } => panic!("jpeg must pass through, not re-encode"),
    }
}

#[test]
fn decode_png_grayscale_alpha_splits_plane() {
    let img = decode(&png_gray_alpha_2x2()).expect("decode gray+alpha");
    assert_eq!(img.color(), ColorSpace::Gray);
    let alpha = img.alpha.expect("gray+alpha must yield a plane");
    assert_eq!(alpha, vec![0, 128, 200, 255]);
    match img.payload {
        Payload::Raw { samples, color } => {
            assert_eq!(color, ColorSpace::Gray);
            assert_eq!(
                samples,
                vec![10, 20, 30, 40],
                "gray samples, alpha stripped"
            );
        }
        Payload::Jpeg { .. } => panic!("png raw expected"),
    }
    // The probe header path also reports alpha for gray+alpha.
    assert!(probe(&png_gray_alpha_2x2()).unwrap().has_alpha);
}

#[test]
fn decode_grayscale_jpeg_is_device_gray_passthrough() {
    let img = decode(JPEG_GRAY_2X2).expect("decode gray jpeg");
    assert_eq!(img.color(), ColorSpace::Gray);
    assert!(matches!(img.payload, Payload::Jpeg { .. }));
}

#[test]
fn decode_cmyk_jpeg_is_unsupported() {
    // CMYK JPEG decodes (header is valid) but has no v1 pass-through color space.
    assert!(
        decode(JPEG_CMYK_2X2).is_none(),
        "cmyk jpeg is not embeddable in v1"
    );
    // Its intrinsic size still probes (alpha false).
    let p = probe(JPEG_CMYK_2X2).expect("cmyk probe");
    assert_eq!((p.width, p.height, p.has_alpha), (2, 2, false));
}

#[test]
fn decode_rejects_malformed_input() {
    // Valid PNG magic but truncated body: the decoder must error, not panic.
    let mut broken = png_rgb_2x2();
    broken.truncate(20);
    assert!(decode(&broken).is_none(), "truncated png must not decode");
    assert!(probe(&broken).is_none(), "truncated png must not probe");

    // Valid JPEG magic, garbage payload.
    let bad_jpeg = [0xFF, 0xD8, 0xFF, 0x00, 0x00, 0x00];
    assert!(decode(&bad_jpeg).is_none(), "garbage jpeg must not decode");

    assert!(decode(b"neither").is_none());
}

// --------------------------------------------------------------------------
// size-cap tests
// --------------------------------------------------------------------------

fn empty_style() -> ComputedStyle {
    ComputedStyle::default()
}

fn box_style(style: &ComputedStyle) -> turbo_pdf_core::layout::value::BoxStyle {
    resolve_box_style(
        style,
        ResolveCtx {
            parent_font_size: 16.0,
            cb_width: 500.0,
        },
    )
}

#[test]
fn intrinsic_size_used_when_it_fits() {
    let style = empty_style();
    let bs = box_style(&style);
    let ctx = SizeCtx {
        style: &bs,
        cb_width: 500.0,
        body_height: Some(1000.0),
    };
    let intrinsic = probe(&png_rgb_2x2()).unwrap();
    let sized = size_replaced("x".into(), intrinsic, &ctx);
    assert_eq!((sized.width, sized.height), (2.0, 2.0));
    assert_eq!(sized.placement.intrinsic_w, 2);
}

#[test]
fn width_cap_clamps_to_containing_block_and_preserves_aspect() {
    let style = empty_style();
    let bs = box_style(&style);
    let ctx = SizeCtx {
        style: &bs,
        cb_width: 200.0,
        body_height: None, // height cap inactive: only the width cap applies
    };
    let intrinsic = probe(&png_wide()).unwrap(); // 400x100
    let sized = size_replaced("x".into(), intrinsic, &ctx);
    assert_eq!(sized.width, 200.0, "width clamped to cb_width");
    assert!(
        (sized.height - 50.0).abs() < 1e-3,
        "aspect preserved: 400x100 -> 200x50, got {}",
        sized.height
    );
}

#[test]
fn height_cap_clamps_to_60_percent_of_body_height() {
    let style = empty_style();
    let bs = box_style(&style);
    let body_height = 1000.0;
    let ctx = SizeCtx {
        style: &bs,
        cb_width: 10_000.0, // width never binds
        body_height: Some(body_height),
    };
    let intrinsic = probe(&png_tall()).unwrap(); // 100x4000
    let sized = size_replaced("x".into(), intrinsic, &ctx);
    let cap = body_height * 0.6;
    assert!(
        (sized.height - cap).abs() < 1e-3,
        "height clamped to 60% body height ({cap}), got {}",
        sized.height
    );
    // 100x4000 scaled so height=600 => scale 0.15 => width 15.
    assert!((sized.width - 15.0).abs() < 1e-3, "aspect preserved");
}

#[test]
fn explicit_width_fills_height_from_aspect_ratio() {
    let style = ComputedStyle::from_pairs([("width", "40px")]);
    let bs = box_style(&style);
    let ctx = SizeCtx {
        style: &bs,
        cb_width: 500.0,
        body_height: Some(10_000.0),
    };
    let intrinsic = probe(&png_wide()).unwrap(); // 400x100, ratio 4:1
    let sized = size_replaced("x".into(), intrinsic, &ctx);
    assert_eq!(sized.width, 40.0);
    assert!((sized.height - 10.0).abs() < 1e-3, "40 / 4 = 10");
}

#[test]
fn explicit_width_and_height_are_used_verbatim() {
    let style = ComputedStyle::from_pairs([("width", "30px"), ("height", "70px")]);
    let bs = box_style(&style);
    let ctx = SizeCtx {
        style: &bs,
        cb_width: 1000.0,
        body_height: Some(10_000.0),
    };
    let intrinsic = probe(&png_rgb_2x2()).unwrap();
    let sized = size_replaced("x".into(), intrinsic, &ctx);
    assert_eq!((sized.width, sized.height), (30.0, 70.0));
}

#[test]
fn explicit_height_fills_width_from_aspect_ratio() {
    let style = ComputedStyle::from_pairs([("height", "25px")]);
    let bs = box_style(&style);
    let ctx = SizeCtx {
        style: &bs,
        cb_width: 1000.0,
        body_height: Some(10_000.0),
    };
    let intrinsic = probe(&png_wide()).unwrap(); // 400x100, ratio 4:1
    let sized = size_replaced("x".into(), intrinsic, &ctx);
    assert_eq!(sized.height, 25.0);
    assert!((sized.width - 100.0).abs() < 1e-3, "25 * 4 = 100");
}

#[test]
fn degenerate_zero_dimension_falls_back_to_given() {
    // A zero-width intrinsic (degenerate) with an explicit height must not divide
    // by zero: the dependent axis falls back to the given value.
    use turbo_pdf_core::image::Intrinsic;
    let style = ComputedStyle::from_pairs([("height", "50px")]);
    let bs = box_style(&style);
    let ctx = SizeCtx {
        style: &bs,
        cb_width: 1000.0,
        body_height: None,
    };
    let intrinsic = Intrinsic {
        width: 10,
        height: 0,
        has_alpha: false,
    };
    let sized = size_replaced("x".into(), intrinsic, &ctx);
    assert_eq!(sized.height, 50.0);
    assert_eq!(sized.width, 50.0, "zero base => width falls back to given");
}

#[test]
fn height_cap_inactive_without_body_height() {
    let style = empty_style();
    let bs = box_style(&style);
    let ctx = SizeCtx {
        style: &bs,
        cb_width: 10_000.0,
        body_height: None,
    };
    let intrinsic = probe(&png_tall()).unwrap(); // 100x4000
    let sized = size_replaced("x".into(), intrinsic, &ctx);
    // No height cap: full intrinsic height kept.
    assert_eq!(sized.height, 4000.0);
}

// --------------------------------------------------------------------------
// layout + emit pipeline tests
// --------------------------------------------------------------------------

fn img_node(src: &str) -> Vec<Node> {
    vec![Node::Element(turbo_pdf_core::Element {
        tag: Tag::Html("img".into()),
        attrs: vec![Attr {
            name: "src".into(),
            value: src.into(),
        }],
        children: Vec::new(),
    })]
}

fn styled_img(src: &str) -> Vec<StyledNode> {
    let cascade = build_cascade("", "", TokenSet::default());
    style_tree(&img_node(src), &cascade)
}

fn page_with(body: Vec<turbo_pdf_core::Fragment>) -> Page {
    Page {
        geometry: PageGeometry::a4(),
        kind: turbo_pdf_core::PageKind::First,
        number: 1,
        body,
        header: Vec::new(),
        footer: Vec::new(),
        footnotes: Vec::new(),
    }
}

/// Lay out a single `<img>` and paginate it into pages, sizing against `images`.
fn paginate_img(src: &str, resolver: &dyn ImageResolver) -> Vec<Page> {
    let styled = styled_img(src);
    let ctx = ImageCtx {
        resolver,
        body_height: Some(700.0),
    };
    let mut diags = Diagnostics::default();
    let galley = layout_with_images(&styled, 540.0, &common::registry(), &ctx, &mut diags);
    paginate(&galley, &[], &mut diags).expect("paginate")
}

#[test]
fn emits_image_xobject_for_resolved_img() {
    let resolver = MapResolver::new(vec![("logo.png", png_rgb_2x2())]);
    let pages = paginate_img("logo.png", &resolver);
    let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(contains(&pdf, b"/Subtype /Image"), "missing image XObject");
    assert!(contains(&pdf, b"/Im0"), "missing image resource name");
    assert!(contains(&pdf, b"/DeviceRGB"), "rgb color space expected");
    assert!(contains(&pdf, b"%%EOF"));
}

#[test]
fn alpha_png_emits_smask() {
    let resolver = MapResolver::new(vec![("a.png", png_rgba_2x2())]);
    let pages = paginate_img("a.png", &resolver);
    let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert!(contains(&pdf, b"/SMask"), "alpha image must emit an SMask");
    assert!(
        contains(&pdf, b"/DeviceGray"),
        "smask must be a DeviceGray image"
    );
}

#[test]
fn jpeg_emits_dct_decode() {
    let resolver = MapResolver::new(vec![("p.jpg", JPEG_RGB_2X2.to_vec())]);
    let pages = paginate_img("p.jpg", &resolver);
    let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert!(contains(&pdf, b"/DCTDecode"), "jpeg must ride DCTDecode");
    assert!(!contains(&pdf, b"/SMask"), "baseline jpeg has no smask");
}

#[test]
fn grayscale_png_emits_device_gray() {
    let resolver = MapResolver::new(vec![("g.png", png_gray_2x2())]);
    let pages = paginate_img("g.png", &resolver);
    let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert!(contains(&pdf, b"/DeviceGray"));
}

#[test]
fn unresolved_image_emits_nothing_and_lays_out_zero_sized() {
    // The resolver has no entry for this name: layout sizes the box at zero and
    // the emitter embeds no XObject, but the document is still valid.
    let resolver = MapResolver::new(vec![]);
    let pages = paginate_img("missing.png", &resolver);
    let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(!contains(&pdf, b"/Subtype /Image"), "no image to embed");
}

#[test]
fn no_image_default_path_skips_image_fragments() {
    // `emit_pdf` (no resolver) on a page that carries an Image fragment must skip
    // it: no XObject, still a valid PDF. Exercises the painter's resolve-miss arm.
    let placement = turbo_pdf_core::ImagePlacement {
        name: "x.png".into(),
        intrinsic_w: 2,
        intrinsic_h: 2,
        has_alpha: false,
    };
    let frag = turbo_pdf_core::Fragment::new(
        turbo_pdf_core::NodeId(1),
        10.0,
        10.0,
        20.0,
        20.0,
        FragmentContent::Image(placement),
    );
    let pdf = emit_pdf(&[page_with(vec![frag])], &EmitOptions::default());
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(!contains(&pdf, b"/Subtype /Image"));
}

#[test]
fn no_images_resolver_resolves_nothing() {
    assert!(NoImages.resolve("anything").is_none());
}

fn image_frag(name: &str) -> turbo_pdf_core::Fragment {
    let placement = turbo_pdf_core::ImagePlacement {
        name: name.into(),
        intrinsic_w: 2,
        intrinsic_h: 2,
        has_alpha: false,
    };
    let mut frag = turbo_pdf_core::Fragment::new(
        turbo_pdf_core::NodeId(1),
        0.0,
        0.0,
        2.0,
        2.0,
        FragmentContent::Box {
            background: None,
            border: Default::default(),
        },
    );
    // Carry the image as a child so collect recurses into it too.
    frag.children.push(turbo_pdf_core::Fragment::new(
        turbo_pdf_core::NodeId(2),
        0.0,
        0.0,
        2.0,
        2.0,
        FragmentContent::Image(placement),
    ));
    frag
}

#[test]
fn image_store_collect_dedups_and_reports_size() {
    use turbo_pdf_core::emit::ImageStore;
    let resolver = MapResolver::new(vec![("dup.png", png_rgb_2x2())]);
    // The same name appears twice (across two fragments): collect must dedup.
    let page = page_with(vec![image_frag("dup.png"), image_frag("dup.png")]);
    let store = ImageStore::collect(&[page], &resolver);
    assert_eq!(store.len(), 1, "duplicate names collapse to one image");
    assert!(!store.is_empty());

    // An empty store reports emptiness.
    let empty = ImageStore::collect(&[page_with(vec![])], &resolver);
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
}

#[test]
fn image_store_skips_unresolvable_names_during_collect() {
    use turbo_pdf_core::emit::ImageStore;
    // The resolver lacks this name: collect resolves nothing and stores nothing.
    let resolver = MapResolver::new(vec![]);
    let store = ImageStore::collect(&[page_with(vec![image_frag("gone.png")])], &resolver);
    assert!(store.is_empty(), "unresolved names are skipped in collect");
}

#[test]
fn image_store_skips_undecodable_names_during_collect() {
    use turbo_pdf_core::emit::ImageStore;
    // The name resolves to bytes that are not a valid image: skipped, not panic.
    let resolver = MapResolver::new(vec![("bad.png", b"not an image".to_vec())]);
    let store = ImageStore::collect(&[page_with(vec![image_frag("bad.png")])], &resolver);
    assert!(store.is_empty(), "undecodable bytes are skipped in collect");
}

#[test]
fn image_emit_is_deterministic() {
    let resolver = MapResolver::new(vec![
        ("a.png", png_rgba_2x2()),
        ("b.jpg", JPEG_RGB_2X2.to_vec()),
    ]);
    let pages = paginate_img("a.png", &resolver);
    let a = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    let b = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert_eq!(a, b, "image embedding must be byte-deterministic");
}

#[test]
fn background_image_emits_xobject_behind_box() {
    let style = ComputedStyle::from_pairs([
        ("background-image", "url('bg.png')"),
        ("width", "100px"),
        ("height", "60px"),
    ]);
    let el = StyledElement {
        tag: Tag::Html("div".into()),
        attrs: Vec::new(),
        style,
        children: vec![StyledNode::Text("hi".into())],
    };
    let styled = vec![StyledNode::Element(el)];
    let resolver = MapResolver::new(vec![("bg.png", png_rgb_2x2())]);
    let ctx = ImageCtx {
        resolver: &resolver,
        body_height: Some(700.0),
    };
    let mut diags = Diagnostics::default();
    let galley = layout_with_images(&styled, 540.0, &common::registry(), &ctx, &mut diags);
    let pages = paginate(&galley, &[], &mut diags).expect("paginate");
    let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
    assert!(contains(&pdf, b"/Subtype /Image"), "bg image must embed");
}

/// The child boxes of a container box, or an empty slice for leaf kinds.
fn box_kids(
    b: &turbo_pdf_core::layout::boxgen::LayoutBox,
) -> &[turbo_pdf_core::layout::boxgen::LayoutBox] {
    use turbo_pdf_core::layout::boxgen::BoxKind;
    match &b.kind {
        BoxKind::Block(kids) | BoxKind::Flex(kids) | BoxKind::Table(kids) => kids,
        _ => &[],
    }
}

/// Whether any box in the tree records a replaced image named `name`.
fn tree_has_replaced_image(b: &turbo_pdf_core::layout::boxgen::LayoutBox, name: &str) -> bool {
    let here = b
        .image
        .as_ref()
        .is_some_and(|s| s.replaced && s.name == name);
    here || box_kids(b).iter().any(|k| tree_has_replaced_image(k, name))
}

#[test]
fn build_box_tree_records_img_source() {
    // The boxgen path records `<img src>` as replaced content.
    let styled = styled_img("z.png");
    let tree = build_box_tree(&styled);
    assert!(
        tree_has_replaced_image(&tree, "z.png"),
        "img src must be recorded on the box"
    );
}

// --------------------------------------------------------------------------
// qpdf structural check (gated on availability)
// --------------------------------------------------------------------------

#[test]
fn qpdf_check_image_pdf_when_available() {
    if !qpdf_available() {
        return;
    }
    let resolver = MapResolver::new(vec![
        ("rgb.png", png_rgb_2x2()),
        ("rgba.png", png_rgba_2x2()),
        ("gray.png", png_gray_2x2()),
        ("photo.jpg", JPEG_RGB_2X2.to_vec()),
    ]);
    for name in ["rgb.png", "rgba.png", "gray.png", "photo.jpg"] {
        let pages = paginate_img(name, &resolver);
        let pdf = emit_pdf_with_images(&pages, &EmitOptions::default(), &resolver);
        assert_qpdf_clean(name, &pdf);
    }
}

fn qpdf_available() -> bool {
    std::process::Command::new("which")
        .arg("qpdf")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn assert_qpdf_clean(name: &str, pdf: &[u8]) {
    let path = std::env::temp_dir().join(format!("turbo-pdf-img-{name}.pdf"));
    std::fs::write(&path, pdf).expect("write temp pdf");
    let out = std::process::Command::new("qpdf")
        .arg("--check")
        .arg(&path)
        .output()
        .expect("run qpdf");
    assert!(
        out.status.success(),
        "{name}: qpdf --check failed: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}
