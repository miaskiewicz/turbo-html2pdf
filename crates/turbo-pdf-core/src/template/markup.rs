//! Parse rendered markup into the resolved node tree (§1 Stage 1, §2.1). The
//! Jinja pass produces an intermediate markup string; html5ever turns it into a
//! DOM, which we lower into our own owned [`Node`] tree, recognizing `t:`
//! elements as typed directives. The markup string never escapes to the caller.

use html5ever::tendril::TendrilSink;
use html5ever::{parse_document, ParseOpts};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::error::{ErrorCode, RenderError, Span};
use crate::node::{t_kind, Attr, Element, Node, Tag};

fn markup_err(code: ErrorCode, message: impl Into<String>) -> RenderError {
    RenderError {
        code,
        message: message.into(),
        span: Span::default(),
    }
}

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
fn expand_self_closing(markup: &str) -> String {
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

/// Parse `markup` into the top-level flow nodes (the `<body>` children).
pub fn parse(markup: &str) -> Result<Vec<Node>, RenderError> {
    let expanded = expand_self_closing(markup);
    let dom = parse_document(RcDom::default(), ParseOpts::default())
        .from_utf8()
        .read_from(&mut expanded.as_bytes())
        .expect("reading from an in-memory slice is infallible");
    let body =
        find_element(&dom.document, "body").expect("the HTML parser always produces a <body>");
    let mut out = Vec::new();
    convert_children(&body, &mut out)?;
    Ok(out)
}

/// Depth-first search for the first element with the given local name.
fn find_element(handle: &Handle, name: &str) -> Option<Handle> {
    for child in handle.children.borrow().iter() {
        if let Some(found) = match_or_descend(child, name) {
            return Some(found);
        }
    }
    None
}

fn match_or_descend(child: &Handle, name: &str) -> Option<Handle> {
    if let NodeData::Element { name: qual, .. } = &child.data {
        if qual.local.as_ref() == name {
            return Some(child.clone());
        }
    }
    find_element(child, name)
}

fn convert_children(parent: &Handle, out: &mut Vec<Node>) -> Result<(), RenderError> {
    for child in parent.children.borrow().iter() {
        if let Some(node) = convert_node(child)? {
            out.push(node);
        }
    }
    Ok(())
}

fn convert_node(handle: &Handle) -> Result<Option<Node>, RenderError> {
    match &handle.data {
        NodeData::Text { contents } => Ok(Some(Node::Text(contents.borrow().to_string()))),
        NodeData::Element { name, attrs, .. } => {
            let local = name.local.to_string();
            let element = convert_element(&local, &collect_attrs(attrs), handle)?;
            Ok(Some(element))
        }
        _ => Ok(None),
    }
}

fn collect_attrs(attrs: &std::cell::RefCell<Vec<html5ever::Attribute>>) -> Vec<Attr> {
    attrs
        .borrow()
        .iter()
        .map(|a| Attr {
            name: a.name.local.to_string(),
            value: a.value.to_string(),
        })
        .collect()
}

fn convert_element(local: &str, attrs: &[Attr], handle: &Handle) -> Result<Node, RenderError> {
    let tag = tag_of(local)?;
    let mut children = Vec::new();
    convert_children(handle, &mut children)?;
    Ok(Node::Element(Element {
        tag,
        attrs: attrs.to_vec(),
        children,
    }))
}

fn tag_of(local: &str) -> Result<Tag, RenderError> {
    match local.strip_prefix("t:") {
        Some(rest) => t_kind(rest)
            .map(Tag::Directive)
            .ok_or_else(|| unknown_t(rest)),
        None => Ok(Tag::Html(local.to_string())),
    }
}

fn unknown_t(rest: &str) -> RenderError {
    markup_err(
        ErrorCode::UnknownElement,
        format!("unknown t: element 't:{rest}'"),
    )
}
