//! The resolved node tree (§1 Stage 1 output): the in-memory tree html5ever
//! yields after the Jinja pass, with `t:` elements recognized as typed layout
//! directives. Later phases (style resolution, layout) consume this tree.

/// A name/value attribute on an element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attr {
    pub name: String,
    pub value: String,
}

/// A recognized `t:` paged-media directive element (§3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TKind {
    Footnote,
    FootnoteSeparator,
    RunningHeader,
    RunningFooter,
    PageMaster,
    Region,
    Variant,
    Page,
    Pages,
    UseMaster,
    Counter,
    Leader,
    Anchor,
    Endnote,
    Endnotes,
}

/// Map a `t:`-stripped local name to its directive kind, or `None` if unknown.
pub fn t_kind(name: &str) -> Option<TKind> {
    let kind = match name {
        "footnote" => TKind::Footnote,
        "footnote-separator" => TKind::FootnoteSeparator,
        "running-header" => TKind::RunningHeader,
        "running-footer" => TKind::RunningFooter,
        "page-master" => TKind::PageMaster,
        "region" => TKind::Region,
        "variant" => TKind::Variant,
        "page" => TKind::Page,
        "pages" => TKind::Pages,
        "use-master" => TKind::UseMaster,
        "counter" => TKind::Counter,
        "leader" => TKind::Leader,
        "anchor" => TKind::Anchor,
        "endnote" => TKind::Endnote,
        "endnotes" => TKind::Endnotes,
        _ => return None,
    };
    Some(kind)
}

/// An element's tag: either a plain HTML element or a `t:` directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tag {
    /// A normal HTML element, by lowercase local name (e.g. `div`, `table`).
    Html(String),
    /// A recognized `t:` directive.
    Directive(TKind),
}

/// A styled-but-unpositioned element node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub tag: Tag,
    pub attrs: Vec<Attr>,
    pub children: Vec<Node>,
}

impl Element {
    /// The value of the named attribute, if present.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.value.as_str())
    }
}

/// A node in the resolved tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Element(Element),
    Text(String),
}

impl Node {
    /// Borrow the element if this node is one.
    pub fn as_element(&self) -> Option<&Element> {
        match self {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        }
    }

    /// Borrow the text if this node is a text node.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Node::Text(t) => Some(t),
            Node::Element(_) => None,
        }
    }
}
