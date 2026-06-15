//! The layout engine (§5, Stage 3): turns the styled tree into a galley — a
//! single continuous coordinate space of positioned [`Fragment`]s that the
//! fragmenter (Stage 4) paginates. Block and table flow and inline/text layout
//! are owned here; flex delegates to `taffy` (§5.3 decision).

pub mod block;
pub mod boxgen;
pub mod flex;
pub mod fragment;
pub mod imgsize;
pub mod inline;
pub mod table;
pub mod value;

use crate::error::Diagnostics;
use crate::image::{ImageResolver, NoImages};
use crate::style::StyledNode;
use crate::text::FontRegistry;

pub use fragment::Fragment;

/// Per-layout image inputs (§7.4): the caller's resolver (for intrinsic sizing)
/// and the page body height that bounds image height (the 60% cap, user spec).
/// `body_height` is `None` when the caller lays out without page geometry (e.g.
/// measuring a region), in which case only the width cap applies.
pub struct ImageCtx<'a> {
    pub resolver: &'a dyn ImageResolver,
    pub body_height: Option<f32>,
}

impl ImageCtx<'_> {
    /// The default: no resolver, no height basis — lays out no images.
    pub(crate) fn none() -> ImageCtx<'static> {
        ImageCtx {
            resolver: &NoImages,
            body_height: None,
        }
    }
}

/// Lay out a styled tree into the galley (Stage 3): box generation followed by
/// block/inline/flex/table layout into one continuous top-down fragment tree of
/// content width `cb_width` px. Missing-glyph and similar issues are collected
/// into `diags`.
///
/// This convenience entry embeds no images; use [`layout_with_images`] to size
/// `<img>`/`background-image` boxes against a caller-supplied resolver.
pub fn layout(
    styled: &[StyledNode],
    cb_width: f32,
    fonts: &FontRegistry,
    diags: &mut Diagnostics,
) -> Fragment {
    let tree = boxgen::build_box_tree(styled);
    block::layout_tree(&tree, cb_width, fonts, diags)
}

/// Lay out a styled tree, sizing raster images against `images` (§7.4). Each
/// `<img>`/`background-image` is probed for its intrinsic pixel size, scaled to
/// preserve aspect ratio, and capped to `max-width = 100%` of its containing
/// block and `max-height ≈ 60%` of `images.body_height` so an image never
/// overflows a page (images are never split across pages).
pub fn layout_with_images(
    styled: &[StyledNode],
    cb_width: f32,
    fonts: &FontRegistry,
    images: &ImageCtx,
    diags: &mut Diagnostics,
) -> Fragment {
    let tree = boxgen::build_box_tree(styled);
    block::layout_tree_with_images(&tree, cb_width, fonts, images, diags)
}
