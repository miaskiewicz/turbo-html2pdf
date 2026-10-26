//! The `pdf-ua`/`xref` derivation behind [`Semantics`](super::Semantics). Only
//! compiled with either feature, so it lives in its own (coverage-excluded) file
//! like the other gated-only modules; exercised by the `--features pdf-ua` /
//! `--features xref` tests.
//!
//! Derivation table (it reproduces the roles/links the layout engine used to
//! stamp on fragments under the old compile-time features):
//!
//! | fragment | role | alt | link | anchor |
//! |---|---|---|---|---|
//! | box of the flow root | `Group` | — | — | — |
//! | box of an anonymous (unsourced) block | `Paragraph` | — | — | — |
//! | box of an element (incl. a table row) | by tag | — | `href="#…"` (not for rows) | — |
//! | block marker box | — | — | `href="#…"` | — |
//! | inline marker | — | — | — | `<t:anchor name>` |
//! | replaced `<img>` image | by tag (`Figure`) | `alt` (or "") | — | — |
//! | background / mask image, text line | — | — | — | — |

// A single-feature build reads only its half of the table.
#![cfg_attr(not(all(feature = "pdf-ua", feature = "xref")), allow(dead_code))]

use std::collections::{HashMap, HashSet};

use turbo_dom_layout::{NodeSources, TreeView};

use crate::layout::fragment::{Fragment, FragmentContent, NodeId};
use crate::markers::TKind;

/// A PDF/UA structure role a fragment carries so the tagged-PDF emitter can build
/// a `StructTreeRoot`. Mirrors the subset of ISO 32000 standard structure types
/// this engine maps to; the emitter translates it to a `pdf_writer::StructRole`.
#[cfg(feature = "pdf-ua")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UaRole {
    /// A structural container with no own marked content (`<div>`, `<section>`).
    Group,
    /// `<h1>`..`<h6>`, by 1-based level.
    Heading(u8),
    /// A paragraph (`<p>` and block text).
    Paragraph,
    /// A list (`<ul>`/`<ol>`).
    List,
    /// A list item (`<li>`).
    ListItem,
    /// A list item's body (the content of an `<li>`).
    ListBody,
    /// A table (`<table>`).
    Table,
    /// A table row (`<tr>`).
    TableRow,
    /// A table header cell (`<th>`).
    TableHeader,
    /// A table data cell (`<td>`).
    TableData,
    /// A figure (`<img>`); carries `/Alt` from the `alt` attribute.
    Figure,
    /// A run of inline text (`<span>`, `<a>`, anonymous inline boxes).
    Span,
    /// Page decoration to be skipped by assistive tech (`/Artifact`): running
    /// header/footer chrome and watermarks. Not a structure element.
    Artifact,
}

/// What one source element contributes.
#[derive(Debug)]
struct NodeInfo {
    /// `Some` for a classified directive marker.
    marker: Option<TKind>,
    /// Whether the element is an `<img>`.
    img: bool,
    #[cfg(feature = "pdf-ua")]
    role: Option<UaRole>,
    #[cfg(feature = "pdf-ua")]
    alt: Option<String>,
    #[cfg(feature = "xref")]
    link: Option<String>,
    #[cfg(feature = "xref")]
    anchor: Option<String>,
}

/// The derived table for one render.
#[derive(Debug, Default)]
pub(crate) struct Derived {
    /// The `[first, next)` id range of every recorded flow.
    ranges: Vec<(u32, u32)>,
    /// Each flow's root box.
    roots: HashSet<NodeId>,
    /// Boxes laid out as table rows.
    rows: HashSet<NodeId>,
    nodes: HashMap<NodeId, NodeInfo>,
}

impl Derived {
    /// Record one laid-out flow: `view` is the document its boxes came from.
    pub(crate) fn record(&mut self, view: &TreeView, sources: &NodeSources) {
        self.ranges.push((sources.root.0, sources.next.0));
        self.roots.insert(sources.root);
        self.rows.extend(sources.table_rows.iter().copied());
        for (&id, &h) in &sources.elements {
            self.nodes.insert(id, node_info(view, h));
        }
    }

    fn in_range(&self, id: NodeId) -> bool {
        self.ranges.iter().any(|&(a, b)| (a..b).contains(&id.0))
    }

    /// The replaced-image element of an `Image` fragment (masks are tinted).
    fn replaced_img(&self, frag: &Fragment) -> Option<&NodeInfo> {
        let FragmentContent::Image(placement) = &frag.content else {
            return None;
        };
        let info = self.nodes.get(&frag.node_id)?;
        (info.img && placement.tint.is_none()).then_some(info)
    }

