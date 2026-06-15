//! Cascade + computed-style resolution (§4.2–4.3). Produces a styled tree from
//! the node tree by matching selectors, applying style tokens and render-time
//! node styles, and folding in inheritance. Cascade order (low→high):
//! UA < author sheets < tokens < node styles < inline < `!important` (AC-4.9).

use std::collections::BTreeMap;

use super::parser::{parse_declarations, Declaration, Rule};
use super::selector::{AttrOp, AttrSel, Combinator, Compound, Pseudo, Selector, Specificity};
use super::token::{resolve_tokens, TokenSet};
use crate::node::{Attr, Element, Node, Tag};

/// Computed property values for one element.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComputedStyle {
    map: BTreeMap<String, String>,
}

impl ComputedStyle {
    /// The computed value of a property, if set.
    pub fn get(&self, property: &str) -> Option<&str> {
        self.map.get(property).map(String::as_str)
    }

    /// Whether no value uses a containing-block-relative (`%`) or font-relative
    /// (`em`/`rem`) unit. When true, the box's resolved metrics do not depend on
    /// the layout context, so they can be resolved once and cached (a `font-size`
    /// is still required to be absolute — see `is_ctx_independent`). Conservative:
    /// any `%`/`em` substring (even inside a value like `lemonchiffon`) just
    /// disables the cache, which is always safe.
    pub fn has_no_relative_units(&self) -> bool {
        self.map
            .values()
            .all(|v| !v.contains('%') && !v.contains("em"))
    }

    /// Build a computed style directly from property/value pairs. Useful for
    /// programmatic node styles and for layout-level testing.
    pub fn from_pairs<I, K, V>(pairs: I) -> ComputedStyle
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let map = pairs
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        ComputedStyle { map }
    }
}

/// A styled element node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledElement {
    pub tag: Tag,
    pub attrs: Vec<Attr>,
    pub style: ComputedStyle,
    pub children: Vec<StyledNode>,
}

/// A node in the styled tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyledNode {
    Element(StyledElement),
    Text(String),
}

impl StyledNode {
    /// Borrow the styled element if this node is one.
    pub fn as_element(&self) -> Option<&StyledElement> {
        match self {
            StyledNode::Element(e) => Some(e),
            StyledNode::Text(_) => None,
        }
    }
}

/// A rule tagged with its cascade level and source order.
#[derive(Debug, Clone)]
pub struct LeveledRule {
    pub level: u8,
    pub order: usize,
    pub rule: Rule,
}

/// Resolved cascade input shared across a render.
#[derive(Debug, Clone, Default)]
pub struct Cascade {
    pub rules: Vec<LeveledRule>,
    pub tokens: TokenSet,
}

/// Properties that inherit by default (§4.2).
const INHERITED: &[&str] = &[
    "color",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "line-height",
    "text-align",
    "text-indent",
    "letter-spacing",
    "word-spacing",
    "white-space",
    "text-transform",
    "hyphens",
    "list-style-type",
    "list-style-position",
    "visibility",
];

// --------------------------------------------------------------------------
// element position + match context
// --------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct ElemPos {
    index: usize,
    of_type_index: usize,
    siblings: usize,
}

#[derive(Clone)]
struct Ctx<'a> {
    tag: Option<&'a str>,
    id: Option<&'a str>,
    classes: Vec<&'a str>,
    attrs: &'a [Attr],
    pos: ElemPos,
}

fn html_name(element: &Element) -> Option<&str> {
    match &element.tag {
        Tag::Html(name) => Some(name),
        Tag::Directive(_) => None,
    }
}

fn tag_key(element: &Element) -> String {
    match &element.tag {
        Tag::Html(name) => name.clone(),
        Tag::Directive(kind) => format!("{kind:?}"),
    }
}

fn ctx_of<'a>(element: &'a Element, pos: ElemPos) -> Ctx<'a> {
    Ctx {
        tag: html_name(element),
        id: element.attr("id"),
        classes: element
            .attr("class")
            .map(str::split_whitespace)
            .map(Iterator::collect)
            .unwrap_or_default(),
        attrs: &element.attrs,
        pos,
    }
}

fn element_positions(nodes: &[Node]) -> Vec<Option<ElemPos>> {
    let elems: Vec<(usize, String)> = nodes
        .iter()
        .enumerate()
        .filter_map(|(i, n)| n.as_element().map(|e| (i, tag_key(e))))
        .collect();
    let siblings = elems.len();
    let mut positions = vec![None; nodes.len()];
    for (order, (i, key)) in elems.iter().enumerate() {
        positions[*i] = Some(position_for(order, key, &elems, siblings));
    }
    positions
}

