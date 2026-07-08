//! The style system (§4): CSS parsing, the selector engine, named style tokens,
//! render-time node styles, and cascade + inheritance into a styled tree.

mod cascade;
mod parser;
mod selector;
mod token;

pub use cascade::{
    style_tree, style_tree_with_roots, Cascade, ComputedStyle, LeveledRule, StyledElement,
    StyledNode,
};
pub use parser::{parse_stylesheet, AtRule, Declaration, Rule, Stylesheet};
pub use token::{StyleToken, TokenSet};

/// A minimal user-agent stylesheet (§4.2): the defaults the cascade starts from.
/// Block elements need no rule (display resolves to `block` by default); this
/// declares the inline, table, and list-item defaults plus a few typographic
/// ones so real HTML lays out without author CSS.
///
/// The `font-family` defaults pin the document to the CSS generic families so a
/// doc renders with no author CSS: `body` resolves to `sans-serif` (inherited by
/// everything) and code-ish elements to `monospace`. With the default
/// `bundled-fonts` feature the [`FontRegistry`](crate::FontRegistry) expands
/// `sans-serif` → Inter → Roboto, `serif` → Liberation Serif → PT Serif, and
/// `monospace` → Fira Code → IBM Plex Mono, so these resolve to embedded faces
/// with no caller fonts.
const UA_CSS: &str = "
body { font-family: sans-serif }
code, kbd, samp, pre { font-family: monospace }
b { font-weight: bold }
strong { font-weight: bold }
i { font-style: italic }
em { font-style: italic }
a { color: #0000ee; display: inline }
h1 { font-weight: bold; font-size: 2em }
h2 { font-weight: bold; font-size: 1.5em }
small { font-size: 0.8em; display: inline }
span { display: inline }
b, strong, i, em { display: inline }
code, kbd, samp { display: inline }
sub, sup, abbr, cite, q, mark, u, s, label, time { display: inline }
sub, sup { font-size: 0.83em }
sub { vertical-align: sub }
sup { vertical-align: super }
table { display: table; text-align: left }
thead { display: table-header-group }
tfoot { display: table-footer-group }
tr { display: table-row }
td, th { display: table-cell }
li { display: list-item }
center { display: block; text-align: center }
br { display: block; height: 1em }
hr { display: block; height: 1px; background-color: #cccccc }
input { display: inline-block; width: 150px; height: 1.4em; border: 1px solid #a9a9a9; background-color: #ffffff }
input[type=hidden] { display: none }
textarea { display: inline-block; width: 200px; height: 3em; border: 1px solid #a9a9a9; background-color: #ffffff }
select, button { display: inline-block; border: 1px solid #a9a9a9; background-color: #efefef }
";

fn add_leveled(rules: &mut Vec<LeveledRule>, order: &mut usize, level: u8, sheet: Stylesheet) {
    for rule in sheet.rules {
        rules.push(LeveledRule {
            level,
            order: *order,
            rule,
        });
        *order += 1;
    }
    // `@media` blocks whose condition matches the viewport contribute their rules
    // at the same level, after the sheet's top-level rules (so they win on order,
    // as in CSS). Non-matching / non-`screen` blocks are dropped. Nested at-rules
    // inside a matched block are handled one level deep (the common case).
    for at in sheet.at_rules {
        let apply = match at.name.as_str() {
            "media" => media_matches(&at.prelude, VIEWPORT_WIDTH.get()),
            "supports" => supports_matches(&at.prelude),
            _ => false,
        };
        if apply {
            add_leveled(rules, order, level, parse_stylesheet(&at.body));
        }
    }
}

/// Whether an `@supports` condition holds for this engine. We render like a modern
/// browser, so a bare feature test `(prop: value)` is treated as supported; `not`
/// negates and `and`/`or` combine. This makes progressive-enhancement blocks —
/// e.g. Wikipedia's `@supports (display:grid){ .vector-pinned-container{display:
/// block} }`, which un-hides the sidebar TOC + appearance panel — actually apply,
/// while their `not(...)` legacy fallbacks stay dropped.
fn supports_matches(cond: &str) -> bool {
    supports_or(&cond.trim().to_ascii_lowercase())
}

/// Split the (already lowercased) `s` on the whole word `op` (space-delimited) at
/// paren depth 0.
fn split_top<'a>(s: &'a str, op: &str) -> Vec<&'a str> {
    let needle = format!(" {op} ");
    let (mut out, mut depth, mut start, bytes) = (Vec::new(), 0i32, 0usize, s.as_bytes());
    let mut i = 0;
    while i < s.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && s[i..].starts_with(&needle) {
            out.push(&s[start..i]);
            i += needle.len();
            start = i;
            continue;
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}

fn supports_or(s: &str) -> bool {
    let parts = split_top(s, "or");
    if parts.len() > 1 {
        return parts.iter().any(|p| supports_and(p.trim()));
    }
    supports_and(s)
}

fn supports_and(s: &str) -> bool {
    let parts = split_top(s, "and");
    if parts.len() > 1 {
        return parts.iter().all(|p| supports_unary(p.trim()));
    }
    supports_unary(s)
}

fn supports_unary(s: &str) -> bool {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("not ").or_else(|| s.strip_prefix("not(")) {
        // `not(` keeps the paren the strip consumed only for the spaced form; re-add
        // it for the parenless `not(...)` case so the group parses.
        let inner = if s.starts_with("not(") { &s[3..] } else { rest };
        return !supports_unary(inner);
    }
    // A parenthesised group: an operator expression (recurse) or a leaf test.
    if let Some(inner) = strip_group(s) {
        if inner.contains('(')
            || split_top(inner, "and").len() > 1
            || split_top(inner, "or").len() > 1
        {
            return supports_or(inner);
        }
        // Leaf `(property: value)`: a modern browser supports it.
        return true;
    }
    // Bare/unrecognised condition — assume supported.
    true
}

/// The inside of `s` if it is exactly one matching-paren group, else `None`.
fn strip_group(s: &str) -> Option<&str> {
    let inner = s.trim().strip_prefix('(')?.strip_suffix(')')?;
    // Confirm the opening paren matches the closing one (not `(a) and (b)`).
    let mut depth = 0i32;
    for ch in inner.chars() {
        match ch {
            '(' => depth += 1,
            ')' if depth == 0 => return None,
            ')' => depth -= 1,
            _ => {}
        }
    }
    (depth == 0).then_some(inner)
}

/// The viewport width (px) `@media` width conditions are evaluated against for the
/// current cascade build. A thread-local so `build_cascade`'s signature (used by
/// many callers) stays unchanged; the screenshot path sets it per render via
/// [`build_cascade_with_width`].
mod viewport {
    use std::cell::Cell;
    thread_local!(static WIDTH: Cell<f32> = const { Cell::new(1280.0) });
    pub(super) struct ViewportWidth;
    impl ViewportWidth {
        pub(super) fn get(&self) -> f32 {
            WIDTH.with(Cell::get)
        }
        pub(super) fn set(&self, w: f32) {
            WIDTH.with(|c| c.set(w));
        }
    }
}
static VIEWPORT_WIDTH: viewport::ViewportWidth = viewport::ViewportWidth;

/// Whether a `@media` query's condition matches `width` (px). Supports a comma
/// list (any clause matches), the `screen`/`all` types (`print` never matches;
/// no/other type is treated as applicable), and `min-width`/`max-width` features
/// in `px`/`em`/`rem` (`em`/`rem` against a 16px root). Other features are ignored
/// (a clause with only unknown features still matches if its type does).
fn media_matches(query: &str, width: f32) -> bool {
    let q = query.to_ascii_lowercase();
    q.split(',')
        .any(|clause| clause_matches(clause.trim(), width))
}

fn clause_matches(clause: &str, width: f32) -> bool {
    if clause.contains("print") {
        return false;
    }
    for feat in clause.split("and") {
        let feat = feat.trim();
        if let Some(min) = feature_len(feat, "min-width") {
            if width < min {
                return false;
            }
        }
        if let Some(max) = feature_len(feat, "max-width") {
            if width > max {
                return false;
            }
        }
    }
    true
}

/// The px value of a `(min-width: …)` / `(max-width: …)` feature in `clause`.
fn feature_len(feat: &str, name: &str) -> Option<f32> {
    let rest = feat.split_once(name)?.1.trim_start();
    // Drop the leading `:` and the feature's own closing paren; a `calc(...)` value
    // keeps its own parens, so strip exactly one trailing `)` (the feature's).
    let val = rest.strip_prefix(':')?.trim();
    let val = val.strip_suffix(')').unwrap_or(val).trim();
    if let Some(inner) = val.strip_prefix("calc(").and_then(|s| s.strip_suffix(')')) {
        return eval_calc(inner);
    }
    parse_len(val)
}

/// A single length token → px (`px` bare, or `em`/`rem` against a 16px root).
fn parse_len(val: &str) -> Option<f32> {
    let val = val.trim();
    if let Some(em) = val.strip_suffix("rem").or_else(|| val.strip_suffix("em")) {
        return em.trim().parse::<f32>().ok().map(|n| n * 16.0);
    }
    val.trim_end_matches("px").trim().parse::<f32>().ok()
}

/// Evaluate a minimal `calc()` body of two length terms — `A + B` / `A - B` (as in
/// Vector's `calc(1120px - 1px)` breakpoint). Falls back to the single term, or a
/// bare parse, so unknown shapes don't silently drop the whole media condition.
fn eval_calc(inner: &str) -> Option<f32> {
    for (op, sign) in [(" - ", -1.0), (" + ", 1.0)] {
        if let Some((a, b)) = inner.split_once(op) {
            return Some(parse_len(a)? + sign * parse_len(b)?);
        }
    }
    parse_len(inner)
}

/// The user-agent stylesheet, parsed once. It is a fixed constant, so parsing it
/// on every `build_cascade` (i.e. every render) is pure waste; cache the parse
/// and clone its rules into each cascade instead.
static UA_SHEET: std::sync::LazyLock<Stylesheet> =
    std::sync::LazyLock::new(|| parse_stylesheet(UA_CSS));

/// Build a [`Cascade`] from author CSS, render-time node-style CSS, and tokens.
/// The UA stylesheet is layered underneath automatically (§4.2).
pub fn build_cascade(author_css: &str, node_style_css: &str, tokens: TokenSet) -> Cascade {
    let mut rules = Vec::new();
    let mut order = 0;
    add_leveled(&mut rules, &mut order, 0, UA_SHEET.clone());
    add_leveled(&mut rules, &mut order, 1, parse_stylesheet(author_css));
    add_leveled(&mut rules, &mut order, 3, parse_stylesheet(node_style_css));
    Cascade { rules, tokens }
}

/// Like [`build_cascade`] but evaluates `@media` width conditions against
/// `viewport_width` px (instead of the 1280px desktop default). The screenshot
/// tier passes its viewport so responsive stylesheets pick the right breakpoint.
pub fn build_cascade_with_width(
    author_css: &str,
    node_style_css: &str,
    tokens: TokenSet,
    viewport_width: f32,
) -> Cascade {
    VIEWPORT_WIDTH.set(viewport_width);
    let cascade = build_cascade(author_css, node_style_css, tokens);
    VIEWPORT_WIDTH.set(1280.0);
    cascade
}
