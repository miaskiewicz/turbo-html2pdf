//! Device color (§7, AC-7.3). The default build emits everything in DeviceRGB;
//! the channel values are the `Rgba` 0..=255 bytes mapped to 0.0..=1.0 floats.
//!
//! With the `print-color` feature (AC-7.x print color) the same fills can be
//! emitted in DeviceCMYK instead, per render: every painter routes its fill
//! through [`set_fill`] carrying the render's `cmyk` flag, so flipping the colour
//! space is a single runtime branch and the rest of the emitter is colour-space
//! agnostic. With `cmyk == false` (the default) the bytes are byte-for-byte the
//! DeviceRGB output, even under the `print-color` build.

use pdf_writer::Content;

use crate::layout::value::Rgba;

/// An RGB triple in PDF's 0.0..=1.0 device range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceRgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

/// Map one 8-bit channel to the 0.0..=1.0 range.
fn channel(v: u8) -> f32 {
    f32::from(v) / 255.0
}

/// Convert an [`Rgba`] to device RGB, dropping alpha (the default build has no
/// transparency).
pub fn device_rgb(c: Rgba) -> DeviceRgb {
    DeviceRgb {
        r: channel(c.r),
        g: channel(c.g),
        b: channel(c.b),
    }
}

/// Set the current non-stroking (fill) colour on `content` for `c`. Painters call
/// this instead of `set_fill_rgb` directly so the device colour space is decided
/// in one place: DeviceRGB by default, DeviceCMYK only when the render set
/// `cmyk == true` *and* the `print-color` feature is compiled in. With `cmyk`
/// off (or the feature absent) the emitted bytes are exactly the DeviceRGB path.
pub fn set_fill(content: &mut Content, c: Rgba, cmyk: bool) {
    #[cfg(feature = "print-color")]
    if cmyk {
        let q = device_cmyk(c);
        content.set_fill_cmyk(q.c, q.m, q.y, q.k);
        return;
    }
    // Default / `cmyk == false` / non-`print-color` build: DeviceRGB.
    let _ = cmyk;
    let rgb = device_rgb(c);
    content.set_fill_rgb(rgb.r, rgb.g, rgb.b);
}

/// A CMYK quadruple in PDF's 0.0..=1.0 device range (`print-color`).
#[cfg(feature = "print-color")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceCmyk {
    pub c: f32,
    pub m: f32,
    pub y: f32,
    pub k: f32,
}

/// Convert an [`Rgba`] to DeviceCMYK with the standard, non-colour-managed naive
/// transform: `k = 1 - max(r,g,b)`, then `c/m/y = (1-channel-k)/(1-k)`. Pure
/// black (`k == 1`) yields `c = m = y = 0`. Deterministic and total.
#[cfg(feature = "print-color")]
pub fn device_cmyk(color: Rgba) -> DeviceCmyk {
    let r = channel(color.r);
    let g = channel(color.g);
    let b = channel(color.b);
    let k = 1.0 - r.max(g).max(b);
    let inv = 1.0 - k;
    if inv <= 0.0 {
        return DeviceCmyk {
            c: 0.0,
            m: 0.0,
            y: 0.0,
            k: 1.0,
        };
    }
    DeviceCmyk {
        c: (1.0 - r - k) / inv,
        m: (1.0 - g - k) / inv,
        y: (1.0 - b - k) / inv,
        k,
    }
}
