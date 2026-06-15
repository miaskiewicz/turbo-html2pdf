//! Endnotes (`endnotes` feature, §3): `<t:endnote>` collects a note out of the
//! body flow and leaves a superscript number where it stood; `<t:endnotes/>`
//! renders the collected notes, in document order, as a numbered list at its own
//! position in the flow.
//!
//! Unlike footnotes — which are page-anchored and resolved by the body/footnote
//! fixpoint (§6.4) — endnotes are *document-anchored*: all notes gather to the one
//! `<t:endnotes/>` slot. That makes this a pure node→node rewrite that runs once,
//! right after templating, before the rest of the spine (style → layout →
//! paginate) sees the tree. The numbering builds on the same first-encounter,
//! document-order counting the footnote infra uses.
//!
//! The rewrite uses only plain HTML elements (`<sup>` for the marker, `<div>` for
//! each rendered note), so it needs no new layout, style, or emit support: the
//! existing pipeline lays it out like any other markup.

use crate::node::{Attr, Element, Node, TKind, Tag};

/// Rewrite `<t:endnote>`/`<t:endnotes/>` in place: replace each `<t:endnote>` with
/// a `<sup>` marker carrying its number, collect the note bodies in document
/// order, then expand the first `<t:endnotes/>` into the rendered list. A tree
/// with neither directive is returned untouched.
pub fn expand(nodes: Vec<Node>) -> Vec<Node> {
    let mut collected: Vec<Vec<Node>> = Vec::new();
    let mut marked = mark_nodes(nodes, &mut collected);
    if !collected.is_empty() {
        place_list(&mut marked, &collected);
    }
    marked
}

/// Replace every `<t:endnote>` with its superscript marker, pushing its body onto
/// `collected` and assigning the next document-order number.
fn mark_nodes(nodes: Vec<Node>, collected: &mut Vec<Vec<Node>>) -> Vec<Node> {
    nodes
        .into_iter()
        .map(|node| mark_node(node, collected))
        .collect()
}

fn mark_node(node: Node, collected: &mut Vec<Vec<Node>>) -> Node {
    let Node::Element(el) = node else {
        return node;
    };
    if matches!(el.tag, Tag::Directive(TKind::Endnote)) {
        collected.push(el.children);
        return marker(collected.len());
    }
    Node::Element(Element {
        tag: el.tag,
        attrs: el.attrs,
        children: mark_nodes(el.children, collected),
    })
}

/// A superscript marker element holding the note number as text.
fn marker(number: usize) -> Node {
    Node::Element(Element {
        tag: Tag::Html("sup".to_string()),
        attrs: vec![class_attr("endnote-ref")],
        children: vec![Node::Text(number.to_string())],
    })
}

/// Expand the first `<t:endnotes/>` in pre-order into the rendered list, dropping
/// any later `<t:endnotes/>` (the list is emitted once). `emitted` carries the
/// "already placed" state across the recursion.
fn place_list(nodes: &mut Vec<Node>, collected: &[Vec<Node>]) {
    let mut emitted = false;
    place_into(nodes, collected, &mut emitted);
}

fn place_into(nodes: &mut Vec<Node>, collected: &[Vec<Node>], emitted: &mut bool) {
    let mut out = Vec::with_capacity(nodes.len());
    for node in std::mem::take(nodes) {
        push_placed(&mut out, node, collected, emitted);
    }
    nodes.extend(out);
}

/// Place one node into `out`: an `<t:endnotes/>` slot becomes the list (once) or
/// is dropped; any other node recurses into its children.
fn push_placed(out: &mut Vec<Node>, node: Node, collected: &[Vec<Node>], emitted: &mut bool) {
    if endnotes_slot(&node) {
        if !*emitted {
            *emitted = true;
            out.push(notes_list(collected));
        }
        return;
    }
    out.push(descend(node, collected, emitted));
}

/// Whether a node is an `<t:endnotes/>` slot.
fn endnotes_slot(node: &Node) -> bool {
    node.as_element()
        .is_some_and(|el| matches!(el.tag, Tag::Directive(TKind::Endnotes)))
}

/// Recurse into an element's children to place a nested `<t:endnotes/>`.
fn descend(node: Node, collected: &[Vec<Node>], emitted: &mut bool) -> Node {
    let Node::Element(el) = node else {
        return node;
    };
    let mut children = el.children;
    if !*emitted {
        place_into(&mut children, collected, emitted);
    }
    Node::Element(Element {
        tag: el.tag,
        attrs: el.attrs,
        children,
    })
}

/// The rendered notes as a `<div class="endnotes">` wrapping one
/// `<div class="endnote">` per note, each prefixed with its number.
fn notes_list(collected: &[Vec<Node>]) -> Node {
    let items = collected
        .iter()
        .enumerate()
        .map(|(i, body)| note_item(i + 1, body))
        .collect();
    Node::Element(Element {
        tag: Tag::Html("div".to_string()),
        attrs: vec![class_attr("endnotes")],
        children: items,
    })
}

/// One rendered note: `<div class="endnote">N. <body…></div>`.
fn note_item(number: usize, body: &[Node]) -> Node {
    let mut children = vec![Node::Text(format!("{number}. "))];
    children.extend(body.iter().cloned());
    Node::Element(Element {
        tag: Tag::Html("div".to_string()),
        attrs: vec![class_attr("endnote")],
        children,
    })
}

/// A `class="…"` attribute.
fn class_attr(value: &str) -> Attr {
    Attr {
        name: "class".to_string(),
        value: value.to_string(),
    }
}
