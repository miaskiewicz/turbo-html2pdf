//! Style-system tests (§4): CSS parsing, selector matching + specificity,
//! tokens, node-style injection, the 6-level cascade, and inheritance.

use serde_json::json;
use turbo_html2pdf_core::style::{
    build_cascade, parse_stylesheet, style_tree, Declaration, StyleToken, StyledElement,
    StyledNode, TokenSet,
};
use turbo_html2pdf_core::{compile, CompileOptions};

fn render_nodes(tpl: &str) -> Vec<turbo_html2pdf_core::Node> {
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
    // A genuinely unknown pseudo-class is dropped, and the rest of the compound
    // still matches (lenient parsing).
    let tree = styled(
        r#"<a id="e">x</a><p id="p">y</p>"#,
        "a:totallyunknown { color: red }",
    );
    assert_eq!(prop(&tree, "e", "color").as_deref(), Some("red"));
    assert_eq!(prop(&tree, "p", "color"), None);
}

#[test]
fn pseudo_element_rule_does_not_style_the_element() {
    // A `::before`/`::after` (and legacy single-colon) selector targets generated
    // content turbo does not create, so its declarations must NOT fall onto the
    // originating element. Regression: `.bar::after{height:1px}` was collapsing to
    // `.bar` and applying `height:1px` to the real element (Wikipedia's page-title
    // bar shrank to 1px, overlapping the `<h1>`).
    let tree = styled(
        r#"<div id="e" class="bar">x</div>"#,
        ".bar::after { height: 1px } .bar:before { color: red } .bar::first-line { color: lime }",
    );
    assert_eq!(prop(&tree, "e", "height"), None);
    assert_eq!(prop(&tree, "e", "color"), None);
}

