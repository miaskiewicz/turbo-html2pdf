//! A rendered template flow: the parsed turbo-dom document plus the nodes that
//! flow onto the pages (the `<body>` children), styled and laid out by
//! turbo-dom-layout with html2pdf's directive classifier and build switches.

use turbo_dom::{Handle, Tree};
use turbo_dom_layout::{
    style_input, Cascade, LayoutOptions, Roots, StyleInput, StyleOptions, StyledNode, TreeView,
};

/// The style-time switches of this build: `cmyk(...)` colours with `print-color`.
pub(crate) fn style_options() -> StyleOptions {
    StyleOptions {
        accept_cmyk: cfg!(feature = "print-color"),
    }
}

/// The layout-time switches of this build (`xref`: atomic internal links;
/// `print-color`: `cmyk(...)` colours), starting the box ids at `first_node_id`.
pub(crate) fn layout_options(first_node_id: u32) -> LayoutOptions {
    LayoutOptions {
        atomic_internal_links: cfg!(feature = "xref"),
        accept_cmyk: cfg!(feature = "print-color"),
        first_node_id,
    }
}

/// A rendered template's document and flow roots (see the module docs).
pub struct Flow {
    tree: Tree,
    roots: Vec<Handle>,
}

impl std::fmt::Debug for Flow {
    /// The flow's roots and their serialized markup.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let markup: String = self
            .roots
            .iter()
            .map(|&h| turbo_dom::rtdom::serialize::serialize_outer(&self.tree, h))
            .collect();
        f.debug_struct("Flow")
            .field("roots", &self.roots)
            .field("markup", &markup)
            .finish()
    }
}

impl Flow {
    /// The flow of a parsed document: its `<body>` content.
    pub fn from_document(tree: Tree) -> Flow {
        Flow::from_body(tree)
    }

    pub(crate) fn from_body(tree: Tree) -> Flow {
        let body = crate::template::markup::body_of(&tree);
        let roots = TreeView::new(&tree).children(body);
        Flow { tree, roots }
    }

    /// The parsed document.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// The document, for in-place rewrites (endnotes, footnote bodies).
    pub(crate) fn tree_mut(&mut self) -> &mut Tree {
        &mut self.tree
    }

    /// The top-level flow nodes (elements and text), in order.
    pub fn roots(&self) -> &[Handle] {
        &self.roots
    }

    /// Replace the top-level flow nodes (after a rewrite).
    #[cfg(feature = "endnotes")]
    pub(crate) fn set_roots(&mut self, roots: Vec<Handle>) {
        self.roots = roots;
    }

    /// The layout view of the document: `t:` directives classified as markers.
    pub fn view(&self) -> TreeView<'_> {
        TreeView::new(&self.tree).markers(&crate::markers::classify)
    }

    /// Style the flow (detached from the document: no ancestors, default parent).
    pub fn style(&self, cascade: &Cascade) -> Vec<StyledNode> {
        self.style_roots(&self.roots, cascade)
    }

    /// Style `roots` of this document as a detached flow of their own.
    pub(crate) fn style_roots(&self, roots: &[Handle], cascade: &Cascade) -> Vec<StyledNode> {
        let input = StyleInput {
            view: self.view(),
            roots: Roots::Detached(roots),
        };
        style_input(&input, cascade, &style_options())
    }
}
