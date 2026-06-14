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
pub mod layout;
pub mod node;
pub mod options;
pub mod paginate;
pub mod style;
mod template;
pub mod text;

pub use emit::{emit_pdf, EmitOptions, SENTINEL_DATE};
pub use error::{CompileError, Diagnostics, ErrorCode, Lint, LintCode, RenderError, Span};
pub use layout::fragment::{
    BreakMeta, Fragment, FragmentContent, NodeId, PositionedGlyph, RepeatKind,
};
pub use layout::layout;
pub use layout::value::{BreakRule, Rgba};
pub use node::{Attr, Element, Node, TKind, Tag};
pub use options::{CompileOptions, MissingPolicy, DEFAULT_INCLUDE_DEPTH};
pub use paginate::{paginate, Page, PageGeometry, PageKind};
pub use style::{build_cascade, style_tree, Cascade, ComputedStyle, StyledElement, StyledNode};
pub use template::{compile, set_now, Program};
pub use text::{layout_text, Align, FontFace, FontRegistry, LineBox, TextStyle, WhiteSpace};