#[test]
fn next_sibling_combinator_tight() {
    // `div+span` (no spaces) matches a span immediately after a div.
    let tree = styled(
        r#"<div id="d">x</div><span id="s">y</span>"#,
        "div+span { color: red }",
    );
    assert_eq!(prop(&tree, "d", "color"), None);
    assert_eq!(prop(&tree, "s", "color").as_deref(), Some("red"));
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
    // font-size is a computed absolute value: 14pt = 14·96/72 ≈ 18.67px.
    let fs = prop(&tree, "e", "font-size").unwrap();
    let px: f32 = fs.strip_suffix("px").unwrap().parse().unwrap();
    assert!((px - 18.6667).abs() < 0.01, "14pt -> {px}px");
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

// ---------------------------------------------------------------- presentational hints

#[test]
fn bgcolor_attribute_maps_to_background() {
    // Legacy `bgcolor` (Hacker News' orange header) is honored as a presentational
    // hint on the box's `background-color`.
    let tree = styled(
        r##"<table bgcolor="#ff6600"><tr><td>hi</td></tr></table>"##,
        "",
    );
    let table = find(
        &tree,
        &|e| matches!(&e.tag, turbo_html2pdf_core::Tag::Html(t) if t == "table"),
    )
    .expect("table");
    assert_eq!(table.style.get("background-color"), Some("#ff6600"));
}

#[test]
fn author_css_overrides_presentational_hint() {
    // A real author rule must beat the presentational hint (hints sit just above UA).
    let tree = styled(
        r##"<table bgcolor="#ff6600"><tr><td>hi</td></tr></table>"##,
        "table { background-color: #00ff00 }",
    );
    let table = find(
        &tree,
        &|e| matches!(&e.tag, turbo_html2pdf_core::Tag::Html(t) if t == "table"),
    )
    .expect("table");
    assert_eq!(table.style.get("background-color"), Some("#00ff00"));
}

#[test]
fn width_height_attributes_map_to_lengths() {
    let tree = styled(r#"<img width="200" height="50%">"#, "");
    let img = find(
        &tree,
        &|e| matches!(&e.tag, turbo_html2pdf_core::Tag::Html(t) if t == "img"),
    )
    .expect("img");
    assert_eq!(img.style.get("width"), Some("200px"));
    assert_eq!(img.style.get("height"), Some("50%"));
}

// ---------------------------------------------------------------- @media queries

fn styled_at_width(tpl: &str, css: &str, width: f32) -> Vec<StyledNode> {
    use turbo_html2pdf_core::build_cascade_with_width;
    let nodes = render_nodes(tpl);
    let cascade = build_cascade_with_width(css, "", TokenSet::new(), width);
    style_tree(&nodes, &cascade)
}

#[test]
fn media_min_width_applies_above_breakpoint() {
    let css = "div { color: red } @media (min-width: 1000px) { div { color: green } }";
    // Wide viewport: the @media rule wins.
    let wide = styled_at_width("<div>x</div>", css, 1280.0);
    let d = find(
        &wide,
        &|e| matches!(&e.tag, turbo_html2pdf_core::Tag::Html(t) if t == "div"),
    )
    .unwrap();
    assert_eq!(d.style.get("color"), Some("green"));
    // Narrow viewport: the @media block is dropped, base rule stands.
    let narrow = styled_at_width("<div>x</div>", css, 500.0);
    let d = find(
        &narrow,
        &|e| matches!(&e.tag, turbo_html2pdf_core::Tag::Html(t) if t == "div"),
    )
    .unwrap();
    assert_eq!(d.style.get("color"), Some("red"));
}

#[test]
fn media_print_is_dropped_for_screen() {
    let css = "@media print { div { color: red } }";
    let tree = styled_at_width("<div>x</div>", css, 1280.0);
    let d = find(
        &tree,
        &|e| matches!(&e.tag, turbo_html2pdf_core::Tag::Html(t) if t == "div"),
    )
    .unwrap();
    assert_eq!(
        d.style.get("color"),
        None,
        "print-only rules never match screen"
    );
}

#[test]
fn media_em_breakpoint_uses_16px_root() {
    // 66.5em = 1064px; matches at 1280, not at 1000.
    let css = "@media (min-width: 66.5em) { div { color: green } }";
    let at1280 = styled_at_width("<div>x</div>", css, 1280.0);
    let d = find(
        &at1280,
        &|e| matches!(&e.tag, turbo_html2pdf_core::Tag::Html(t) if t == "div"),
    )
    .unwrap();
    assert_eq!(d.style.get("color"), Some("green"));
    let at1000 = styled_at_width("<div>x</div>", css, 1000.0);
    let d = find(
        &at1000,
        &|e| matches!(&e.tag, turbo_html2pdf_core::Tag::Html(t) if t == "div"),
    )
    .unwrap();
    assert_eq!(d.style.get("color"), None);
}

#[test]
fn table_cellpadding_pads_its_cells() {
    // Legacy `cellpadding` on a table applies to its cells (HN's divider bar +
    // list spacing rely on it); `cellpadding="0"` yields 0.
    let padded = styled(
        "<table cellpadding='2'><tr><td id='c'>x</td></tr></table>",
        "",
    );
    assert_eq!(prop(&padded, "c", "padding").as_deref(), Some("2px"));
    let zero = styled(
        "<table cellpadding='0'><tr><td id='c'>x</td></tr></table>",
        "",
    );
    assert_eq!(prop(&zero, "c", "padding").as_deref(), Some("0px"));
}

#[test]
fn next_sibling_combinator() {
    let css = "p + p { color: red }";
    let n = styled("<div><p id='a'>1</p><p id='b'>2</p></div>", css);
    assert_eq!(prop(&n, "a", "color"), None, "first p has no preceding p");
    assert_eq!(prop(&n, "b", "color").as_deref(), Some("red"));
}

#[test]
fn subsequent_sibling_combinator() {
    let css = ".x ~ p { color: red }";
    let n = styled(
        "<div><p id='a'>1</p><span class='x'></span><p id='b'>2</p><p id='c'>3</p></div>",
        css,
    );
    assert_eq!(prop(&n, "a", "color"), None, "p before .x unaffected");
    assert_eq!(prop(&n, "b", "color").as_deref(), Some("red"));
    assert_eq!(prop(&n, "c", "color").as_deref(), Some("red"));
}

#[test]
fn not_pseudo_excludes() {
    let css = "p:not(.skip) { color: red }";
    let n = styled(
        "<div><p id='a'>1</p><p id='b' class='skip'>2</p></div>",
        css,
    );
    assert_eq!(prop(&n, "a", "color").as_deref(), Some("red"));
    assert_eq!(prop(&n, "b", "color"), None, "skipped p not matched");
}

#[test]
fn where_pseudo_matches_only_its_argument() {
    // `:where(...)` / `:is(...)` must honor their argument list. Wikipedia colours
    // broken links with `a:where(.new){color:var(--color-destructive)}`; treating
    // `:where` as always-matching (dropping its arg) reddened EVERY link. A plain
    // `<a>` keeps the base colour; only `a.new` gets the `:where(.new)` rule.
    let css = "a { color: blue } a:where(.new) { color: red } a:is(.stub) { color: green }";
    let n = styled(
        "<div><a id='p'>plain</a><a id='n' class='new'>new</a><a id='s' class='stub'>stub</a></div>",
        css,
    );
    assert_eq!(
        prop(&n, "p", "color").as_deref(),
        Some("blue"),
        "plain link unaffected"
    );
    assert_eq!(
        prop(&n, "n", "color").as_deref(),
        Some("red"),
        ":where(.new) matches a.new"
    );
    assert_eq!(
        prop(&n, "s", "color").as_deref(),
        Some("green"),
        ":is(.stub) matches a.stub"
    );
}

#[test]
fn where_with_nested_not_matches() {
    // The exact Wikipedia shape: `a:where(.new:not([role='button']))`.
    let css = "a { color: blue } a:where(.new:not([role='button'])) { color: red }";
    let n = styled(
        "<div><a id='p'>plain</a><a id='n' class='new'>new</a><a id='b' class='new' role='button'>btn</a></div>",
        css,
    );
    assert_eq!(
        prop(&n, "p", "color").as_deref(),
        Some("blue"),
        "plain link unaffected"
    );
    assert_eq!(
        prop(&n, "n", "color").as_deref(),
        Some("red"),
        "new link matched"
    );
    assert_eq!(
        prop(&n, "b", "color").as_deref(),
        Some("blue"),
        "role=button excluded by nested :not"
    );
}

#[test]
fn em_font_size_does_not_compound_on_inheritance() {
    // `font-size` computes to an absolute px at the element that declares the `em`,
    // so descendants inherit the resolved value — they must NOT re-multiply. A
    // Wikipedia `h1{font-size:1.8em}` whose text re-applied 1.8em rendered at
    // 1.8²·16 = 51.8px instead of 28.8px.
    let css = ".big { font-size: 1.8em }";
    let n = styled(
        "<div class='big' id='h'>Cat<span id='c'>inner</span></div>",
        css,
    );
    let px = |id| {
        prop(&n, id, "font-size")
            .unwrap()
            .strip_suffix("px")
            .unwrap()
            .parse::<f32>()
            .unwrap()
    };
    assert!(
        (px("h") - 28.8).abs() < 0.01,
        "1.8em of 16px root = 28.8, got {}",
        px("h")
    );
    assert!(
        (px("c") - 28.8).abs() < 0.01,
        "child inherits the resolved 28.8px, not 1.8em again (got {})",
        px("c")
    );
}

#[test]
fn superscript_ua_is_smaller_and_raised() {
    // A footnote marker `<sup>[4]</sup>` must render small + raised, not full-size
    // at the baseline. The UA sheet gives `sub,sup` `font-size:0.83em` on top of
    // their `vertical-align`.
    let n = styled("<p>t<sup id='s'>[4]</sup></p>", "");
    // font-size is a computed (absolute) value: 0.83em of the 16px root = 13.28px.
    assert_eq!(prop(&n, "s", "font-size").as_deref(), Some("13.28px"));
    assert_eq!(prop(&n, "s", "vertical-align").as_deref(), Some("super"));
}

#[test]
fn hover_never_matches_in_static_render() {
    let css = "a:hover { color: red }";
    let n = styled("<div><a id='a' href='#'>x</a></div>", css);
    // `a` keeps its UA blue — the hover rule is inert at rest, not applied.
    assert_ne!(prop(&n, "a", "color").as_deref(), Some("red"));
}

#[test]
fn checked_gates_sibling_reveal_like_wikipedia_dropdowns() {
    // Wikipedia's dropdowns: hidden by default, revealed by
    // `input:checked ~ .content`. An unchecked checkbox must leave them hidden.
    let css = ".c { visibility: hidden } input:checked ~ .c { visibility: visible }";
    let closed = styled(
        "<div><input type='checkbox'><span id='s' class='c'>menu</span></div>",
        css,
    );
    assert_eq!(prop(&closed, "s", "visibility").as_deref(), Some("hidden"));
    let open = styled(
        "<div><input type='checkbox' checked><span id='s' class='c'>menu</span></div>",
        css,
    );
    assert_eq!(prop(&open, "s", "visibility").as_deref(), Some("visible"));
}

#[test]
fn nth_child_still_parses_with_paren_arg() {
    // `2n+1` — the `+` inside the pseudo arg must not lex as a combinator.
    let css = "li:nth-child(2n+1) { color: red }";
    let n = styled(
        "<ul><li id='a'>1</li><li id='b'>2</li><li id='c'>3</li></ul>",
        css,
    );
    assert_eq!(prop(&n, "a", "color").as_deref(), Some("red"));
    assert_eq!(prop(&n, "b", "color"), None);
    assert_eq!(prop(&n, "c", "color").as_deref(), Some("red"));
}

#[test]
fn media_query_without_space_after_at_media_applies() {
    // Minified CSS writes `@media(min-width:640px){…}` with no space; the rule
    // inside must still apply at a matching viewport (name parsed as `media`, not
    // `media(min-width:640px)`). Wikipedia floats its infobox this way.
    let css = "@media(min-width:640px){ .x { color: red } }";
    let tree = styled(r#"<p id="e" class="x">y</p>"#, css);
    assert_eq!(prop(&tree, "e", "color").as_deref(), Some("red"));
}

#[test]
fn var_resolves_inherited_custom_property() {
    // Custom properties inherit; `var(--x)` resolves to the inherited value.
    let n = styled(
        r#"<div class="p"><div id="e" class="c">x</div></div>"#,
        ".p{--w:200px} .c{width:var(--w)}",
    );
    assert_eq!(prop(&n, "e", "width").as_deref(), Some("200px"));
}

#[test]
fn var_uses_fallback_when_undefined() {
    let n = styled(
        r#"<div id="e" style="width:var(--missing, 50px)">x</div>"#,
        "",
    );
    assert_eq!(prop(&n, "e", "width").as_deref(), Some("50px"));
}

#[test]
fn var_multiple_in_one_value() {
    let n = styled(
        r#"<div class="p"><div id="e" class="c">x</div></div>"#,
        ".p{--a:10px;--b:20px} .c{padding:var(--a) var(--b)}",
    );
    assert_eq!(prop(&n, "e", "padding").as_deref(), Some("10px 20px"));
}

#[test]
fn var_nested_reference() {
    // A custom property whose value is itself a var().
    let n = styled(
        r#"<div class="p"><div id="e" class="c">x</div></div>"#,
        ".p{--base:15px;--w:var(--base)} .c{width:var(--w)}",
    );
    assert_eq!(prop(&n, "e", "width").as_deref(), Some("15px"));
}

#[test]
fn self_referential_var_uses_fallback() {
    // Codex redefines a design token from its inherited value:
    // `--font-size-medium: var(--font-size-medium, 1rem)`. turbo flattens the
    // cascade so it can't follow the reference up the tree; a self-cycle must
    // resolve to the fallback (not spin to the depth cap and leave an unresolved
    // `var()` that breaks the surrounding `calc()` — which zeroed every icon box).
    let n = styled(
        r#"<div id="e" class="ic">x</div>"#,
        ".ic{--fsm:var(--fsm,1rem);width:calc(var(--fsm,1rem) + 4px)}",
    );
    assert_eq!(prop(&n, "e", "width").as_deref(), Some("calc(1rem + 4px)"));
}

#[test]
fn declaration_semicolon_inside_url_is_not_a_separator() {
    // A `data:image/svg+xml;utf8,<svg…>` mask URL contains a `;` and commas; the
    // declaration splitter must not cut the value there (it zeroed Wikipedia's
    // search icon into a solid box). The whole url() survives, plus the trailing
    // declaration still parses.
    let n = styled(
        r#"<div id="e" class="ic">x</div>"#,
        ".ic{-webkit-mask-image:url('data:image/svg+xml;utf8,<svg width=\"20\">a,b;c</svg>');color:red}",
    );
    let mask = prop(&n, "e", "-webkit-mask-image").unwrap_or_default();
    assert!(
        mask.contains("<svg") && mask.contains("a,b;c"),
        "url() kept whole: {mask}"
    );
    assert_eq!(
        prop(&n, "e", "color").as_deref(),
        Some("red"),
        "trailing decl parses"
    );
}

#[test]
fn hidden_input_is_not_rendered() {
    // `<input type=hidden>` must not paint a default input box (Wikipedia's search
    // form has one, which overlapped the visible field).
    let n = styled(
        r#"<input id="e" type="hidden"><input id="v" type="text">"#,
        "",
    );
    assert_eq!(prop(&n, "e", "display").as_deref(), Some("none"));
    assert_ne!(prop(&n, "v", "display").as_deref(), Some("none"));
}
