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
    of_type_total: usize,
    siblings: usize,
}

#[derive(Clone)]
struct Ctx<'a> {
    tag: Option<&'a str>,
    id: Option<&'a str>,
    classes: Vec<&'a str>,
    attrs: &'a [Attr],
    pos: ElemPos,
    /// Whether the element has no element/text children (`:empty`).
    empty: bool,
    /// Preceding element siblings in document order (for the `+`/`~` combinators).
    /// Each carries an empty `prev`; the matcher re-slices this list when it steps
    /// left across a sibling combinator.
    prev: Vec<Ctx<'a>>,
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

fn ctx_of<'a>(element: &'a Element, pos: ElemPos, prev: Vec<Ctx<'a>>) -> Ctx<'a> {
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
        empty: element.children.is_empty(),
        prev,
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
    let of_type_total = elems.iter().filter(|(_, k)| k == key).count();
    ElemPos {
        index: order + 1,
        of_type_index,
        of_type_total,
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

fn pseudo_matches(pseudo: &Pseudo, ctx: &Ctx) -> bool {
    let pos = ctx.pos;
    match pseudo {
        Pseudo::FirstChild => pos.index == 1,
        Pseudo::LastChild => pos.index == pos.siblings,
        Pseudo::OnlyChild => pos.siblings == 1,
        Pseudo::FirstOfType => pos.of_type_index == 1,
        Pseudo::LastOfType => pos.of_type_index == pos.of_type_total,
        Pseudo::OnlyOfType => pos.of_type_total == 1,
        Pseudo::NthChild(a, b) => nth_match(*a, *b, pos.index),
        Pseudo::NthOfType(a, b) => nth_match(*a, *b, pos.of_type_index),
        Pseudo::Root => ctx.tag == Some("html"),
        Pseudo::Empty => ctx.empty,
        Pseudo::Checked => has_attr(ctx.attrs, "checked"),
        Pseudo::Disabled => has_attr(ctx.attrs, "disabled"),
        Pseudo::Enabled => is_form_control(ctx.tag) && !has_attr(ctx.attrs, "disabled"),
        Pseudo::Link => {
            matches!(ctx.tag, Some("a" | "area" | "link")) && has_attr(ctx.attrs, "href")
        }
        Pseudo::Not(compounds) => !compounds.iter().any(|c| compound_matches(c, ctx)),
        Pseudo::Is(compounds) => compounds.iter().any(|c| compound_matches(c, ctx)),
        Pseudo::NeverMatch => false,
    }
}

fn has_attr(attrs: &[Attr], name: &str) -> bool {
    attrs.iter().any(|a| a.name.eq_ignore_ascii_case(name))
}

fn is_form_control(tag: Option<&str>) -> bool {
    matches!(
        tag,
        Some("input" | "button" | "select" | "textarea" | "option")
    )
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
        && compound.pseudos.iter().all(|p| pseudo_matches(p, ctx))
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

/// Match compound `ci-1` against a preceding sibling of the element at `path[pi]`,
/// then continue the selector from that sibling. `immediate` restricts to the
/// directly-preceding sibling (`+`); otherwise any earlier sibling (`~`).
fn match_sibling(sel: &Selector, ci: usize, path: &[Ctx], pi: usize, immediate: bool) -> bool {
    let sibs = &path[pi].prev;
    let range = if immediate {
        sibs.len().saturating_sub(1)..sibs.len()
    } else {
        0..sibs.len()
    };
    range.rev().any(|k| {
        if !compound_matches(&sel.compounds[ci - 1], &sibs[k]) {
            return false;
        }
        // Continue the match from the sibling at the same depth: reuse the shared
        // ancestors, and give the sibling its own preceding siblings so chained
        // sibling combinators (`a ~ b + c`) keep working.
        let mut sp = path[..pi].to_vec();
        let mut sib = sibs[k].clone();
        sib.prev = sibs[..k].to_vec();
        sp.push(sib);
        match_ancestors(sel, ci - 1, &sp, pi)
    })
}

fn match_ancestors(sel: &Selector, ci: usize, path: &[Ctx], pi: usize) -> bool {
    if ci == 0 {
        return true;
    }
    match sel.combinators[ci - 1] {
        Combinator::Child => match_child(sel, ci, path, pi),
        Combinator::Descendant => match_descendant(sel, ci, path, pi),
        Combinator::NextSibling => match_sibling(sel, ci, path, pi, true),
        Combinator::SubsequentSibling => match_sibling(sel, ci, path, pi, false),
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

/// Legacy presentational attributes mapped to CSS declarations (`bgcolor`,
/// `width`/`height`, `<font color>`). These are "presentational hints": they sit
/// just above the UA sheet and below any real author rule (level 1, specificity
/// 0), so an author stylesheet always overrides them. Old table-layout sites
/// (Hacker News' orange header `<td bgcolor>`, sized `<img>`) depend on them.
fn collect_presentational(element: &Element, out: &mut Vec<Cand>) {
    // Order matters (bgcolor, width, height, then `<font color>`); each entry is
    // present only when its attribute is set and usable.
    let decls: Vec<Declaration> = [
        bgcolor_decl(element),
        element.attr("width").and_then(attr_length).map(width_decl),
        element
            .attr("height")
            .and_then(attr_length)
            .map(height_decl),
        font_color_decl(element),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !decls.is_empty() {
        push_decls(&decls, 1, (0, 0, 0), 0, out);
    }
}

/// A presentational-hint declaration (level 1, never `important`).
fn pres_decl(property: &str, value: String) -> Declaration {
    Declaration {
        property: property.to_string(),
        value,
        important: false,
    }
}

fn width_decl(value: String) -> Declaration {
    pres_decl("width", value)
}

fn height_decl(value: String) -> Declaration {
    pres_decl("height", value)
}

/// `bgcolor="..."` → `background-color`, when non-empty.
fn bgcolor_decl(element: &Element) -> Option<Declaration> {
    let bg = element
        .attr("bgcolor")
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    Some(pres_decl("background-color", bg.to_string()))
}

/// `<font color="...">` → `color`, when non-empty (only on the `<font>` tag).
fn font_color_decl(element: &Element) -> Option<Declaration> {
    if !is_tag(element, "font") {
        return None;
    }
    let c = element
        .attr("color")
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    Some(pres_decl("color", c.to_string()))
}

/// A presentational length attribute → a CSS length value: a bare number is `px`,
/// a `N%` passes through; anything else (e.g. `width="*"`) is ignored.
fn attr_length(v: &str) -> Option<String> {
    let v = v.trim();
    if let Some(pct) = v.strip_suffix('%') {
        return pct.trim().parse::<f32>().ok().map(|_| v.to_string());
    }
    v.parse::<f32>().ok().map(|n| format!("{n}px"))
}

fn is_tag(element: &Element, name: &str) -> bool {
    matches!(&element.tag, Tag::Html(t) if t.eq_ignore_ascii_case(name))
}

/// The `cellpadding` (px) of the nearest ancestor `<table>` — a legacy table
/// attribute that pads every cell (e.g. HN's `cellpadding="1"` divider bar, an
/// empty coloured `<td>` that must reserve its padding to show as a line).
fn ancestor_cellpadding(path: &[Ctx]) -> Option<f32> {
    let table = path.iter().rev().skip(1).find(|c| c.tag == Some("table"))?;
    let attr = table
        .attrs
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case("cellpadding"))?;
    attr.value.trim().parse::<f32>().ok()
}

/// Apply a table's `cellpadding` to its cells as a presentational padding hint.
fn collect_cellpadding(element: &Element, path: &[Ctx], out: &mut Vec<Cand>) {
    if !(is_tag(element, "td") || is_tag(element, "th")) {
        return;
    }
    if let Some(pad) = ancestor_cellpadding(path) {
        let decl = Declaration {
            property: "padding".to_string(),
            value: format!("{pad}px"),
            important: false,
        };
        push_decls(&[decl], 1, (0, 0, 0), 0, out);
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

/// The document root font size (px). CSS `rem` and the initial `font-size` both
/// resolve against it; turbo does not honor an `html { font-size }` override.
const ROOT_FONT_PX: f32 = 16.0;

/// Resolve `font-size` to an absolute px value against the parent's (already
/// absolute) font size, then store it back as `"<n>px"`. Font-relative units
/// resolve at the element that *declares* them (§4.2): if we left `em`/`%` as a
/// string, every descendant that inherits it would re-multiply against the
/// parent's px — e.g. `h1{font-size:1.8em}` compounding to 1.8²·16 on its text.
fn resolve_font_size(map: &mut BTreeMap<String, String>, parent: &ComputedStyle) {
    let parent_px = parent
        .get("font-size")
        .and_then(parse_abs_px)
        .unwrap_or(ROOT_FONT_PX);
    let Some(raw) = map.get("font-size") else {
        return;
    };
    let px = font_size_px(raw, parent_px);
    map.insert("font-size".to_string(), format!("{px}px"));
}

/// Parse an already-absolute px string (`"28.8px"` / `"16"`); `None` for anything
/// still carrying a relative unit.
fn parse_abs_px(v: &str) -> Option<f32> {
    let t = v.trim();
    t.strip_suffix("px").unwrap_or(t).trim().parse::<f32>().ok()
}

/// Resolve one `font-size` value to px against `parent_px`: `px`/`pt`/`pc`/`in`/
/// `cm`/`mm` absolute, `em`/`%` parent-relative, `rem` root-relative, and the
/// `larger`/`smaller`/`xx-small…xx-large` keywords (coarse steps).
fn font_size_px(value: &str, parent_px: f32) -> f32 {
    let t = value.trim();
    let kw = match t {
        "larger" => Some(parent_px * 1.2),
        "smaller" => Some(parent_px / 1.2),
        "xx-small" => Some(ROOT_FONT_PX * 0.6),
        "x-small" => Some(ROOT_FONT_PX * 0.75),
        "small" => Some(ROOT_FONT_PX * 0.89),
        "medium" => Some(ROOT_FONT_PX),
        "large" => Some(ROOT_FONT_PX * 1.2),
        "x-large" => Some(ROOT_FONT_PX * 1.5),
        "xx-large" => Some(ROOT_FONT_PX * 2.0),
        _ => None,
    };
    if let Some(px) = kw {
        return px;
    }
    let split = t
        .char_indices()
        .find(|(_, c)| c.is_ascii_alphabetic() || *c == '%')
        .map(|(i, _)| i)
        .unwrap_or(t.len());
    let Ok(n) = t[..split].parse::<f32>() else {
        return parent_px;
    };
    match t[split..].trim() {
        "" | "px" => n,
        "pt" => n * 96.0 / 72.0,
        "pc" => n * 16.0,
        "in" => n * 96.0,
        "cm" => n * 96.0 / 2.54,
        "mm" => n * 96.0 / 25.4,
        "em" => n * parent_px,
        "rem" => n * ROOT_FONT_PX,
        "%" => n / 100.0 * parent_px,
        _ => parent_px,
    }
}

fn inherit(own: BTreeMap<String, String>, parent: &ComputedStyle) -> ComputedStyle {
    let mut map = BTreeMap::new();
    for prop in INHERITED {
        if let Some(v) = parent.get(prop) {
            map.insert((*prop).to_string(), v.to_string());
        }
    }
    // Custom properties (`--*`) inherit by default (they're how `var()` cascades).
    for (k, v) in &parent.map {
        if k.starts_with("--") {
            map.insert(k.clone(), v.clone());
        }
    }
    map.extend(own);
    resolve_var_refs(&mut map);
    resolve_font_size(&mut map, parent);
    ComputedStyle { map }
}

/// Substitute every `var(--name, fallback)` in the map's values with the resolved
/// custom-property value (or the fallback). Custom properties are read from the
/// same map (already inherited), so `var()` sees the cascaded value. Runs after
/// inheritance so a child's `var()` picks up an ancestor's custom property.
fn resolve_var_refs(map: &mut BTreeMap<String, String>) {
    let vars: BTreeMap<String, String> = map
        .iter()
        .filter(|(k, _)| k.starts_with("--"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (key, value) in map.iter_mut() {
        if value.contains("var(") {
            *value = substitute_vars(value, &vars, 0, &[]);
            let _ = key;
        }
    }
}

/// Replace `var(--name[, fallback])` references in `value` using `vars`. Balanced
/// parens (a fallback may itself contain `var()` / functions); a missing or empty
/// custom property falls back. Depth-limited against a cyclic `--a: var(--a)`.
fn substitute_vars(
    value: &str,
    vars: &BTreeMap<String, String>,
    depth: u8,
    chain: &[&str],
) -> String {
    if depth > 16 || !value.contains("var(") {
        return value.to_string();
    }
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    while i < bytes.len() {
        if value[i..].starts_with("var(") {
            let inner_start = i + 4;
            let Some(close) = matching_paren(value, inner_start) else {
                out.push_str(&value[i..]);
                break;
            };
            let inner = &value[inner_start..close];
            out.push_str(&resolve_one_var(inner, vars, depth, chain));
            i = close + 1;
        } else {
            let ch = value[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// The index of the `)` matching the `(` that opened just before `open` (i.e. the
/// close of the `var(` at `open-4`), accounting for nested parens.
fn matching_paren(s: &str, open: usize) -> Option<usize> {
    let mut depth = 1i32;
    for (rel, ch) in s[open..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + rel);
                }
            }
            _ => {}
        }
    }
    None
}

/// Resolve one `var()` body (`--name` or `--name, fallback`) to its value. `chain`
/// is the stack of custom properties currently being resolved: a name that appears
/// again is a reference cycle (`--a: var(--a, 1rem)` — Codex's "redefine a token
/// from its inherited value" idiom, which we flatten and so cannot follow up the
/// tree), so it resolves to the fallback instead of spinning to the depth cap and
/// leaving an unresolved `var()` that breaks the surrounding `calc()`.
fn resolve_one_var(
    inner: &str,
    vars: &BTreeMap<String, String>,
    depth: u8,
    chain: &[&str],
) -> String {
    let (name, fallback) = match inner.find(',') {
        Some(c) => (inner[..c].trim(), inner[c + 1..].trim()),
        None => (inner.trim(), ""),
    };
    let cyclic = chain.contains(&name);
    match vars.get(name).map(|v| v.trim()).filter(|v| !v.is_empty()) {
        Some(v) if !cyclic => {
            let mut next: Vec<&str> = chain.to_vec();
            next.push(name);
            substitute_vars(v, vars, depth + 1, &next)
        }
        _ => substitute_vars(fallback, vars, depth + 1, chain),
    }
}

fn resolve_style(
    element: &Element,
    path: &[Ctx],
    parent: &ComputedStyle,
    cascade: &Cascade,
) -> ComputedStyle {
    let mut cands = Vec::new();
    collect_presentational(element, &mut cands);
    collect_cellpadding(element, path, &mut cands);
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
    prev: Vec<Ctx<'a>>,
    parent: &ComputedStyle,
    cascade: &Cascade,
) -> StyledNode {
    let mut path = ancestors.to_vec();
    path.push(ctx_of(element, pos, prev));
    let style = resolve_style(element, &path, parent, cascade);
    let children = style_siblings(&element.children, &path, &style, cascade);
    StyledNode::Element(StyledElement {
        tag: element.tag.clone(),
        attrs: element.attrs.clone(),
        style,
        children,
    })
}

fn style_siblings<'a>(
    nodes: &'a [Node],
    ancestors: &[Ctx<'a>],
    parent: &ComputedStyle,
    cascade: &Cascade,
) -> Vec<StyledNode> {
    let positions = element_positions(nodes);
    // Accumulate each preceding element sibling's (prev-less) context so a later
    // sibling can be matched against it by the `+`/`~` combinators.
    let mut prev: Vec<Ctx<'a>> = Vec::new();
    let mut out = Vec::with_capacity(nodes.len());
    for (i, node) in nodes.iter().enumerate() {
        match node {
            Node::Text(t) => out.push(StyledNode::Text(t.clone())),
            Node::Element(e) => {
                let pos = positions[i].expect("element has a position");
                out.push(style_element(
                    e,
                    pos,
                    ancestors,
                    prev.clone(),
                    parent,
                    cascade,
                ));
                prev.push(ctx_of(e, pos, Vec::new()));
            }
        }
    }
    out
}

/// Build the styled tree for a flow of nodes under the given cascade.
pub fn style_tree(nodes: &[Node], cascade: &Cascade) -> Vec<StyledNode> {
    style_siblings(nodes, &[], &ComputedStyle::default(), cascade)
}

/// [`style_tree`] with `<html>`/`<body>` ancestor shells seeded as match-only
/// contexts, so descendant selectors gated on their classes resolve. The shells
/// are not laid out; they only participate in selector matching.
pub fn style_tree_with_roots(
    nodes: &[Node],
    cascade: &Cascade,
    roots: &[Element],
) -> Vec<StyledNode> {
    let ancestors: Vec<Ctx> = roots
        .iter()
        .map(|e| ctx_of(e, root_pos(), Vec::new()))
        .collect();
    style_siblings(nodes, &ancestors, &ComputedStyle::default(), cascade)
}

/// The position for a match-only root shell (a lone only-child); the structural
/// pseudos on `<html>`/`<body>` don't affect real page layout.
fn root_pos() -> ElemPos {
    ElemPos {
        index: 0,
        of_type_index: 0,
        of_type_total: 1,
        siblings: 1,
    }
}
