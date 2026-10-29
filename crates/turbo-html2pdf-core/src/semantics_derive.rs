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
//! | box of an element (incl. a table row) | by tag | — | `href` (not for rows) | `id` |
//! | the anonymous grid box of a captioned table | — | — | — | — |
//! | block marker box | — | — | `href` | — |
//! | inline marker | — | — | — | `<t:anchor name>` |
//! | replaced `<img>` image | by tag (`Figure`) | `alt` (or "") | — | — |
//! | background / mask image, text line | — | — | — | — |
//!
//! A link `href` is internal (`#name`, a GoTo to the named destination) or external
//! (an absolute URL with a scheme, a URI action); a relative href has no base URL to
//! resolve against, so it carries no link. Like Chrome, every element `id` names a
//! destination, as does `<t:anchor name>`.

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
    /// A table caption (`<caption>`), a child of its `Table`.
    Caption,
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
    link: Option<LinkTarget>,
    #[cfg(feature = "xref")]
    anchor: Option<String>,
    /// The element's non-empty `id`: a named destination (as in Chrome).
    #[cfg(feature = "xref")]
    id: Option<String>,
}

/// Where a link `href` points.
#[cfg(feature = "xref")]
#[derive(Debug, PartialEq, Eq)]
enum LinkTarget {
    /// `#name`: a named destination in this document.
    Internal(String),
    /// An absolute URL (it has a scheme).
    Uri(String),
}

/// Classify an `href`: `#name` (non-empty) is internal, an absolute URL is a URI,
/// anything else (empty, bare `#`, relative) carries no link.
#[cfg(feature = "xref")]
fn link_target(href: &str) -> Option<LinkTarget> {
    if let Some(name) = href.strip_prefix('#') {
        return (!name.is_empty()).then(|| LinkTarget::Internal(name.to_string()));
    }
    has_scheme(href).then(|| LinkTarget::Uri(href.to_string()))
}

/// Whether `href` starts with a URL scheme (`ALPHA *(ALPHA / DIGIT / "+" / "-" / ".") ":"`).
#[cfg(feature = "xref")]
fn has_scheme(href: &str) -> bool {
    let Some((scheme, _)) = href.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
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
    /// The anonymous grid boxes of captioned tables: no structure of their own, so the
    /// rows they hold are the table's children (miaskiewicz/turbo-dom#51).
    grids: HashSet<NodeId>,
    nodes: HashMap<NodeId, NodeInfo>,
}

impl Derived {
    /// Record one laid-out flow: `view` is the document its boxes came from.
    pub(crate) fn record(&mut self, view: &TreeView, sources: &NodeSources) {
        self.ranges.push((sources.root.0, sources.next.0));
        self.roots.insert(sources.root);
        self.rows.extend(sources.table_rows.iter().copied());
        self.grids.extend(sources.table_grids.iter().copied());
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
        link: view.attr(h, "href").and_then(link_target),
        #[cfg(feature = "xref")]
        anchor: (marker == Some(TKind::Anchor))
            .then(|| view.attr(h, "name").filter(|n| !n.is_empty()))
            .flatten()
            .map(str::to_string),
        #[cfg(feature = "xref")]
        id: view
            .attr(h, "id")
            .filter(|id| !id.is_empty())
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
        "caption" => UaRole::Caption,
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
            FragmentContent::Box { .. } if d.grids.contains(&frag.node_id) => None,
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
        match self.derived.link_box(frag)?.link.as_ref()? {
            LinkTarget::Internal(name) => Some(name),
            LinkTarget::Uri(_) => None,
        }
    }

    /// The absolute URL of an external-link box fragment (`href` with a scheme).
    #[cfg(feature = "xref")]
    pub fn link_uri(&self, frag: &Fragment) -> Option<&str> {
        match self.derived.link_box(frag)?.link.as_ref()? {
            LinkTarget::Uri(uri) => Some(uri),
            LinkTarget::Internal(_) => None,
        }
    }

    /// The destination name a fragment defines: a `<t:anchor name>` marker's
    /// `name`, or an element box's `id` (Chrome names a destination per `id`).
    #[cfg(feature = "xref")]
    pub fn anchor(&self, frag: &Fragment) -> Option<&str> {
        let info = self.derived.nodes.get(&frag.node_id)?;
        match &frag.content {
            FragmentContent::Marker(kind) if kind.inline => info.marker.and(info.anchor.as_deref()),
            FragmentContent::Box { .. } => info.id.as_deref(),
            _ => None,
        }
    }
}
