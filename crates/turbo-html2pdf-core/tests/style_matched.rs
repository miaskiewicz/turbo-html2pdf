use turbo_html2pdf_core::style::{
    parse_declarations, presentational_declarations, resolve_matched_style, user_agent_css,
    user_agent_stylesheet, CascadeOrigin, ComputedStyle, MatchedDeclaration,
};
use turbo_html2pdf_core::{Attr, Element, Tag};

fn matched(css: &str, origin: CascadeOrigin, depth: u32) -> MatchedDeclaration {
    MatchedDeclaration {
        declaration: parse_declarations(css).remove(0),
        origin,
        scope_depth: depth,
        inline: false,
        specificity: (0, 0, 1),
        order: 0,
    }
}

fn width(declarations: &[MatchedDeclaration]) -> String {
    resolve_matched_style(&ComputedStyle::default(), declarations)
        .get("width")
        .unwrap()
        .to_owned()
}

#[test]
fn normal_encapsulation_beats_specificity_inline_and_source_order() {
    let outer = matched("width:40px", CascadeOrigin::Author, 0);
    let mut inner = matched("width:80px", CascadeOrigin::Author, 1);
    inner.inline = true;
    inner.specificity = (100, 100, 100);
    inner.order = usize::MAX;
    assert_eq!(width(&[outer.clone(), inner.clone()]), "40px");
    assert_eq!(width(&[inner, outer]), "40px");
}

#[test]
fn important_encapsulation_reverses_with_nested_shadow_contexts() {
    let mut outer = matched("width:40px !important", CascadeOrigin::Author, 0);
    outer.inline = true;
    let inner = matched("width:80px !important", CascadeOrigin::Author, 1);
    let nested = matched("width:120px !important", CascadeOrigin::Author, 2);
    assert_eq!(width(&[nested.clone(), outer, inner]), "120px");
    assert_eq!(
        width(&[nested, matched("width:900px", CascadeOrigin::Author, 3)]),
        "120px"
    );
}

#[test]
fn origin_precedes_scope_and_reverses_for_important_rules() {
    let ua = matched("width:10px", CascadeOrigin::UserAgent, 0);
    let user = matched("width:20px", CascadeOrigin::User, 0);
    let author = matched("width:30px", CascadeOrigin::Author, 5);
    assert_eq!(width(&[author, user, ua]), "30px");
    let ua = matched("width:10px !important", CascadeOrigin::UserAgent, 0);
    let user = matched("width:20px !important", CascadeOrigin::User, 4);
    let author = matched("width:30px !important", CascadeOrigin::Author, 5);
    assert_eq!(width(&[author, user, ua]), "10px");
}

#[test]
fn same_scope_inline_specificity_and_order_remain_independent() {
    let mut first = matched("width:10px", CascadeOrigin::Author, 1);
    first.specificity = (1, 0, 0);
    let mut later = matched("width:20px", CascadeOrigin::Author, 1);
    later.order = 100;
    assert_eq!(width(&[first.clone(), later.clone()]), "10px");
    later.specificity = first.specificity;
    assert_eq!(width(&[later.clone(), first.clone()]), "20px");
    first.inline = true;
    assert_eq!(width(&[first, later]), "10px");
}

#[test]
fn inherited_variables_shorthand_and_font_size_use_existing_resolution() {
    let parent = ComputedStyle::from_pairs([
        ("--spacing", "2px 4px"),
        ("font-size", "20px"),
        ("color", "blue"),
    ]);
    let declarations = [
        matched("padding-left:99px", CascadeOrigin::Author, 1),
        matched("padding:var(--spacing)", CascadeOrigin::Author, 0),
        matched("font-size:150%", CascadeOrigin::Author, 1),
    ];
    let result = resolve_matched_style(&parent, &declarations);
    assert_eq!(result.get("padding-left"), Some("4px"));
    assert_eq!(result.get("padding-top"), Some("2px"));
    assert_eq!(result.get("font-size"), Some("30px"));
    assert_eq!(result.get("color"), Some("blue"));
}

#[test]
fn attribute_hints_are_explicit_candidates_overridden_by_author_rules() {
    let element = Element {
        tag: Tag::Html("img".into()),
        attrs: vec![Attr {
            name: "width".into(),
            value: "80".into(),
        }],
        children: Vec::new(),
    };
    let hints = presentational_declarations(&element);
    let mut declarations: Vec<_> = hints
        .into_iter()
        .map(|declaration| MatchedDeclaration {
            declaration,
            origin: CascadeOrigin::Author,
            scope_depth: 0,
            inline: false,
            specificity: (0, 0, 0),
            order: 0,
        })
        .collect();
    assert_eq!(width(&declarations), "80px");
    declarations.push(matched("width:40px", CascadeOrigin::Author, 0));
    assert_eq!(width(&declarations), "40px");
}

#[test]
fn ua_defaults_are_available_without_inherited_style_becoming_a_rule() {
    let sheet = user_agent_stylesheet();
    assert_eq!(
        turbo_html2pdf_core::style::parse_stylesheet(user_agent_css()),
        sheet
    );
    assert!(sheet
        .rules
        .iter()
        .flat_map(|rule| &rule.declarations)
        .any(|declaration| declaration.property == "font-family"
            && declaration.value == "sans-serif"));
    assert!(sheet
        .rules
        .iter()
        .flat_map(|rule| &rule.declarations)
        .any(|declaration| declaration.property == "display" && declaration.value == "inline"));
    let mut declarations = sheet
        .rules
        .iter()
        .flat_map(|rule| &rule.declarations)
        .filter(|declaration| declaration.property == "font-family")
        .map(|declaration| MatchedDeclaration {
            declaration: declaration.clone(),
            origin: CascadeOrigin::UserAgent,
            scope_depth: 0,
            inline: false,
            specificity: (0, 0, 1),
            order: 0,
        })
        .collect::<Vec<_>>();
    declarations.push(matched("font-family:serif", CascadeOrigin::Author, 1));
    let parent = ComputedStyle::from_pairs([("color", "green")]);
    let resolved = resolve_matched_style(&parent, &declarations);
    assert_eq!(resolved.get("font-family"), Some("serif"));
    assert_eq!(resolved.get("color"), Some("green"));
}
