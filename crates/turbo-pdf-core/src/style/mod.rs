//! The style system (§4): CSS parsing, the selector engine, named style tokens,
//! render-time node styles, and cascade + inheritance into a styled tree.

mod cascade;
mod parser;
mod selector;
mod token;

pub use cascade::{style_tree, Cascade, ComputedStyle, LeveledRule, StyledElement, StyledNode};
pub use parser::{parse_stylesheet, AtRule, Declaration, Rule, Stylesheet};
pub use token::{StyleToken, TokenSet};

/// A minimal user-agent stylesheet (§4.2): the defaults the cascade starts from.
const UA_CSS: &str = "
b { font-weight: bold }
strong { font-weight: bold }
i { font-style: italic }
em { font-style: italic }
a { color: #0000ee }
h1 { font-weight: bold; font-size: 2em }
h2 { font-weight: bold; font-size: 1.5em }
small { font-size: 0.8em }
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
}

/// Build a [`Cascade`] from author CSS, render-time node-style CSS, and tokens.
/// The UA stylesheet is layered underneath automatically (§4.2).
pub fn build_cascade(author_css: &str, node_style_css: &str, tokens: TokenSet) -> Cascade {
    let mut rules = Vec::new();
    let mut order = 0;
    add_leveled(&mut rules, &mut order, 0, parse_stylesheet(UA_CSS));
    add_leveled(&mut rules, &mut order, 1, parse_stylesheet(author_css));
    add_leveled(&mut rules, &mut order, 3, parse_stylesheet(node_style_css));
    Cascade { rules, tokens }
}
