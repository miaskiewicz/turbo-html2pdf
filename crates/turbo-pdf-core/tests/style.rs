//! Style-system tests (§4): CSS parsing, selector matching + specificity,
//! tokens, node-style injection, the 6-level cascade, and inheritance.

use serde_json::json;
use turbo_pdf_core::style::{
    build_cascade, parse_stylesheet, style_tree, Declaration, StyleToken, StyledElement,
    StyledNode, TokenSet,
};
use turbo_pdf_core::{compile, CompileOptions};

fn render_nodes(tpl: &str) -> Vec<turbo_pdf_core::Node> {
    let (program, _) = compile(tpl, &CompileOptions::default()).expect("compile");
    program.render_nodes(&json!({}), None).expect("render").0
}

fn styled_with(tpl: &str, css: &str, node_css: &str, tokens: TokenSet) -> Vec<StyledNode> {
    let nodes = render_nodes(tpl);
    let cascade = build_cascade(css, node_css, tokens);
    style_tree(&nodes, &cascade)
}

fn styled(tpl: &str, css: &str) -> Vec<StyledNode> {
    styled_with(tpl, css, "", TokenSet::new())
}

fn find<'a>(
    nodes: &'a [StyledNode],
    pred: &dyn Fn(&StyledElement) -> bool,
) -> Option<&'a StyledElement> {
    for node in nodes {
        if let StyledNode::Element(el) = node {
            if pred(el) {
                return Some(el);
            }
            if let Some(found) = find(&el.children, pred) {
                return Some(found);
            }
        }
    }
    None
}

fn by_id<'a>(nodes: &'a [StyledNode], id: &str) -> &'a StyledElement {
    find(nodes, &|el| {
        el.attrs.iter().any(|a| a.name == "id" && a.value == id)
    })
    .expect("element by id")
}

fn prop(nodes: &[StyledNode], id: &str, property: &str) -> Option<String> {
    by_id(nodes, id).style.get(property).map(str::to_string)
}

fn decl(property: &str, value: &str) -> Declaration {
    Declaration {
        property: property.into(),
        value: value.into(),
        important: false,
    }
}

fn token(extends: &[&str], decls: &[(&str, &str)]) -> StyleToken {
    StyleToken {
        extends: extends.iter().map(|s| s.to_string()).collect(),
        declarations: decls.iter().map(|(p, v)| decl(p, v)).collect(),
    }
}

// --------------------------------------------------------------------------
// parser
// --------------------------------------------------------------------------

#[test]
fn parses_rules_comments_and_at_rules() {
    let sheet = parse_stylesheet(
        "/* c */ .a { color: red; font-weight: 700 !important } @page { size: A4 }",
    );
    assert_eq!(sheet.rules.len(), 1);
    assert_eq!(sheet.rules[0].declarations.len(), 2);
    assert!(sheet.rules[0].declarations[1].important);
    assert_eq!(sheet.at_rules.len(), 1);
    assert_eq!(sheet.at_rules[0].name, "page");
    assert_eq!(sheet.at_rules[0].body, "size: A4");
}

#[test]
fn malformed_declarations_are_skipped() {
    let sheet = parse_stylesheet(".a { ; color: red; novalue: ; :bad ; width: 10px }");
    assert_eq!(sheet.rules[0].declarations.len(), 2);
    let empty = parse_stylesheet("{ color: red }");
    assert!(empty.rules.is_empty());
}

#[test]
fn unterminated_comment_consumes_rest() {
    let sheet = parse_stylesheet(".a { color: red } /* dangling");
    assert_eq!(sheet.rules.len(), 1);
}

#[test]
fn nested_braces_in_at_rule_stay_intact() {
    let sheet = parse_stylesheet("@media print { .a { color: red } }");
    assert_eq!(sheet.rules.len(), 0);
    assert_eq!(sheet.at_rules.len(), 1);
    assert_eq!(sheet.at_rules[0].name, "media");
    assert!(sheet.at_rules[0].body.contains(".a"));
}

#[test]
fn empty_selector_list_produces_no_rule() {
    let sheet = parse_stylesheet(", { color: red }");
    assert!(sheet.rules.is_empty());
}

// --------------------------------------------------------------------------
// selectors + specificity
// --------------------------------------------------------------------------

