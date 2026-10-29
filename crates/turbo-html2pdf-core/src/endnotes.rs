//! Endnotes (`endnotes` feature, §3): `<t:endnote>` collects a note out of the
//! body flow and leaves a superscript number where it stood; `<t:endnotes/>`
//! renders the collected notes, in document order, as a numbered list at its own
//! position in the flow.
//!
//! Unlike footnotes — which are page-anchored and resolved by the body/footnote
//! fixpoint (§6.4) — endnotes are *document-anchored*: all notes gather to the one
//! `<t:endnotes/>` slot. That makes this a pure rewrite of the flow's document
//! that runs once, right after templating, before the rest of the spine
//! (style → layout → paginate) sees the tree. The numbering builds on the same
//! first-encounter, document-order counting the footnote infra uses.
//!
//! The rewrite uses only plain HTML elements (`<sup>` for the marker, `<div>` for
//! each rendered note), built by mutating the document, so it needs no new
//! layout, style, or emit support: the existing pipeline lays it out like any
//! other markup.

use turbo_dom::rtdom::tree::NodeType;
use turbo_dom::{Handle, Tree};
use turbo_dom_layout::TreeView;

use crate::flow::Flow;
use crate::markers::{directive_of, TKind};

/// Rewrite `<t:endnote>`/`<t:endnotes/>` in place: replace each `<t:endnote>` with
/// a `<sup>` marker carrying its number, collect the note bodies in document
/// order, then expand the first `<t:endnotes/>` into the rendered list. A flow
/// with neither directive is left untouched.
pub fn expand(flow: &mut Flow) {
    let roots = flow.roots().to_vec();
    let tree = flow.tree_mut();
    let mut collected: Vec<Vec<Handle>> = Vec::new();
    mark_nodes(tree, roots, &mut collected);
    if collected.is_empty() {
        return;
    }
    let roots = kept_children(tree, crate::template::markup::body_of(tree));
    place_into(tree, roots, &collected, &mut false);
    let roots = kept_children(tree, crate::template::markup::body_of(tree));
    flow.set_roots(roots);
}

/// The content children (elements and text) of `h`.
fn kept_children(tree: &Tree, h: Handle) -> Vec<Handle> {
    TreeView::new(tree).children(h)
}

/// The directive an element node is, if any.
fn directive(tree: &Tree, h: Handle) -> Option<TKind> {
    tree.local_name(h).and_then(directive_of)
}

/// The element nodes among `nodes` (node types never change, so filtering up
/// front is the same as skipping text while rewriting).
fn elements(tree: &Tree, nodes: Vec<Handle>) -> Vec<Handle> {
    nodes
        .into_iter()
        .filter(|&h| tree.node_type(h) == NodeType::Element)
        .collect()
}

/// Replace every `<t:endnote>` with its superscript marker, pushing its body onto
/// `collected` and assigning the next document-order number.
fn mark_nodes(tree: &mut Tree, nodes: Vec<Handle>, collected: &mut Vec<Vec<Handle>>) {
    for node in elements(tree, nodes) {
        if directive(tree, node) == Some(TKind::Endnote) {
            collected.push(kept_children(tree, node));
            let marker = marker(tree, collected.len());
            tree.replace_with(node, &[marker]);
            continue;
        }
        let children = kept_children(tree, node);
        mark_nodes(tree, children, collected);
    }
}

/// A superscript marker element holding the note number as text.
fn marker(tree: &mut Tree, number: usize) -> Handle {
    let sup = element_with_class(tree, "sup", "endnote-ref");
    let text = tree.create_text_node(&number.to_string());
    tree.append_child(sup, text);
    sup
}

/// Place the list at the first `<t:endnotes/>` in pre-order, dropping every later
/// slot met at a level still being walked (the list is emitted once). After the
/// list is placed, elements are no longer descended into.
fn place_into(tree: &mut Tree, nodes: Vec<Handle>, collected: &[Vec<Handle>], emitted: &mut bool) {
    for node in elements(tree, nodes) {
        if directive(tree, node) == Some(TKind::Endnotes) {
            place_slot(tree, node, collected, emitted);
        } else if !*emitted {
            let children = kept_children(tree, node);
            place_into(tree, children, collected, emitted);
        }
    }
}

/// An `<t:endnotes/>` slot becomes the list (once) or is dropped.
fn place_slot(tree: &mut Tree, slot: Handle, collected: &[Vec<Handle>], emitted: &mut bool) {
    if *emitted {
        let parent = tree.parent(slot).expect("a flow slot has a parent");
        tree.remove_child(parent, slot);
        return;
    }
    *emitted = true;
    let list = notes_list(tree, collected);
    tree.replace_with(slot, &[list]);
}

/// The rendered notes as a `<div class="endnotes">` wrapping one
/// `<div class="endnote">` per note, each prefixed with its number.
fn notes_list(tree: &mut Tree, collected: &[Vec<Handle>]) -> Handle {
    let list = element_with_class(tree, "div", "endnotes");
    for (i, body) in collected.iter().enumerate() {
        let item = note_item(tree, i + 1, body);
        tree.append_child(list, item);
    }
    list
}

/// One rendered note: `<div class="endnote">N. <body…></div>` (the body nodes
/// move out of their detached `<t:endnote>`).
fn note_item(tree: &mut Tree, number: usize, body: &[Handle]) -> Handle {
    let item = element_with_class(tree, "div", "endnote");
    let prefix = tree.create_text_node(&format!("{number}. "));
    tree.append_child(item, prefix);
    for &node in body {
        tree.append_child(item, node);
    }
    item
}

/// A new `<name class="…">` element.
fn element_with_class(tree: &mut Tree, name: &str, class: &str) -> Handle {
    let element = tree.create_element(name);
    tree.set_attribute(element, "class", class);
    element
}
