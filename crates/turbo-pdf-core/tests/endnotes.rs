//! Phase 15 `endnotes` feature tests (§3). Only compiled with `--features
//! endnotes`. Exercises the node→node `expand` rewrite directly (markers,
//! numbering, list placement, edge cases) and one end-to-end render to prove the
//! collected notes flow into the body where `<t:endnotes/>` stood.

#![cfg(feature = "endnotes")]

mod common;

use turbo_pdf_core::endnotes::expand;
use turbo_pdf_core::layout::fragment::{Fragment, FragmentContent};
use turbo_pdf_core::node::{Attr, Element, Node, TKind, Tag};
use turbo_pdf_core::style::TokenSet;
use turbo_pdf_core::{
    build_cascade, compile, render_pages, style::AtRule, CompileOptions, Diagnostics, Page,
    RenderInputs,
};

// -- node helpers --------------------------------------------------------------

fn text(s: &str) -> Node {
    Node::Text(s.to_string())
}

fn endnote(body: &str) -> Node {
    Node::Element(Element {
        tag: Tag::Directive(TKind::Endnote),
        attrs: Vec::new(),
        children: vec![text(body)],
    })
}

fn endnotes_slot() -> Node {
    Node::Element(Element {
        tag: Tag::Directive(TKind::Endnotes),
        attrs: Vec::new(),
        children: Vec::new(),
    })
}

fn div(children: Vec<Node>) -> Node {
    Node::Element(Element {
        tag: Tag::Html("div".to_string()),
        attrs: Vec::new(),
        children,
    })
}

/// Find the first element with the given `class`, returning its text leaves
/// concatenated.
fn first_class_text(nodes: &[Node], class: &str) -> Option<String> {
    nodes.iter().find_map(|n| class_text(n, class))
}

fn class_text(node: &Node, class: &str) -> Option<String> {
    let el = node.as_element()?;
    if has_class(el, class) {
        return Some(text_of(node));
    }
    el.children.iter().find_map(|c| class_text(c, class))
}

fn has_class(el: &Element, class: &str) -> bool {
    el.attrs
        .iter()
        .any(|a: &Attr| a.name == "class" && a.value == class)
}

fn text_of(node: &Node) -> String {
    match node {
        Node::Text(t) => t.clone(),
        Node::Element(el) => el.children.iter().map(text_of).collect(),
    }
}

// -- transform tests -----------------------------------------------------------

#[test]
fn no_directives_is_identity() {
    let input = vec![div(vec![text("hello")])];
    assert_eq!(expand(input.clone()), input);
}

#[test]
fn endnote_becomes_numbered_superscript_marker() {
    let out = expand(vec![
        div(vec![text("body"), endnote("note one")]),
        endnotes_slot(),
    ]);
    // The first child's marker is a <sup class="endnote-ref"> holding "1".
    let marker = first_class_text(&out, "endnote-ref").expect("marker present");
    assert_eq!(marker, "1");
    // The note body lands in the list, prefixed by its number.
    let item = first_class_text(&out, "endnote").expect("note item present");
    assert_eq!(item, "1. note one");
}

#[test]
fn multiple_notes_number_in_document_order() {
    let out = expand(vec![
        endnote("first"),
        endnote("second"),
        endnote("third"),
        endnotes_slot(),
    ]);
    let list = out
        .iter()
        .find_map(|n| n.as_element().filter(|e| has_class(e, "endnotes")))
        .expect("endnotes list");
    let items: Vec<String> = list.children.iter().map(text_of).collect();
    assert_eq!(items, vec!["1. first", "2. second", "3. third"]);
}

#[test]
fn nested_endnote_and_slot_are_found() {
    let out = expand(vec![div(vec![
        div(vec![endnote("deep")]),
        div(vec![endnotes_slot()]),
    ])]);
    assert_eq!(first_class_text(&out, "endnote-ref").as_deref(), Some("1"));
    assert_eq!(
        first_class_text(&out, "endnote").as_deref(),
        Some("1. deep")
    );
}

#[test]
fn second_slot_is_dropped() {
    let out = expand(vec![endnote("x"), endnotes_slot(), endnotes_slot()]);
    let lists = out
        .iter()
        .filter(|n| n.as_element().is_some_and(|e| has_class(e, "endnotes")))
        .count();
    assert_eq!(lists, 1, "the list is emitted exactly once");
}

#[test]
fn notes_without_a_slot_collect_but_emit_no_list() {
    // A note with no <t:endnotes/> still leaves its marker; the list is simply
    // not placed anywhere (collected notes have nowhere to go).
    let out = expand(vec![div(vec![endnote("orphan")])]);
    assert_eq!(first_class_text(&out, "endnote-ref").as_deref(), Some("1"));
    assert!(first_class_text(&out, "endnote").is_none());
}

// -- end-to-end render ---------------------------------------------------------

fn at_rules(css: &str) -> Vec<AtRule> {
    turbo_pdf_core::style::parse_stylesheet(css).at_rules
}

fn pages(template: &str) -> Vec<Page> {
    let (program, _) = compile(template, &CompileOptions::default()).expect("compile");
    let cascade = build_cascade("", "", TokenSet::default());
    let fonts = common::registry();
    let rules = at_rules("");
    let inputs = RenderInputs {
        program: &program,
        data: &serde_json::json!({}),
        cascade: &cascade,
        at_rules: &rules,
        fonts: &fonts,
        images: &turbo_pdf_core::NoImages,
        now: Some(0),
    };
    let mut diags = Diagnostics::default();
    render_pages(&inputs, &mut diags).expect("render_pages")
}

fn body_text(pages: &[Page]) -> String {
    let mut out = String::new();
    for page in pages {
        for frag in &page.body {
            collect_text(frag, &mut out);
        }
    }
    out
}

fn collect_text(frag: &Fragment, out: &mut String) {
    if let FragmentContent::TextLine { glyphs, .. } = &frag.content {
        // We can't reverse glyph ids to chars cheaply, so just count that text
        // lines exist; the structural assertion below is on glyph presence.
        if !glyphs.is_empty() {
            out.push('x');
        }
    }
    for child in &frag.children {
        collect_text(child, out);
    }
}

#[test]
fn render_flows_note_body_into_the_endnotes_slot() {
    // Body has a marker and an endnotes list; both produce glyph runs, so the
    // body band carries strictly more text lines than a no-note control.
    let with_notes = pages("<div>Intro<t:endnote>cited work</t:endnote></div><t:endnotes/>");
    let control = pages("<div>Intro</div>");
    assert!(
        body_text(&with_notes).len() > body_text(&control).len(),
        "endnotes add a marker + a rendered note list to the body flow"
    );
}
