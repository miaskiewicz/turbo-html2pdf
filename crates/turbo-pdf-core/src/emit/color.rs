//! Device RGB color (§7, AC-7.3). v1 emits everything in DeviceRGB; the channel
//! values are the `Rgba` 0..=255 bytes mapped to 0.0..=1.0 floats.

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

/// Convert an [`Rgba`] to device RGB, dropping alpha (v1 has no transparency).
pub fn device_rgb(c: Rgba) -> DeviceRgb {
    DeviceRgb {
        r: channel(c.r),
        g: channel(c.g),
        b: channel(c.b),
    }
}