#[test]
fn ac_4_2_specificity_id_beats_class_beats_type() {
    let css = "p { color: red } .c { color: green } #t { color: blue }";
    let tree = styled(r#"<p class="c" id="t">x</p>"#, css);
    assert_eq!(prop(&tree, "t", "color").as_deref(), Some("blue"));
}

#[test]
fn ac_4_2_later_rule_wins_on_equal_specificity() {
    let css = ".c { color: red } .c { color: green }";
    let tree = styled(r#"<p class="c" id="t">x</p>"#, css);
    assert_eq!(prop(&tree, "t", "color").as_deref(), Some("green"));
}

#[test]
fn child_vs_descendant_combinator() {
    let css = "div > span { color: red } section span { color: green }";
    let tree = styled(
        r#"<div><span id="direct">a</span><p><span id="nested">b</span></p></div><section><span id="desc">c</span></section>"#,
        css,
    );
    assert_eq!(prop(&tree, "direct", "color").as_deref(), Some("red"));
    assert_eq!(prop(&tree, "nested", "color"), None);
    assert_eq!(prop(&tree, "desc", "color").as_deref(), Some("green"));
}

#[test]
fn ac_4_4_nth_child_zebra_and_first_last() {
    let css = "tr:nth-child(even) { color: gray } tr:first-child { color: red } tr:last-child { color: blue }";
    let tpl = r#"<table><tbody>
        <tr id="r1"><td>1</td></tr><tr id="r2"><td>2</td></tr>
        <tr id="r3"><td>3</td></tr><tr id="r4"><td>4</td></tr>
    </tbody></table>"#;
    let tree = styled(tpl, css);
    assert_eq!(prop(&tree, "r1", "color").as_deref(), Some("red"));
    assert_eq!(prop(&tree, "r2", "color").as_deref(), Some("gray"));
    assert_eq!(prop(&tree, "r3", "color"), None);
    assert_eq!(prop(&tree, "r4", "color").as_deref(), Some("blue"));
}

#[test]
fn nth_child_odd_and_an_plus_b() {
    let css = "li:nth-child(odd) { color: red } li:nth-child(2n+1) { font-weight: 700 }";
    let tpl = r#"<ul><li id="a">1</li><li id="b">2</li><li id="c">3</li></ul>"#;
    let tree = styled(tpl, css);
    assert_eq!(prop(&tree, "a", "color").as_deref(), Some("red"));
    assert_eq!(prop(&tree, "b", "color"), None);
    assert_eq!(prop(&tree, "c", "font-weight").as_deref(), Some("700"));
}

#[test]
fn nth_of_type_counts_per_tag() {
    let css = "p:nth-of-type(2) { color: red }";
    let tpl = r#"<div><span>s</span><p id="p1">1</p><p id="p2">2</p></div>"#;
    let tree = styled(tpl, css);
    assert_eq!(prop(&tree, "p2", "color").as_deref(), Some("red"));
    assert_eq!(prop(&tree, "p1", "color"), None);
}

#[test]
fn attribute_selectors_all_ops() {
    let css = "[data-x] { --exists: 1 } \
        [type=\"text\"] { --eq: 1 } \
        [rel~=\"next\"] { --inc: 1 } \
        [lang|=\"en\"] { --dash: 1 } \
        [href^=\"https\"] { --pre: 1 } \
        [src$=\".png\"] { --suf: 1 } \
        [title*=\"mid\"] { --sub: 1 }";
    let tpl = r#"<a id="e" data-x type="text" rel="prev next" lang="en-US" href="https://x" src="a.png" title="a mid z">x</a>"#;
    let tree = styled(tpl, css);
    for v in [
        "--exists", "--eq", "--inc", "--dash", "--pre", "--suf", "--sub",
    ] {
        assert_eq!(prop(&tree, "e", v).as_deref(), Some("1"), "op {v}");
    }
}

#[test]
fn attribute_selector_negative_cases() {
    let css = "[type=\"text\"] { --eq: 1 } [missing] { --m: 1 }";
    let tree = styled(r#"<input id="e" type="number">"#, css);
    assert_eq!(prop(&tree, "e", "--eq"), None);
    assert_eq!(prop(&tree, "e", "--m"), None);
}

#[test]
fn universal_selector_matches_all() {
    let tree = styled(r#"<p id="e">x</p>"#, "* { color: teal }");
    assert_eq!(prop(&tree, "e", "color").as_deref(), Some("teal"));
}

#[test]
fn unknown_pseudo_class_is_ignored() {
    let tree = styled(
        r#"<a id="e">x</a><p id="p">y</p>"#,
        "a:hover { color: red }",
    );
    assert_eq!(prop(&tree, "e", "color").as_deref(), Some("red"));
    assert_eq!(prop(&tree, "p", "color"), None);
}

#[test]
fn unrecognized_selector_chars_are_skipped() {
    // Sibling combinators are deferred; stray chars are tolerated, not fatal.
    let tree = styled(r#"<div id="e">x</div>"#, "div+span { color: red }");
    assert_eq!(prop(&tree, "e", "color").as_deref(), Some("red"));
}

// --------------------------------------------------------------------------
// inheritance + UA
// --------------------------------------------------------------------------

#[test]
fn ac_4_3_inheritance() {
    let tree = styled(
        r#"<div style="color: purple"><span id="c">x</span></div>"#,
        "",
    );
    assert_eq!(prop(&tree, "c", "color").as_deref(), Some("purple"));
}

#[test]
fn non_inherited_property_does_not_descend() {
    let tree = styled(
        r#"<div style="margin: 5px"><span id="c">x</span></div>"#,
        "",
    );
    assert_eq!(prop(&tree, "c", "margin"), None);
}

#[test]
fn ua_default_overridden_by_author() {
    let plain = styled(r#"<b id="e">x</b>"#, "");
    assert_eq!(prop(&plain, "e", "font-weight").as_deref(), Some("bold"));
    let over = styled(r#"<b id="e">x</b>"#, "b { font-weight: 400 }");
    assert_eq!(prop(&over, "e", "font-weight").as_deref(), Some("400"));
}

// --------------------------------------------------------------------------
// tokens + node styles + full cascade
// --------------------------------------------------------------------------

#[test]
fn ac_4_6_token_beats_sheet_inline_beats_token() {
    let mut tokens = TokenSet::new();
    tokens.insert("total".into(), token(&[], &[("font-weight", "700")]));
    let tree = styled_with(
        r#"<span id="e" class="c" t:style="total">x</span>"#,
        ".c { font-weight: 400 }",
        "",
        tokens.clone(),
    );
    assert_eq!(prop(&tree, "e", "font-weight").as_deref(), Some("700"));

    let inline = styled_with(
        r#"<span id="e" class="c" t:style="total" style="font-weight: 100">x</span>"#,
        ".c { font-weight: 400 }",
        "",
        tokens,
    );
    assert_eq!(prop(&inline, "e", "font-weight").as_deref(), Some("100"));
}

#[test]
fn tokens_extends_and_later_token_wins() {
    let mut tokens = TokenSet::new();
    tokens.insert(
        "emphatic".into(),
        token(&[], &[("color", "#0a0a0a"), ("font-weight", "700")]),
    );
    tokens.insert(
        "total".into(),
        token(&["emphatic"], &[("font-size", "14pt")]),
    );
    tokens.insert("muted".into(), token(&[], &[("color", "#666")]));
    // extends pulls emphatic; later token `muted` overrides color.
    let tree = styled_with(
        r#"<span id="e" t:style="total muted">x</span>"#,
        "",
        "",
        tokens,
    );
    assert_eq!(prop(&tree, "e", "color").as_deref(), Some("#666"));
    assert_eq!(prop(&tree, "e", "font-weight").as_deref(), Some("700"));
    assert_eq!(prop(&tree, "e", "font-size").as_deref(), Some("14pt"));
}

#[test]
fn unknown_token_is_ignored() {
    let tree = styled_with(
        r#"<span id="e" t:style="nope">x</span>"#,
        "",
        "",
        TokenSet::new(),
    );
    assert_eq!(prop(&tree, "e", "color"), None);
}

#[test]
fn token_extends_cycle_terminates() {
    let mut tokens = TokenSet::new();
    tokens.insert("a".into(), token(&["b"], &[("color", "red")]));
    tokens.insert("b".into(), token(&["a"], &[("font-weight", "700")]));
    let tree = styled_with(r#"<span id="e" t:style="a">x</span>"#, "", "", tokens);
    assert_eq!(prop(&tree, "e", "color").as_deref(), Some("red"));
}

#[test]
fn ac_4_8_node_styles_render_time() {
    let tree = styled_with(
        r#"<span id="e" class="c">x</span>"#,
        ".c { color: red }",
        ".c { color: green }",
        TokenSet::new(),
    );
    assert_eq!(prop(&tree, "e", "color").as_deref(), Some("green"));
}

#[test]
fn ac_4_9_full_six_level_order() {
    // UA(a) < author(.c) < token < nodeStyle < inline < !important
    let mut tokens = TokenSet::new();
    tokens.insert("tok".into(), token(&[], &[("color", "tokencolor")]));
    let tree = styled_with(
        r#"<a id="e" class="c" t:style="tok" style="color: inlinecolor">x</a>"#,
        ".c { color: authorcolor } a { color: uacolor !important }",
        ".c { color: nodecolor }",
        tokens,
    );
    // the author !important on `a` beats inline non-important (top level).
    assert_eq!(prop(&tree, "e", "color").as_deref(), Some("uacolor"));
}

#[test]
fn important_inline_beats_important_author() {
    let tree = styled(
        r#"<p id="e" class="c" style="color: win !important">x</p>"#,
        ".c { color: lose !important }",
    );
    assert_eq!(prop(&tree, "e", "color").as_deref(), Some("win"));
}

#[test]
fn text_nodes_have_no_style() {
    let tree = styled("hello", "");
    assert!(matches!(tree[0], StyledNode::Text(_)));
}

#[test]
fn directive_elements_are_styled_too() {
    let tree = styled(
        r#"<t:region slot="header"><span id="e">x</span></t:region>"#,
        "#e { color: red }",
    );
    assert_eq!(prop(&tree, "e", "color").as_deref(), Some("red"));
}