    /// The element of a box-like fragment that may carry a link: an element box
    /// or a block marker box, not a table row.
    #[cfg(feature = "xref")]
    fn link_box(&self, frag: &Fragment) -> Option<&NodeInfo> {
        let boxlike = match &frag.content {
            FragmentContent::Box { .. } => true,
            FragmentContent::Marker(kind) => !kind.inline,
            _ => false,
        };
        if !boxlike || self.rows.contains(&frag.node_id) {
            return None;
        }
        self.nodes.get(&frag.node_id)
    }
}

fn node_info(view: &TreeView, h: turbo_dom::Handle) -> NodeInfo {
    let local = view.tree().local_name(h).unwrap_or_default();
    let marker = view.marker(h).and_then(TKind::from_marker);
    let img = marker.is_none() && local == "img";
    NodeInfo {
        marker,
        img,
        #[cfg(feature = "pdf-ua")]
        role: marker.map_or_else(|| role_for_tag(local), |_| None),
        #[cfg(feature = "pdf-ua")]
        alt: img.then(|| view.attr(h, "alt").unwrap_or("").to_string()),
        #[cfg(feature = "xref")]
        link: view
            .attr(h, "href")
            .and_then(|href| href.strip_prefix('#'))
            .filter(|target| !target.is_empty())
            .map(str::to_string),
        #[cfg(feature = "xref")]
        anchor: (marker == Some(TKind::Anchor))
            .then(|| view.attr(h, "name").filter(|n| !n.is_empty()))
            .flatten()
            .map(str::to_string),
    }
}

/// Map a lowercase HTML tag name to its structure role.
#[cfg(feature = "pdf-ua")]
fn role_for_tag(name: &str) -> Option<UaRole> {
    if let Some(level) = heading_level(name) {
        return Some(UaRole::Heading(level));
    }
    Some(match name {
        "p" => UaRole::Paragraph,
        "ul" | "ol" => UaRole::List,
        "li" => UaRole::ListItem,
        "table" => UaRole::Table,
        "tr" => UaRole::TableRow,
        "th" => UaRole::TableHeader,
        "td" => UaRole::TableData,
        "img" => UaRole::Figure,
        "span" | "a" | "em" | "strong" | "b" | "i" => UaRole::Span,
        "div" | "section" | "article" | "header" | "footer" | "main" | "nav" | "aside" => {
            UaRole::Group
        }
        _ => return None,
    })
}

/// The 1-based heading level of `h1`..`h6`, or `None`.
#[cfg(feature = "pdf-ua")]
fn heading_level(name: &str) -> Option<u8> {
    let level = name.strip_prefix('h')?.parse::<u8>().ok()?;
    (1..=6).contains(&level).then_some(level)
}

impl super::Semantics {
    /// Wrap a finished table.
    pub(crate) fn from_derived(derived: Derived) -> Self {
        super::Semantics {
            derived: std::sync::Arc::new(derived),
        }
    }

    /// The PDF/UA structure role of `frag` (`None` = transparent to tagging).
    #[cfg(feature = "pdf-ua")]
    pub fn role(&self, frag: &Fragment) -> Option<UaRole> {
        let d = &self.derived;
        match &frag.content {
            FragmentContent::Box { .. } if d.roots.contains(&frag.node_id) => Some(UaRole::Group),
            FragmentContent::Box { .. } => match d.nodes.get(&frag.node_id) {
                Some(info) => info.role,
                None => d.in_range(frag.node_id).then_some(UaRole::Paragraph),
            },
            FragmentContent::Image(_) => d.replaced_img(frag).and_then(|info| info.role),
            _ => None,
        }
    }

    /// The `/Alt` text of a replaced-image fragment.
    #[cfg(feature = "pdf-ua")]
    pub fn alt(&self, frag: &Fragment) -> Option<&str> {
        self.derived.replaced_img(frag)?.alt.as_deref()
    }

    /// The bare `#name` target of an internal-link box fragment.
    #[cfg(feature = "xref")]
    pub fn link_href(&self, frag: &Fragment) -> Option<&str> {
        self.derived.link_box(frag)?.link.as_deref()
    }

    /// The destination name a `<t:anchor name>` marker fragment defines.
    #[cfg(feature = "xref")]
    pub fn anchor(&self, frag: &Fragment) -> Option<&str> {
        let FragmentContent::Marker(kind) = &frag.content else {
            return None;
        };
        if !kind.inline {
            return None;
        }
        let info = self.derived.nodes.get(&frag.node_id)?;
        info.marker.and(info.anchor.as_deref())
    }
}
