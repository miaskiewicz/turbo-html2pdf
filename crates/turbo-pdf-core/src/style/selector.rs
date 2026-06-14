//! CSS selectors for the v1 subset (§4.2): type, universal, `.class`, `#id`,
//! attribute selectors, `:first-child`/`:last-child`/`:nth-child`/`:nth-of-type`,
//! and the descendant/child combinators. Sibling combinators (`+`, `~`) and
//! pseudo-elements are deferred to a later refinement.

/// CSS specificity as (ids, classes+attrs+pseudo-classes, type/element).
pub type Specificity = (u32, u32, u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pseudo {
    FirstChild,
    LastChild,
    NthChild(i32, i32),
    NthOfType(i32, i32),
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
}

impl Lexer {
    fn flush(&mut self) {
        if !self.buf.is_empty() {
            self.tokens
                .push(Token::Compound(std::mem::take(&mut self.buf)));
        }
    }

    fn feed(&mut self, ch: char) {
        match ch {
            '[' => self.attr(ch, true),
            ']' => self.attr(ch, false),
            '>' if !self.in_attr => self.combinator(Combinator::Child),
            c if c.is_whitespace() && !self.in_attr => self.space(),
            c => self.buf.push(c),
        }
    }

    fn attr(&mut self, ch: char, opening: bool) {
        self.in_attr = opening;
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

/// Parse a comma-separated selector list.
pub fn parse_selector_list(list: &str) -> Vec<Selector> {
    list.split(',')
        .filter_map(|s| parse_selector(s.trim()))
        .collect()
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
        "nth-child" => Some(nth(arg, Pseudo::NthChild)),
        "nth-of-type" => Some(nth(arg, Pseudo::NthOfType)),
        _ => None,
    }
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
