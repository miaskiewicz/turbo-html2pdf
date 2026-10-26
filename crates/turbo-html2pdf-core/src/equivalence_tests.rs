//! Consolidation S2 gate (html2pdf side): the turbo-dom Tree input styles every
//! template render and HTML document exactly like the legacy owned-`Node` input.
//!
//! The legacy reference is the pre-S2 pipeline, reconstructed from what remains
//! of it: `expand_self_closing` → html5ever/rcdom → `markup::parse_classified`
//! with the old `t:` tagging (unknown `t:` names are an error) → `style_tree`
//! (detached template flow, footnote paragraphs) or `style_tree_with_roots` over
//! the stripped body (the HTML path), plus the old `Node` endnote rewrite. The new
//! path is `Flow` / the Tree HTML drive. For every input the full `Result` —
//! errors included — must match, with the styled trees compared up to their new
//! `source` fields.
//!
//! Inputs: the in-repo template fixtures (rendered with their data), the
//! conformance fixtures (as templates and as HTML documents), the adversarial
//! documents below, plus every `*.html` under `TURBO_HTML2PDF_EQUIV_DIRS`
//! (`:`-separated, recursive; used locally for the consolidation corpora).

use std::path::{Path, PathBuf};

use turbo_dom_layout::html_layout::strip_non_visual;
use turbo_dom_layout::style::{style_tree_with_options, style_tree_with_roots_options, TokenSet};
use turbo_dom_layout::{
    build_cascade, build_cascade_with_width, collect_style_css, collect_style_css_tree,
    style_input, Cascade, Element, Node, Roots, StyleInput, StyledNode, Tag,
};

use crate::error::{ErrorCode, RenderError, Span};
use crate::flow::style_options;
use crate::markers::{t_kind, TKind};
use crate::template::markup::{self, expand_self_closing};

const ADVERSARIAL: &[(&str, &str)] = &[
    (
        "comment-between-siblings",
        "<style>p+p{color:red}</style><p>a</p><!-- c --><p>b</p>x<!--y-->z",
    ),
    (
        "style-first-child",
        "<style>p:first-child{color:red}</style><p>a</p>",
    ),
    (
        "script-sole-child",
        "<div><script>var a = '<p>';</script></div><style>div:empty{color:red}</style>",
    ),
    (
        "template",
        "<template><p>t</p></template><p>after</p><style>template+p{color:red}</style>",
    ),
    (
        "noscript-head-and-body",
        "<html><head><noscript><p>h</p></noscript></head><body><noscript><p>n</p></noscript><p>a</p></body></html>",
    ),
    (
        "svg-xlink-href",
        r##"<svg xmlns:xlink="http://www.w3.org/1999/xlink"><a xlink:href="#t"><text>x</text></a></svg><a href="#t">l</a>"##,
    ),
    (
        "content-in-head",
        "<html><head><p class=early>early</p></head><body><p>b</p></body></html>",
    ),
    (
        "template-with-unknown-t",
        "<template><t:bogus/></template><p>x</p>",
    ),
    ("svg-with-t-element", "<svg><t:x/></svg><p>y</p>"),
    ("unknown-t-in-body", "<p>a<t:nope>b</t:nope></p><t:also-bad/>"),
    (
        "directives",
        "<p>a<t:footnote mark='*'>n <b>bold</b><t:footnote>inner</t:footnote></t:footnote>b<t:page/>/<t:pages/></p>\
         <t:anchor name='x'/><t:region slot='h'>r</t:region><t:leader/><t:counter/>\
         <style>t\\:region{color:red} p > :first-child{margin:1px} p :nth-of-type(2){padding:2px}</style>",
    ),
    (
        "endnotes",
        "<div>a<t:endnote>one</t:endnote><div><t:endnote>two <i>it</i></t:endnote></div></div>\
         <div><t:endnotes/><t:endnotes/></div><div><t:endnotes/></div><t:endnotes/>",
    ),
    (
        "footnote-reset",
        "<p>x<t:footnote footnote-reset='page'>n1</t:footnote>y<t:footnote>n2<!--c--></t:footnote></p>",
    ),
    ("cmyk", "<div style='border: 1px solid; border-color: cmyk(0,0,0,1); color: cmyk(1,0,0,0)'>c</div>"),
    ("empty", ""),
];

// ----------------------------------------------------------------- legacy path

fn unknown_t(rest: &str) -> RenderError {
    RenderError {
        code: ErrorCode::UnknownElement,
        message: format!("unknown t: element 't:{rest}'"),
        span: Span::default(),
    }
}

