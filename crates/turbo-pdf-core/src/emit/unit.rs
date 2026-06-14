//! Coordinate conversion between the galley (CSS px @ 96 dpi, y-down) and PDF
//! user space (points @ 72/in, y-up).

/// Pixels-to-points factor: `72 / 96`. The galley is 96 dpi; PDF is 72 dpi.
const PX_TO_PT: f32 = 72.0 / 96.0;

/// Convert a CSS-pixel length to PDF points.
pub fn px_to_pt(px: f32) -> f32 {
    px * PX_TO_PT
}

/// Flip a galley y (top-down, in px) into a PDF y (bottom-up, in points)
/// against a page height already expressed in points.
pub fn flip_y(y_px: f32, page_height_pt: f32) -> f32 {
    page_height_pt - px_to_pt(y_px)
}
