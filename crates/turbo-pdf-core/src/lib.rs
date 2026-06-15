//! turbo-pdf core engine.
//!
//! A native HTML/CSS-to-PDF engine with a Jinja-compatible templating DSL. A
//! template is compiled once into a reusable [`Program`], then rendered against
//! data. See `docs/spec.md` for the full specification and build order.
//!
//! This crate is built in phases. The templating layer (§2) lands first; layout,
//! pagination, and the PDF emitter follow. Public re-exports grow per phase.

#![forbid(unsafe_code)]

pub mod emit;
pub mod error;
pub mod image;
pub mod layout;
pub mod node;
pub mod options;
pub mod paginate;
pub mod perf;
pub mod render;
pub mod style;
mod template;
pub mod text;

pub use emit::{
    emit_pdf, emit_pdf_with_images, EmitOptions, ImageWatermark, TextWatermark, Watermark,
    SENTINEL_DATE,
};
pub use error::{CompileError, Diagnostics, ErrorCode, Lint, LintCode, RenderError, Span};
pub use image::{ImageResolver, NoImages};
pub use layout::fragment::{
    BreakMeta, Fragment, FragmentContent, ImagePlacement, NodeId, PositionedGlyph, RepeatKind,
};
pub use layout::value::{BreakRule, Rgba};
pub use layout::{layout, layout_with_images, ImageCtx};
pub use node::{Attr, Element, Node, TKind, Tag};
pub use options::{CompileOptions, MissingPolicy, DEFAULT_INCLUDE_DEPTH};
pub use paginate::{
    paginate, paginate_with_footnotes, paginate_with_geometry, resolve_geometry, FootnoteBand,
    Note, Page, PageGeometry, PageKind,
};
pub use render::{render_pages, PageContext, RenderInputs};
pub use style::{build_cascade, style_tree, Cascade, ComputedStyle, StyledElement, StyledNode};
pub use template::{compile, set_now, Program};
pub use text::{layout_text, Align, FontFace, FontRegistry, LineBox, TextStyle, WhiteSpace};