/// The pre-S2 `t:` tagging (`markup.rs` `tag_of`).
fn legacy_tag(local: &str) -> Result<Tag, RenderError> {
    match local.strip_prefix("t:") {
        Some(rest) => t_kind(rest)
            .map(|k| Tag::Marker(k.marker()))
            .ok_or_else(|| unknown_t(rest)),
        None => Ok(Tag::Html(local.to_string())),
    }
}

fn legacy_parse(markup: &str) -> Result<(Vec<Node>, Vec<Element>), RenderError> {
    turbo_dom_layout::markup::parse_classified(&expand_self_closing(markup), &legacy_tag)
}

fn kind_of(node: &Node) -> Option<TKind> {
    match &node.as_element()?.tag {
        Tag::Marker(kind) => TKind::from_marker(*kind),
        Tag::Html(_) => None,
    }
}

fn element(tag: &str, class: &str, children: Vec<Node>) -> Node {
    Node::Element(Element {
        tag: Tag::Html(tag.to_string()),
        attrs: vec![crate::node::Attr {
            name: "class".to_string(),
            value: class.to_string(),
        }],
        children,
    })
}

/// The pre-S2 `Node` endnote rewrite (`endnotes::expand`), kept as the oracle.
fn legacy_endnotes(nodes: Vec<Node>) -> Vec<Node> {
    fn mark(nodes: Vec<Node>, collected: &mut Vec<Vec<Node>>) -> Vec<Node> {
        nodes
            .into_iter()
            .map(|node| {
                if kind_of(&node) == Some(TKind::Endnote) {
                    let Node::Element(el) = node else {
                        unreachable!()
                    };
                    collected.push(el.children);
                    return element(
                        "sup",
                        "endnote-ref",
                        vec![Node::Text(collected.len().to_string())],
                    );
                }
                match node {
                    Node::Element(mut el) => {
                        el.children = mark(el.children, collected);
                        Node::Element(el)
                    }
                    text => text,
                }
            })
            .collect()
    }
    fn place(nodes: Vec<Node>, collected: &[Vec<Node>], emitted: &mut bool) -> Vec<Node> {
        let mut out = Vec::new();
        for node in nodes {
            if kind_of(&node) == Some(TKind::Endnotes) {
                if !*emitted {
                    *emitted = true;
                    let items = collected
                        .iter()
                        .enumerate()
                        .map(|(i, body)| {
                            let mut children = vec![Node::Text(format!("{}. ", i + 1))];
                            children.extend(body.iter().cloned());
                            element("div", "endnote", children)
                        })
                        .collect();
                    out.push(element("div", "endnotes", items));
                }
                continue;
            }
            out.push(match node {
                Node::Element(mut el) if !*emitted => {
                    el.children = place(el.children, collected, emitted);
                    Node::Element(el)
                }
                other => other,
            });
        }
        out
    }
    let mut collected = Vec::new();
    let marked = mark(nodes, &mut collected);
    if collected.is_empty() {
        return marked;
    }
    place(marked, &collected, &mut false)
}

/// The pre-S2 footnote collection + `<p class="footnote">` note nodes.
fn legacy_notes(nodes: &[Node]) -> Vec<Vec<Node>> {
    fn collect(nodes: &[Node], out: &mut Vec<(Option<String>, Vec<Node>)>) {
        for node in nodes {
            let Some(el) = node.as_element() else {
                continue;
            };
            if kind_of(node) == Some(TKind::Footnote) {
                out.push((el.attr("mark").map(str::to_string), el.children.clone()));
            } else {
                collect(&el.children, out);
            }
        }
    }
    let mut found = Vec::new();
    collect(nodes, &mut found);
    let mut auto = 0;
    found
        .into_iter()
        .map(|(manual, body)| {
            let mark = manual.unwrap_or_else(|| {
                auto += 1;
                auto.to_string()
            });
            let mut children = vec![Node::Text(format!("{mark} "))];
            children.extend(body);
            vec![element("p", "footnote", children)]
        })
        .collect()
}

// ------------------------------------------------------------- comparison

fn stripped(nodes: &[StyledNode]) -> Vec<StyledNode> {
    nodes.iter().map(StyledNode::without_sources).collect()
}

fn same<T: std::fmt::Debug>(name: &str, what: &str, old: &T, new: &T) {
    assert_eq!(format!("{new:?}"), format!("{old:?}"), "{name}: {what}");
}

