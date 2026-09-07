//! CSS selectors for the v1 subset (§4.2): type, universal, `.class`, `#id`,
//! attribute selectors, structural + stateful pseudo-classes
//! (`:first-child`/`:last-child`/`:nth-child`/`:nth-of-type`/`:only-child`/
//! `:first-of-type`/`:last-of-type`/`:only-of-type`/`:root`/`:empty`/`:not()`/
//! `:checked`/`:enabled`/`:disabled`), and all four combinators
//! (descendant, child `>`, next-sibling `+`, subsequent-sibling `~`). Interactive
//! pseudo-classes (`:hover`/`:focus`/`:active`/`:target`/`:visited`/…) parse but
//! never match — a static screenshot is the resting state, so styles they gate
//! (e.g. hover-revealed menus) stay in their default (hidden) state.

/// CSS specificity as (ids, classes+attrs+pseudo-classes, type/element).
pub type Specificity = (u32, u32, u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
    NextSibling,
    SubsequentSibling,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttrOp {
    Exists,
    Equals,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttrSel {
    pub name: String,
    pub op: AttrOp,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pseudo {
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
    OnlyOfType,
    NthChild(i32, i32),
    NthOfType(i32, i32),
    Root,
    Empty,
    Checked,
    Enabled,
    Disabled,
    /// `:link` / `:any-link` — an unvisited hyperlink (an `<a>`/`<area>`/`<link>`
    /// with `href`). In a static render every link is unvisited, so this matches
    /// all of them (and `:visited` never does).
    Link,
    /// `:not(...)` — matches when the element matches none of the inner compound
    /// selectors (a simple selector list; combinators inside `:not` are not split).
    Not(Vec<Compound>),
    /// `:is(...)` / `:where(...)` (+ `:matches`/`-webkit-any`) — matches when the
    /// element matches ANY inner compound. Without this the arg is dropped and the
    /// bare compound matches everything (e.g. `a:where(.new)` reddening every link).
    /// Specificity differences between `:is`/`:where` are not modeled.
    Is(Vec<Compound>),
    /// An interactive/dynamic pseudo-class (`:hover`/`:focus`/`:active`/`:target`/
    /// `:visited`/…) — never matches in a static render (resting state).
    NeverMatch,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Compound {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<AttrSel>,
    pub pseudos: Vec<Pseudo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    pub compounds: Vec<Compound>,
    pub combinators: Vec<Combinator>,
    pub specificity: Specificity,
}

// --------------------------------------------------------------------------
// tokenizing a complex selector into compounds + combinators
// --------------------------------------------------------------------------

enum Token {
    Compound(String),
    Explicit(Combinator),
    Space,
}

#[derive(Default)]
struct Lexer {
    buf: String,
    tokens: Vec<Token>,
    in_attr: bool,
    /// Paren depth — a combinator/space char inside `:nth-child(2n+1)` or
    /// `:not(a + b)` is part of the argument, not a top-level combinator.
    parens: u32,
}

impl Lexer {
    fn flush(&mut self) {
        if !self.buf.is_empty() {
            self.tokens
                .push(Token::Compound(std::mem::take(&mut self.buf)));
        }
    }

    /// Whether combinator/whitespace chars are structural here (not inside an
    /// attribute selector or a pseudo-class argument).
    fn top_level(&self) -> bool {
        !self.in_attr && self.parens == 0
    }

    fn feed(&mut self, ch: char) {
        match ch {
            '[' => self.attr(ch, true),
            ']' => self.attr(ch, false),
            '(' => self.paren(ch, 1),
            ')' => self.paren(ch, -1),
            '>' if self.top_level() => self.combinator(Combinator::Child),
            '+' if self.top_level() => self.combinator(Combinator::NextSibling),
            '~' if self.top_level() => self.combinator(Combinator::SubsequentSibling),
            c if c.is_whitespace() && self.top_level() => self.space(),
            c => self.buf.push(c),
        }
    }

    fn attr(&mut self, ch: char, opening: bool) {
        self.in_attr = opening;
        self.buf.push(ch);
    }

    fn paren(&mut self, ch: char, delta: i32) {
        self.parens = self.parens.saturating_add_signed(delta);
        self.buf.push(ch);
    }

    fn combinator(&mut self, comb: Combinator) {
        self.flush();
        self.tokens.push(Token::Explicit(comb));
    }

    fn space(&mut self) {
        self.flush();
        self.tokens.push(Token::Space);
    }
}

fn lex(sel: &str) -> Vec<Token> {
    let mut lexer = Lexer::default();
    for ch in sel.chars() {
        lexer.feed(ch);
    }
    lexer.flush();
    lexer.tokens
}

#[derive(Default)]
struct Builder {
    compounds: Vec<Compound>,
    combinators: Vec<Combinator>,
    pending: Option<Combinator>,
}

impl Builder {
    fn add_compound(&mut self, text: &str) {
        if !self.compounds.is_empty() {
            self.combinators
                .push(self.pending.unwrap_or(Combinator::Descendant));
        }
        self.compounds.push(parse_compound(text));
        self.pending = None;
    }

    fn space(&mut self) {
        if self.pending.is_none() {
            self.pending = Some(Combinator::Descendant);
        }
    }

    fn take(&mut self, token: Token) {
        match token {
            Token::Compound(text) => self.add_compound(&text),
            Token::Explicit(comb) => self.pending = Some(comb),
            Token::Space => self.space(),
        }
    }
}

/// Parse one complex selector, or `None` if it is empty/invalid.
fn parse_selector(sel: &str) -> Option<Selector> {
    let mut builder = Builder::default();
    for token in lex(sel) {
        builder.take(token);
    }
    if builder.compounds.is_empty() {
        return None;
    }
    let specificity = specificity_of(&builder.compounds);
    Some(Selector {
        compounds: builder.compounds,
        combinators: builder.combinators,
        specificity,
    })
}

/// Parse a comma-separated selector list. Commas are split only at the top level:
/// a comma nested inside a functional pseudo-class (`:is(p, table)`,
/// `:not(a, b)`, `:nth-child(...)`), an `[attr="a,b"]` value, or a quoted string
/// is part of that argument, not a list separator. A naive `split(',')` tore
/// `:is(p,table,thead + tbody)` into a bare `table` selector that then matched
/// (and hid) every table on the page.
pub fn parse_selector_list(list: &str) -> Vec<Selector> {
    split_selector_list(list)
        .into_iter()
        .filter_map(|s| parse_selector(s.trim()))
        .collect()
}

/// Tracks `()`/`[]` nesting depth and `'`/`"` string state while scanning a
/// selector list, so only a TOP-LEVEL comma separates items.
#[derive(Default)]
struct Nest {
    parens: i32,
    brackets: i32,
    quote: Option<char>,
}

/// `depth ± 1` for an open/close bracket (never below zero).
fn step(depth: i32, opening: bool) -> i32 {
    if opening {
        depth + 1
    } else {
        depth.saturating_sub(1)
    }
}

impl Nest {
    /// Update string state for `c`; returns true while inside a string (so the
    /// char is inert for depth/comma purposes).
    fn in_quote(&mut self, c: char) -> bool {
        match self.quote {
            Some(q) => {
                if c == q {
                    self.quote = None;
                }
                true
            }
            None if c == '\'' || c == '"' => {
                self.quote = Some(c);
                true
            }
            None => false,
        }
    }

    fn update_depth(&mut self, c: char) {
        match c {
            '(' | ')' => self.parens = step(self.parens, c == '('),
            '[' | ']' => self.brackets = step(self.brackets, c == '['),
            _ => {}
        }
    }

    /// Feed one char; true only when `c` is a comma at the top level.
    fn is_top_comma(&mut self, c: char) -> bool {
        if self.in_quote(c) {
            return false;
        }
        self.update_depth(c);
        c == ',' && self.parens == 0 && self.brackets == 0
    }
}

/// Split a selector list on top-level commas, honoring `()`/`[]` nesting and
/// `'`/`"` strings.
fn split_selector_list(list: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut nest = Nest::default();
    let mut start = 0usize;
    for (i, c) in list.char_indices() {
        if nest.is_top_comma(c) {
            out.push(&list[start..i]);
            start = i + 1;
        }
    }
    out.push(&list[start..]);
    out
}

// --------------------------------------------------------------------------
// parsing a compound selector
// --------------------------------------------------------------------------

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_'
}

fn take_name(chars: &[char], start: usize) -> (String, usize) {
    let mut i = start;
    let mut name = String::new();
    while i < chars.len() && is_name_char(chars[i]) {
        name.push(chars[i]);
        i += 1;
    }
    (name, i)
}

fn parse_compound(text: &str) -> Compound {
    let chars: Vec<char> = text.chars().collect();
    let mut compound = Compound::default();
    let mut i = read_type(&chars, &mut compound);
    while i < chars.len() {
        i = read_piece(&chars, i, &mut compound);
    }
    compound
}

fn read_type(chars: &[char], compound: &mut Compound) -> usize {
    match chars.first() {
        Some('*') => 1,
        Some(c) if is_name_char(*c) => {
            let (name, next) = take_name(chars, 0);
            compound.tag = Some(name.to_ascii_lowercase());
            next
        }
        _ => 0,
    }
}

fn read_piece(chars: &[char], i: usize, compound: &mut Compound) -> usize {
    match chars[i] {
        '.' => push_class(chars, i, compound),
        '#' => push_id(chars, i, compound),
        '[' => push_attr(chars, i, compound),
        ':' => push_pseudo(chars, i, compound),
        _ => i + 1,
    }
}

fn push_class(chars: &[char], i: usize, compound: &mut Compound) -> usize {
    let (name, next) = take_name(chars, i + 1);
    compound.classes.push(name);
    next
}

fn push_id(chars: &[char], i: usize, compound: &mut Compound) -> usize {
    let (name, next) = take_name(chars, i + 1);
    compound.id = Some(name);
    next
}

fn slice_until(chars: &[char], start: usize, end: char) -> (String, usize) {
    let mut i = start;
    let mut out = String::new();
    while i < chars.len() && chars[i] != end {
        out.push(chars[i]);
        i += 1;
    }
    (out, (i + 1).min(chars.len()))
}

fn push_attr(chars: &[char], i: usize, compound: &mut Compound) -> usize {
    let (body, next) = slice_until(chars, i + 1, ']');
    compound.attrs.push(parse_attr(&body));
    next
}

fn push_pseudo(chars: &[char], i: usize, compound: &mut Compound) -> usize {
    let (name, after) = take_name(chars, i + 1);
    let (arg, next) = pseudo_arg(chars, after);
    if let Some(pseudo) = parse_pseudo(&name, &arg) {
        compound.pseudos.push(pseudo);
    }
    next
}

fn pseudo_arg(chars: &[char], i: usize) -> (String, usize) {
    if chars.get(i) == Some(&'(') {
        slice_until(chars, i + 1, ')')
    } else {
        (String::new(), i)
    }
}

// --------------------------------------------------------------------------
// attribute + pseudo value parsing
// --------------------------------------------------------------------------

const ATTR_OPS: [(&str, AttrOp); 5] = [
    ("~=", AttrOp::Includes),
    ("|=", AttrOp::DashMatch),
    ("^=", AttrOp::Prefix),
    ("$=", AttrOp::Suffix),
    ("*=", AttrOp::Substring),
];

fn detect_op(body: &str) -> Option<(usize, AttrOp)> {
    ATTR_OPS
        .iter()
        .find_map(|(tok, op)| body.find(tok).map(|idx| (idx, op.clone())))
}

fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    trimmed.trim_matches('"').trim_matches('\'').to_string()
}

fn parse_attr(body: &str) -> AttrSel {
    let body = body.trim();
    if let Some((idx, op)) = detect_op(body) {
        return split_attr(body, idx, 2, op);
    }
    match body.find('=') {
        Some(idx) => split_attr(body, idx, 1, AttrOp::Equals),
        None => AttrSel {
            name: body.to_string(),
            op: AttrOp::Exists,
            value: String::new(),
        },
    }
}

fn split_attr(body: &str, idx: usize, op_len: usize, op: AttrOp) -> AttrSel {
    let name = body[..idx].trim().to_string();
    let value = unquote(&body[idx + op_len..]);
    AttrSel { name, op, value }
}

fn parse_pseudo(name: &str, arg: &str) -> Option<Pseudo> {
    match name {
        "first-child" => Some(Pseudo::FirstChild),
        "last-child" => Some(Pseudo::LastChild),
        "only-child" => Some(Pseudo::OnlyChild),
        "first-of-type" => Some(Pseudo::FirstOfType),
        "last-of-type" => Some(Pseudo::LastOfType),
        "only-of-type" => Some(Pseudo::OnlyOfType),
        "nth-child" => Some(nth(arg, Pseudo::NthChild)),
        "nth-of-type" => Some(nth(arg, Pseudo::NthOfType)),
        "root" => Some(Pseudo::Root),
        "empty" => Some(Pseudo::Empty),
        "checked" => Some(Pseudo::Checked),
        "enabled" => Some(Pseudo::Enabled),
        "disabled" => Some(Pseudo::Disabled),
        "not" => Some(Pseudo::Not(parse_not(arg))),
        "is" | "where" | "matches" | "-webkit-any" | "-moz-any" => Some(Pseudo::Is(parse_not(arg))),
        "link" | "any-link" => Some(Pseudo::Link),
        // Interactive / dynamic pseudo-classes: valid but never active in a
        // static render, so they never match (their gated styles stay off).
        "hover" | "focus" | "focus-within" | "focus-visible" | "active" | "target" | "visited"
        | "autofocus" | "default" | "placeholder-shown" => Some(Pseudo::NeverMatch),
        // A pseudo-*ELEMENT* (`::before`/`::after`/`::first-line`/…, whether written
        // with `::` or the legacy single `:`). turbo generates no pseudo-elements, so
        // their declared styles must NOT fall onto the originating element — the
        // selector matches nothing. (Wikipedia's `.vector-page-titlebar::after` sets
        // `height:1px;position:absolute`; leaking that onto the real titlebar collapsed
        // it to 1px so the `<h1>` overlapped the tabs below.)
        "before"
        | "after"
        | "first-line"
        | "first-letter"
        | "marker"
        | "selection"
        | "placeholder"
        | "backdrop"
        | "cue"
        | "file-selector-button"
        | "target-text"
        | "grammar-error"
        | "spelling-error"
        | "highlight" => Some(Pseudo::NeverMatch),
        // The empty extra `:` segment of a `::pseudo` (first colon parses as an empty
        // name), or any unknown pseudo: ignored (matches nothing extra).
        _ => None,
    }
}

/// Parse a `:not(...)` argument (a comma-separated simple-selector list) into the
/// compounds it forbids. Combinators inside `:not` are not supported (each clause
/// is treated as a single compound).
fn parse_not(arg: &str) -> Vec<Compound> {
    arg.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_compound)
        .collect()
}

fn nth(arg: &str, make: fn(i32, i32) -> Pseudo) -> Pseudo {
    let (a, b) = parse_nth(arg);
    make(a, b)
}

fn parse_nth(arg: &str) -> (i32, i32) {
    let cleaned: String = arg.chars().filter(|c| !c.is_whitespace()).collect();
    match cleaned.as_str() {
        "even" => (2, 0),
        "odd" => (2, 1),
        s if s.contains('n') => parse_an_b(s),
        s => (0, s.parse().unwrap_or(0)),
    }
}

fn parse_an_b(s: &str) -> (i32, i32) {
    let (a_part, b_part) = s.split_once('n').unwrap_or((s, ""));
    (coeff(a_part), b_part.parse().unwrap_or(0))
}

fn coeff(a_part: &str) -> i32 {
    match a_part {
        "" | "+" => 1,
        "-" => -1,
        other => other.parse().unwrap_or(1),
    }
}

// --------------------------------------------------------------------------
// specificity
// --------------------------------------------------------------------------

fn compound_specificity(compound: &Compound, acc: &mut Specificity) {
    acc.0 += u32::from(compound.id.is_some());
    acc.1 += (compound.classes.len() + compound.attrs.len() + compound.pseudos.len()) as u32;
    acc.2 += u32::from(compound.tag.is_some());
}

fn specificity_of(compounds: &[Compound]) -> Specificity {
    let mut acc = (0, 0, 0);
    for compound in compounds {
        compound_specificity(compound, &mut acc);
    }
    acc
}