fn position_for(order: usize, key: &str, elems: &[(usize, String)], siblings: usize) -> ElemPos {
    let of_type_index = elems[..=order].iter().filter(|(_, k)| k == key).count();
    ElemPos {
        index: order + 1,
        of_type_index,
        siblings,
    }
}

// --------------------------------------------------------------------------
// selector matching
// --------------------------------------------------------------------------

fn find_attr<'a>(attrs: &'a [Attr], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|a| a.name == name)
        .map(|a| a.value.as_str())
}

fn dash_match(actual: &str, expected: &str) -> bool {
    actual == expected || actual.starts_with(&format!("{expected}-"))
}

fn op_matches(op: &AttrOp, actual: &str, expected: &str) -> bool {
    match op {
        AttrOp::Exists => true,
        AttrOp::Equals => actual == expected,
        AttrOp::Includes => actual.split_whitespace().any(|w| w == expected),
        AttrOp::DashMatch => dash_match(actual, expected),
        AttrOp::Prefix => actual.starts_with(expected),
        AttrOp::Suffix => actual.ends_with(expected),
        AttrOp::Substring => actual.contains(expected),
    }
}

fn attr_matches(sel: &AttrSel, attrs: &[Attr]) -> bool {
    match find_attr(attrs, &sel.name) {
        Some(actual) => op_matches(&sel.op, actual, &sel.value),
        None => false,
    }
}

fn nth_match(a: i32, b: i32, index: usize) -> bool {
    let idx = index as i32;
    if a == 0 {
        return idx == b;
    }
    let n = idx - b;
    n % a == 0 && n / a >= 0
}

fn pseudo_matches(pseudo: &Pseudo, pos: ElemPos) -> bool {
    match pseudo {
        Pseudo::FirstChild => pos.index == 1,
        Pseudo::LastChild => pos.index == pos.siblings,
        Pseudo::NthChild(a, b) => nth_match(*a, *b, pos.index),
        Pseudo::NthOfType(a, b) => nth_match(*a, *b, pos.of_type_index),
    }
}

fn tag_ok(compound: &Compound, ctx: &Ctx) -> bool {
    match &compound.tag {
        Some(t) => ctx.tag == Some(t.as_str()),
        None => true,
    }
}

fn id_ok(compound: &Compound, ctx: &Ctx) -> bool {
    match &compound.id {
        Some(i) => ctx.id == Some(i.as_str()),
        None => true,
    }
}

fn classes_ok(compound: &Compound, ctx: &Ctx) -> bool {
    compound
        .classes
        .iter()
        .all(|c| ctx.classes.contains(&c.as_str()))
}

fn compound_matches(compound: &Compound, ctx: &Ctx) -> bool {
    tag_ok(compound, ctx)
        && id_ok(compound, ctx)
        && classes_ok(compound, ctx)
        && compound.attrs.iter().all(|a| attr_matches(a, ctx.attrs))
        && compound.pseudos.iter().all(|p| pseudo_matches(p, ctx.pos))
}

fn match_child(sel: &Selector, ci: usize, path: &[Ctx], pi: usize) -> bool {
    pi > 0
        && compound_matches(&sel.compounds[ci - 1], &path[pi - 1])
        && match_ancestors(sel, ci - 1, path, pi - 1)
}

fn match_descendant(sel: &Selector, ci: usize, path: &[Ctx], pi: usize) -> bool {
    (0..pi).rev().any(|parent| {
        compound_matches(&sel.compounds[ci - 1], &path[parent])
            && match_ancestors(sel, ci - 1, path, parent)
    })
}

fn match_ancestors(sel: &Selector, ci: usize, path: &[Ctx], pi: usize) -> bool {
    if ci == 0 {
        return true;
    }
    match sel.combinators[ci - 1] {
        Combinator::Child => match_child(sel, ci, path, pi),
        Combinator::Descendant => match_descendant(sel, ci, path, pi),
    }
}

fn matches(sel: &Selector, path: &[Ctx]) -> bool {
    let last = path.len() - 1;
    let subject = &sel.compounds[sel.compounds.len() - 1];
    compound_matches(subject, &path[last])
        && match_ancestors(sel, sel.compounds.len() - 1, path, last)
}

// --------------------------------------------------------------------------
// candidate collection + winner selection
// --------------------------------------------------------------------------

type Key = (bool, u8, Specificity, usize);

