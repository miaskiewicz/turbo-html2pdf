//! Node-tree tests (§1 Stage 1, §2.1): Jinja render → html5ever → typed `t:`
//! nodes. Covers t: recognition, self-closing expansion, unknown-element errors,
//! and the real-markup half of AC-2.6.

use serde_json::{json, Value as Json};
use turbo_pdf_core::node::t_kind;
use turbo_pdf_core::{compile, CompileOptions, Element, ErrorCode, Node, TKind, Tag};

fn nodes(tpl: &str, data: &Json) -> Vec<Node> {
    let (program, _) = compile(tpl, &CompileOptions::default()).expect("compile");
    program.render_nodes(data, None).expect("render_nodes").0
}

fn nodes_nil(tpl: &str) -> Vec<Node> {
    nodes(tpl, &json!({}))
}

fn directive_of(node: &Node) -> Option<TKind> {
    match node {
        Node::Element(Element {
            tag: Tag::Directive(k),
            ..
        }) => Some(*k),
        _ => None,
    }
}

fn html_name(node: &Node) -> Option<&str> {
    match node {
        Node::Element(Element {
            tag: Tag::Html(name),
            ..
        }) => Some(name.as_str()),
        _ => None,
    }
}

#[test]
fn t_element_becomes_typed_directive() {
    let tree = nodes_nil("<t:footnote>note</t:footnote>");
    assert_eq!(tree.len(), 1);
    assert_eq!(directive_of(&tree[0]), Some(TKind::Footnote));
    let body = tree[0].as_element().unwrap();
    assert_eq!(body.children[0].as_text(), Some("note"));
}

#[test]
fn self_closing_t_elements_are_siblings_not_nested() {
    // The crux: `<t:page/>` must not swallow following siblings as children.
    let tree = nodes_nil("X<t:page/>Y<t:pages/>Z");
    let kinds: Vec<Option<TKind>> = tree.iter().map(directive_of).collect();
    let texts: Vec<Option<&str>> = tree.iter().map(Node::as_text).collect();
    assert_eq!(texts[0], Some("X"));
    assert_eq!(kinds[1], Some(TKind::Page));
    assert_eq!(texts[2], Some("Y"));
    assert_eq!(kinds[3], Some(TKind::Pages));
    assert_eq!(texts[4], Some("Z"));
    // the page directive has no children
    assert!(tree[1].as_element().unwrap().children.is_empty());
}

#[test]
fn html_element_attrs_and_children() {
    let tree = nodes_nil(r#"<div class="x" id="y">hi</div>"#);
    assert_eq!(html_name(&tree[0]), Some("div"));
    let div = tree[0].as_element().unwrap();
    assert_eq!(div.attr("class"), Some("x"));
    assert_eq!(div.attr("id"), Some("y"));
    assert_eq!(div.attr("missing"), None);
    assert_eq!(div.children[0].as_text(), Some("hi"));
}

#[test]
fn unknown_t_element_errors() {
    let (program, _) = compile("<t:bogus></t:bogus>", &CompileOptions::default()).unwrap();
    let err = program.render_nodes(&json!({}), None).unwrap_err();
    assert_eq!(err.code, ErrorCode::UnknownElement);
}

#[test]
fn ac_2_6_safe_output_becomes_real_markup_node() {
    let tree = nodes_nil("{{ '<b>x</b>' | safe }}");
    assert_eq!(html_name(&tree[0]), Some("b"));
    assert_eq!(
        tree[0].as_element().unwrap().children[0].as_text(),
        Some("x")
    );
}

#[test]
fn self_closing_with_attrs_expands() {
    let tree = nodes_nil(r#"<t:region slot="header"/>after"#);
    assert_eq!(directive_of(&tree[0]), Some(TKind::Region));
    assert_eq!(tree[0].as_element().unwrap().attr("slot"), Some("header"));
    assert_eq!(tree[1].as_text(), Some("after"));
}

#[test]
fn data_driven_t_element() {
    let tpl = "{% if show %}<t:leader/>{% endif %}done";
    let tree = nodes(tpl, &json!({"show": true}));
    assert_eq!(directive_of(&tree[0]), Some(TKind::Leader));
}

#[test]
fn all_t_directive_names_map() {
    let cases = [
        ("footnote", TKind::Footnote),
        ("footnote-separator", TKind::FootnoteSeparator),
        ("running-header", TKind::RunningHeader),
        ("running-footer", TKind::RunningFooter),
        ("page-master", TKind::PageMaster),
        ("region", TKind::Region),
        ("variant", TKind::Variant),
        ("page", TKind::Page),
        ("pages", TKind::Pages),
        ("use-master", TKind::UseMaster),
        ("counter", TKind::Counter),
        ("leader", TKind::Leader),
        ("anchor", TKind::Anchor),
        ("endnote", TKind::Endnote),
        ("endnotes", TKind::Endnotes),
    ];
    for (name, kind) in cases {
        assert_eq!(t_kind(name), Some(kind));
    }
    assert_eq!(t_kind("nope"), None);
}

#[test]
fn node_accessors_cover_both_arms() {
    let text = Node::Text("t".into());
    let elem = nodes_nil("<p>x</p>").into_iter().next().unwrap();
    assert!(text.as_element().is_none());
    assert!(elem.as_text().is_none());
    assert_eq!(text.as_text(), Some("t"));
    assert!(elem.as_element().is_some());
}

#[test]
fn html_comments_are_dropped() {
    // Comment inside an element so it lands in the body subtree we convert.
    let tree = nodes_nil("<div><!-- hidden -->kept</div>");
    let div = tree[0].as_element().unwrap();
    assert_eq!(div.children.len(), 1);
    assert_eq!(div.children[0].as_text(), Some("kept"));
}

#[test]
fn truncated_t_tag_without_close_does_not_panic() {
    // Exercises the no-`>` path in the self-closing expander.
    let result = nodes_nil(r#"{{ "<t:" | safe }}done"#);
    assert!(result.iter().any(|n| n.as_text() == Some("done")) || result.is_empty());
}
