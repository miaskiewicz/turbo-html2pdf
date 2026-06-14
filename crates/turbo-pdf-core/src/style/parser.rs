//! A small CSS parser for the v1 supported subset (§4.1). Hand-rolled rather
//! than built on Servo's `selectors`/`cssparser` stack: the subset is narrow,
//! and a focused parser keeps the engine dependency-light and deterministic.
//! At-rules (`@page`, `@media`, …) are sliced out here and handled elsewhere
//! (page geometry in the pagination phase); unknown at-rules are dropped.

use super::selector::{parse_selector_list, Selector};

/// A single `property: value` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub property: String,
    pub value: String,
    pub important: bool,
}

/// A style rule: a selector list sharing a declaration block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
}

/// A parsed stylesheet plus any at-rule preludes/bodies kept verbatim for later
/// phases (e.g. `@page`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    pub at_rules: Vec<AtRule>,
}

/// An at-rule kept for a later phase, e.g. `@page { ... }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtRule {
    pub name: String,
    pub prelude: String,
    pub body: String,
}

/// Remove `/* ... */` comments from CSS source.
pub fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(open) = rest.find("/*") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        rest = after.find("*/").map_or("", |close| &after[close + 2..]);
    }
    out.push_str(rest);
    out
}

/// Parse a single declaration like `font-weight: 700 !important`.
fn parse_declaration(text: &str) -> Option<Declaration> {
    let (raw_prop, raw_val) = text.split_once(':')?;
    let property = raw_prop.trim().to_ascii_lowercase();
    if property.is_empty() {
        return None;
    }
    let (value, important) = split_important(raw_val.trim());
    if value.is_empty() {
        return None;
    }
    Some(Declaration {
        property,
        value,
        important,
    })
}

fn split_important(value: &str) -> (String, bool) {
    match value.strip_suffix("!important") {
        Some(head) => (head.trim().to_string(), true),
        None => (value.trim().to_string(), false),
    }
}

/// Parse a `;`-separated declaration block body.
pub fn parse_declarations(body: &str) -> Vec<Declaration> {
    body.split(';').filter_map(parse_declaration).collect()
}

/// A top-level chunk of the stylesheet: either a qualified rule or an at-rule.
struct Chunk {
    prelude: String,
    body: String,
    is_at_rule: bool,
}

fn finish_chunk(prelude: &str, body: &str) -> Option<Chunk> {
    let trimmed = prelude.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(Chunk {
        prelude: trimmed.to_string(),
        body: body.to_string(),
        is_at_rule: trimmed.starts_with('@'),
    })
}

/// Split a stylesheet into top-level brace-delimited chunks, tracking nesting so
/// at-rule bodies with inner braces stay intact.
fn split_chunks(css: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut scan = Scan::default();
    for ch in css.chars() {
        scan.feed(ch, &mut chunks);
    }
    chunks
}

#[derive(Default)]
struct Scan {
    prelude: String,
    body: String,
    depth: u32,
}

impl Scan {
    fn feed(&mut self, ch: char, chunks: &mut Vec<Chunk>) {
        match ch {
            '{' if self.depth == 0 => self.depth = 1,
            '{' => self.push_nested(ch, true),
            '}' if self.depth == 1 => self.close(chunks),
            '}' => self.push_nested(ch, false),
            _ if self.depth == 0 => self.prelude.push(ch),
            _ => self.body.push(ch),
        }
    }

    fn push_nested(&mut self, ch: char, opening: bool) {
        self.depth = if opening {
            self.depth + 1
        } else {
            self.depth - 1
        };
        self.body.push(ch);
    }

    fn close(&mut self, chunks: &mut Vec<Chunk>) {
        if let Some(chunk) = finish_chunk(&self.prelude, &self.body) {
            chunks.push(chunk);
        }
        self.prelude.clear();
        self.body.clear();
        self.depth = 0;
    }
}

fn at_rule_from(chunk: &Chunk) -> AtRule {
    let prelude = chunk.prelude.trim_start_matches('@');
    let (name, rest) = prelude
        .split_once(char::is_whitespace)
        .unwrap_or((prelude, ""));
    AtRule {
        name: name.trim().to_ascii_lowercase(),
        prelude: rest.trim().to_string(),
        body: chunk.body.trim().to_string(),
    }
}

fn rule_from(chunk: &Chunk) -> Option<Rule> {
    let selectors = parse_selector_list(&chunk.prelude);
    if selectors.is_empty() {
        return None;
    }
    Some(Rule {
        selectors,
        declarations: parse_declarations(&chunk.body),
    })
}

fn absorb(sheet: &mut Stylesheet, chunk: &Chunk) {
    if chunk.is_at_rule {
        sheet.at_rules.push(at_rule_from(chunk));
    } else if let Some(rule) = rule_from(chunk) {
        sheet.rules.push(rule);
    }
}

/// Parse a CSS stylesheet into qualified rules and at-rules.
pub fn parse_stylesheet(css: &str) -> Stylesheet {
    let cleaned = strip_comments(css);
    let mut sheet = Stylesheet::default();
    for chunk in split_chunks(&cleaned) {
        absorb(&mut sheet, &chunk);
    }
    sheet
}