struct Cand {
    key: Key,
    property: String,
    value: String,
}

fn push_decls(
    decls: &[Declaration],
    level: u8,
    spec: Specificity,
    base: usize,
    out: &mut Vec<Cand>,
) {
    for (i, d) in decls.iter().enumerate() {
        let key = (d.important, level, spec, base + i);
        out.push(Cand {
            key,
            property: d.property.clone(),
            value: d.value.clone(),
        });
    }
}

fn best_match(selectors: &[Selector], path: &[Ctx]) -> Option<Specificity> {
    selectors
        .iter()
        .filter(|s| matches(s, path))
        .map(|s| s.specificity)
        .max()
}

fn collect_rules(cascade: &Cascade, path: &[Ctx], out: &mut Vec<Cand>) {
    for lr in &cascade.rules {
        if let Some(spec) = best_match(&lr.rule.selectors, path) {
            push_decls(&lr.rule.declarations, lr.level, spec, lr.order, out);
        }
    }
}

fn collect_tokens(element: &Element, cascade: &Cascade, out: &mut Vec<Cand>) {
    if let Some(t_style) = element.attr("t:style") {
        let decls = resolve_tokens(t_style, &cascade.tokens);
        push_decls(&decls, 2, (0, 0, 0), 0, out);
    }
}

fn collect_inline(element: &Element, out: &mut Vec<Cand>) {
    if let Some(style) = element.attr("style") {
        let decls = parse_declarations(style);
        push_decls(&decls, 4, (0, 0, 0), 0, out);
    }
}

fn pick_winners(cands: Vec<Cand>) -> BTreeMap<String, String> {
    let mut best: BTreeMap<String, (Key, String)> = BTreeMap::new();
    for cand in cands {
        let beats = best.get(&cand.property).is_none_or(|(k, _)| cand.key >= *k);
        if beats {
            best.insert(cand.property, (cand.key, cand.value));
        }
    }
    best.into_iter().map(|(k, (_, v))| (k, v)).collect()
}

fn inherit(own: BTreeMap<String, String>, parent: &ComputedStyle) -> ComputedStyle {
    let mut map = BTreeMap::new();
    for prop in INHERITED {
        if let Some(v) = parent.get(prop) {
            map.insert((*prop).to_string(), v.to_string());
        }
    }
    map.extend(own);
    ComputedStyle { map }
}

fn resolve_style(
    element: &Element,
    path: &[Ctx],
    parent: &ComputedStyle,
    cascade: &Cascade,
) -> ComputedStyle {
    let mut cands = Vec::new();
    collect_rules(cascade, path, &mut cands);
    collect_tokens(element, cascade, &mut cands);
    collect_inline(element, &mut cands);
    inherit(pick_winners(cands), parent)
}

// --------------------------------------------------------------------------
// tree construction
// --------------------------------------------------------------------------

fn style_element<'a>(
    element: &'a Element,
    pos: ElemPos,
    ancestors: &[Ctx<'a>],
    parent: &ComputedStyle,
    cascade: &Cascade,
) -> StyledNode {
    let mut path = ancestors.to_vec();
    path.push(ctx_of(element, pos));
    let style = resolve_style(element, &path, parent, cascade);
    let children = style_siblings(&element.children, &path, &style, cascade);
    StyledNode::Element(StyledElement {
        tag: element.tag.clone(),
        attrs: element.attrs.clone(),
        style,
        children,
    })
}

fn style_one<'a>(
    node: &'a Node,
    pos: Option<ElemPos>,
    ancestors: &[Ctx<'a>],
    parent: &ComputedStyle,
    cascade: &Cascade,
) -> StyledNode {
    match node {
        Node::Text(t) => StyledNode::Text(t.clone()),
        Node::Element(e) => style_element(
            e,
            pos.expect("element has a position"),
            ancestors,
            parent,
            cascade,
        ),
    }
}

fn style_siblings<'a>(
    nodes: &'a [Node],
    ancestors: &[Ctx<'a>],
    parent: &ComputedStyle,
    cascade: &Cascade,
) -> Vec<StyledNode> {
    let positions = element_positions(nodes);
    nodes
        .iter()
        .enumerate()
        .map(|(i, n)| style_one(n, positions[i], ancestors, parent, cascade))
        .collect()
}

/// Build the styled tree for a flow of nodes under the given cascade.
pub fn style_tree(nodes: &[Node], cascade: &Cascade) -> Vec<StyledNode> {
    style_siblings(nodes, &[], &ComputedStyle::default(), cascade)
}
