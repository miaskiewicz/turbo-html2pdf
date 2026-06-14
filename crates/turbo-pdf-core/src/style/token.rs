//! Named style tokens (§4.3b): semantic property bundles referenced by
//! `t:style="a b"`, decoupled from selectors. Tokens may `extends` other tokens;
//! later tokens and the extending token win on conflict, resolved by cascade
//! source order.

use std::collections::BTreeMap;

use super::parser::Declaration;

/// A named bundle of declarations, optionally extending other tokens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StyleToken {
    pub extends: Vec<String>,
    pub declarations: Vec<Declaration>,
}

/// The registered token set, keyed by name.
pub type TokenSet = BTreeMap<String, StyleToken>;

/// Maximum `extends` resolution depth, guarding against cycles.
const MAX_DEPTH: u32 = 32;

fn resolve_into(name: &str, set: &TokenSet, depth: u32, out: &mut Vec<Declaration>) {
    if depth >= MAX_DEPTH {
        return;
    }
    let Some(token) = set.get(name) else {
        return;
    };
    for base in &token.extends {
        resolve_into(base, set, depth + 1, out);
    }
    out.extend(token.declarations.iter().cloned());
}

/// Resolve the declarations contributed by a `t:style="a b c"` attribute, in
/// application order (later declarations override earlier ones via cascade
/// source order).
pub fn resolve_tokens(t_style: &str, set: &TokenSet) -> Vec<Declaration> {
    let mut out = Vec::new();
    for name in t_style.split_whitespace() {
        resolve_into(name, set, 0, &mut out);
    }
    out
}
