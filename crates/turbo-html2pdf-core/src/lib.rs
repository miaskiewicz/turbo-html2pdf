//! turbo-pdf core engine.
//!
//! A native HTML/CSS-to-PDF engine with a Jinja-compatible templating DSL. A
//! template is compiled once into a reusable [`Program`], then rendered against
//! data. See `docs/spec.md` for the full specification and build order.
//!
//! This crate is built in phases. The templating layer (§2) lands first; layout,
//! pagination, and the PDF emitter follow. Public re-exports grow per phase.

#![forbid(unsafe_code)]

#[cfg(feature = "append")]
pub mod append;
pub mod emit;
#[cfg(feature = "endnotes")]
pub mod endnotes;
pub mod options;
pub mod paginate;
pub mod render;
#[cfg(feature = "stamp")]
pub mod stamp;
#[cfg(feature = "stamp")]
mod stamp_font;
mod template;

// The layout half (style cascade, layout, text/fonts, layout snapshots, the HTML
// drive, images, errors) lives in `turbo-dom-layout`; it is re-exported here at the
// paths it had in this crate, so `turbo_html2pdf_core::style::…`, `::layout::…`,
// `::Node`, `::Fragment`, … keep working unchanged.
#[cfg(feature = "svg")]
pub use turbo_dom_layout::svg;
pub use turbo_dom_layout::{
    error, hot, html_layout, image, layout, layout_snapshot, node, perf, style, text,
};

#[cfg(feature = "append")]
pub use append::{append_pdfs, AppendError};
pub use emit::{
    emit_pdf, emit_pdf_with_images, EmitOptions, ImageWatermark, TextWatermark, Watermark,
    SENTINEL_DATE,
};
#[cfg(feature = "encrypt")]
pub use emit::{Encryption, Permissions};
pub use error::{CompileError, Diagnostics, ErrorCode, Lint, LintCode, RenderError, Span};
pub use html_layout::{
    collect_style_css, layout_boxes, layout_html, layout_html_with_images, parse_html, CidBox,
};
pub use image::{ImageResolver, NoImages};
pub use layout::fragment::{
    BoxShadow, BreakMeta, Fragment, FragmentContent, GradientStop, ImagePlacement, LinearGradient,
    NodeId, PositionedGlyph, RepeatKind, Transform2D,
};
pub use layout::value::{BreakRule, Rgba};
pub use layout::{layout_with_images, ImageCtx}; // `layout` (fn + module) comes with the module re-export above
pub use node::{Attr, Element, Node, TKind, Tag};
pub use options::{CompileOptions, MissingPolicy, DEFAULT_INCLUDE_DEPTH};
pub use paginate::{
    paginate, paginate_with_footnotes, paginate_with_geometry, resolve_geometry, FootnoteBand,
    Note, Page, PageGeometry, PageKind,
};
pub use render::{render_pages, PageContext, RenderInputs};
#[cfg(feature = "stamp")]
pub use stamp::{stamp, StampError, StampWatermark, WATERMARK_XOBJECT_NAME};
pub use style::{
    build_cascade, build_cascade_with_width, set_media_viewport_height, style_tree, Cascade,
    ComputedStyle, StyledElement, StyledNode,
};
pub use template::{compile, set_now, Program};
pub use text::{layout_text, Align, FontFace, FontRegistry, LineBox, TextStyle, WhiteSpace};
