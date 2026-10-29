//! Parse rendered markup into a [`Flow`] (§1 Stage 1, §2.1). The Jinja pass
//! produces an intermediate markup string; it is parsed into a turbo-dom
//! [`Tree`](turbo_dom::Tree) (html5ever defaults: scripting on,
//! so `<noscript>` content is raw text) whose `<body>` children are the flow. `t:`
//! elements are recognized as directives by local name ([`crate::markers`]); an
//! unknown `t:` name is a [`ErrorCode::UnknownElement`] error. The markup string
//! never escapes to the caller.

use turbo_dom::rtdom::tree::NodeType;
use turbo_dom::{Handle, ParseOptions, Tree};
use turbo_dom_layout::TreeView;

use crate::error::{ErrorCode, RenderError, Span};
use crate::flow::Flow;
use crate::markers::t_kind;

/// The element name in an open tag prefix like `<t:region slot="x"` (no `<`/ws).
fn elem_name(open: &str) -> &str {
    let body = &open[1..];
    let end = body.find(char::is_whitespace).unwrap_or(body.len());
    &body[..end]
}

/// Rewrite one `<t:...>` tag starting at `s`, expanding a self-closing form into
/// an explicit open+close pair. Returns (bytes consumed, replacement).
fn rewrite_one(s: &str) -> (usize, String) {
    let Some(gt) = s.find('>') else {
        return (s.len(), s.to_string());
    };
    let tag = &s[..=gt];
    match tag.strip_suffix("/>") {
        Some(open) => (gt + 1, format!("{open}></{}>", elem_name(open))),
        None => (gt + 1, tag.to_string()),
    }
}

/// Expand self-closing `t:` convenience elements (`<t:page/>`) into balanced
/// pairs (`<t:page></t:page>`), since the HTML parser does not honor `/>` on
/// unknown elements and would otherwise nest following siblings inside them.
pub(crate) fn expand_self_closing(markup: &str) -> String {
    let mut out = String::with_capacity(markup.len());
    let mut rest = markup;
    while let Some(idx) = rest.find("<t:") {
        out.push_str(&rest[..idx]);
        let after = &rest[idx..];
        let (consumed, rewritten) = rewrite_one(after);
        out.push_str(&rewritten);
        rest = &after[consumed..];
    }
    out.push_str(rest);
    out
}

/// Parse `markup` into its flow: `<t:…/>` expanded, parsed into a tree, every `t:`
/// element in the body content checked (the first unknown one, in document
/// order, is an error).
pub(crate) fn parse(markup: &str) -> Result<Flow, RenderError> {
    let tree = parse_tree(markup)?;
    Ok(Flow::from_body(tree))
}

/// [`parse`] stopping at the checked tree (the HTML drive styles it whole).
pub(crate) fn parse_tree(markup: &str) -> Result<Tree, RenderError> {
    let tree = Tree::parse_with(
        &expand_self_closing(markup),
        ParseOptions { scripting: true },
    );
    check_directives(&tree)?;
    Ok(tree)
}

/// The document's `<body>` (the parser always produces one).
pub(crate) fn body_of(tree: &Tree) -> Handle {
    TreeView::new(tree)
        .find_element("body")
        .expect("the HTML parser always produces a <body>")
}

/// Reject the first unknown `t:` element in the body content (pre-order; elements
/// and text only, `<template>` content excluded).
fn check_directives(tree: &Tree) -> Result<(), RenderError> {
    let view = TreeView::new(tree);
    let mut pending: Vec<Handle> = view.children(body_of(tree));
    pending.reverse();
    while let Some(node) = pending.pop() {
        if tree.node_type(node) != NodeType::Element {
            continue;
        }
        if let Some(rest) = tree.local_name(node).and_then(|n| n.strip_prefix("t:")) {
            if t_kind(rest).is_none() {
                return Err(unknown_t(rest));
            }
        }
        pending.extend(view.children(node).into_iter().rev());
    }
    Ok(())
}

fn unknown_t(rest: &str) -> RenderError {
    RenderError {
        code: ErrorCode::UnknownElement,
        message: format!("unknown t: element 't:{rest}'"),
        span: Span::default(),
    }
}