/// The template (PDF) path: body flow, endnotes, footnote paragraphs.
fn check_template_markup(name: &str, rendered: &str, cascade: &Cascade) {
    let options = style_options();
    let legacy = legacy_parse(rendered).map(|(nodes, _)| {
        let nodes = if cfg!(feature = "endnotes") {
            legacy_endnotes(nodes)
        } else {
            nodes
        };
        let body = style_tree_with_options(&nodes, cascade, &options);
        let notes: Vec<_> = legacy_notes(&nodes)
            .iter()
            .map(|note| style_tree_with_options(note, cascade, &options))
            .collect();
        (nodes, body, notes)
    });
    let new = markup::parse(rendered).map(|mut flow| {
        if cfg!(feature = "endnotes") {
            #[cfg(feature = "endnotes")]
            crate::endnotes::expand(&mut flow);
        }
        let nodes = flow.to_nodes();
        let body = stripped(&flow.style(cascade));
        let sources = crate::render::collect_footnotes(&flow);
        let marks = crate::render::continuous_marks(&sources);
        let bodies = crate::render::note_bodies(&mut flow, &sources);
        let notes: Vec<_> = bodies
            .iter()
            .zip(&marks)
            .map(|(body, mark)| {
                flow.tree_mut()
                    .set_node_value(body.mark, &format!("{mark} "));
                stripped(&flow.style_roots(&[body.paragraph], cascade))
            })
            .collect();
        (nodes, body, notes)
    });
    same(name, "template flow", &legacy, &new);
}

/// The HTML drive path (html2pdf's `layout_boxes`/`layout_html`): shells,
/// body-only `<style>`, stripped content.
fn check_html(name: &str, html: &str) {
    let options = style_options();
    let legacy = legacy_parse(html).map(|(nodes, roots)| {
        let css = collect_style_css(&nodes);
        let cascade = build_cascade_with_width(&css, "", TokenSet::default(), 1280.0);
        let styled =
            style_tree_with_roots_options(&strip_non_visual(nodes), &cascade, &roots, &options);
        (css, styled)
    });
    let new = markup::parse_tree(html).map(|tree| {
        let view = html2pdf_view(&tree);
        let body = markup::body_of(&tree);
        let css = collect_style_css_tree(&view, &view.children(body));
        let cascade = build_cascade_with_width(&css, "", TokenSet::default(), 1280.0);
        let styled = style_input(
            &StyleInput {
                view: view.strip_non_visual(true),
                roots: Roots::Shell,
            },
            &cascade,
            &options,
        );
        (css, stripped(&styled))
    });
    same(name, "HTML path", &legacy, &new);
}

/// html2pdf's view of a parsed document (the `t:` classifier).
fn html2pdf_view(tree: &turbo_dom::Tree) -> turbo_dom_layout::TreeView<'_> {
    turbo_dom_layout::TreeView::new(tree).markers(&crate::markers::classify)
}

/// Both paths for one source: as a template (when it compiles and renders) and as
/// a raw HTML document.
fn check(name: &str, source: &str, data: &serde_json::Value, css: &str) {
    check_html(name, source);
    let opts = crate::options::CompileOptions::default();
    let Ok((program, _)) = crate::compile(source, &opts) else {
        return;
    };
    let Ok((rendered, _)) = program.render_markup(data, Some(0)) else {
        return;
    };
    let cascade = build_cascade(css, "", TokenSet::default());
    check_template_markup(name, &rendered, &cascade);
}

fn html_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            html_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "html") {
            out.push(path);
        }
    }
}

/// The in-repo template fixtures (template + data + stylesheet); returns the count.
fn check_template_fixtures(root: &Path) -> usize {
    let mut dirs: Vec<_> = std::fs::read_dir(root.join("tests/fixtures"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|d| d.join("template.html").exists())
        .collect();
    dirs.sort();
    for dir in &dirs {
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).ok();
        let data = read("data.json").map_or(serde_json::Value::Null, |d| {
            serde_json::from_str(&d).expect("fixture data")
        });
        check(
            &dir.display().to_string(),
            &read("template.html").unwrap(),
            &data,
            &read("style.css").unwrap_or_default(),
        );
    }
    dirs.len()
}

/// The conformance fixtures plus `TURBO_HTML2PDF_EQUIV_DIRS`.
fn corpus_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    html_files(&root.join("../../benches/conformance/fixtures"), &mut files);
    let extra = std::env::var("TURBO_HTML2PDF_EQUIV_DIRS").unwrap_or_default();
    for dir in extra.split(':').filter(|d| !d.is_empty()) {
        html_files(Path::new(dir), &mut files);
    }
    files
}

#[test]
fn tree_input_matches_the_legacy_node_input() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let null = serde_json::Value::Null;
    for (name, html) in ADVERSARIAL {
        check(name, html, &null, "");
    }
    let fixtures = check_template_fixtures(&root);
    let files = corpus_files(&root);
    for path in &files {
        let html = std::fs::read_to_string(path).unwrap();
        check(&path.display().to_string(), &html, &null, "");
    }
    eprintln!(
        "html2pdf tree_input_equivalence: {} documents equal",
        ADVERSARIAL.len() + fixtures + files.len()
    );
}
