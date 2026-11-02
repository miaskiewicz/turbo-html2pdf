//! Per-render fragment semantics the PDF emitter needs beyond geometry: PDF/UA
//! structure roles + `/Alt` text (`pdf-ua`) and internal-link targets + named
//! destinations (`xref`).
//!
//! The layout engine is PDF-agnostic: it records which source element each box
//! came from ([`turbo_dom_layout::NodeSources`]) and html2pdf derives the
//! semantics from that map plus the element's tag and attributes after layout.
//! Every flow of a render (body, footnotes, running regions) is laid out with its
//! own disjoint [`NodeId`](crate::NodeId) range, so one table serves the whole
//! document; each [`Page`](crate::Page) carries it. Without the `pdf-ua`/`xref`
//! features the table is empty and nothing is recorded.

/// The derived semantics of one render (see the module docs). Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct Semantics {
    #[cfg(any(feature = "pdf-ua", feature = "xref"))]
    pub(crate) derived: std::sync::Arc<derive::Derived>,
}

#[cfg(any(feature = "pdf-ua", feature = "xref"))]
#[path = "semantics_derive.rs"]
pub(crate) mod derive;

#[cfg(feature = "pdf-ua")]
pub use derive::UaRole;
