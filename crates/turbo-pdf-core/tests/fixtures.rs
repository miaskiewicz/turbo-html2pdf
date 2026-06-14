//! Realistic test corpus (§14). Each fixture is a directory under
//! `tests/fixtures/<name>/` holding a `template.html`, an optional `style.css`,
//! and a `data.json`. This test drives every fixture through the *existing*
//! pipeline — template string → `compile` → `render_nodes` → `build_cascade` +
//! `style_tree` → `layout` — and asserts the produced galley actually carries
//! laid-out text. It exists so the Phase 9 emitter's golden/e2e tests have a
//! vetted, render-clean corpus to consume.
//!
//! Determinism: `render_nodes` is called with `now = Some(0)` so any `now()` /
//! `date(now(), …)` usage is reproducible (§3.3).

mod common;

use turbo_pdf_core::layout::fragment::{Fragment, FragmentContent};
use turbo_pdf_core::style::TokenSet;
use turbo_pdf_core::{
    build_cascade, compile, layout, style_tree, CompileOptions, Diagnostics, StyledNode,
};

/// The four fixtures that make up the corpus.
const FIXTURES: &[&str] = &["invoice", "report", "legal", "mixed"];

/// Absolute path to a file inside a fixture directory.
fn fixture_path(name: &str, file: &str) -> String {
    format!(
        "{}/tests/fixtures/{name}/{file}",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// Read a fixture file, or `None` if it does not exist (used for optional CSS).
fn read_optional(name: &str, file: &str) -> Option<String> {
    std::fs::read_to_string(fixture_path(name, file)).ok()
}

/// Read a required fixture file, panicking with the path on failure.
fn read_required(name: &str, file: &str) -> String {
    let path = fixture_path(name, file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// Recursively count the `TextLine` fragments in a galley.
fn count_text_lines(frag: &Fragment) -> usize {
    let here = usize::from(matches!(frag.content, FragmentContent::TextLine { .. }));
    here + frag.children.iter().map(count_text_lines).sum::<usize>()
}

/// Total number of fragments in the galley (root included).
fn count_fragments(frag: &Fragment) -> usize {
    1 + frag.children.iter().map(count_fragments).sum::<usize>()
}

/// Run one fixture all the way through the pipeline and return its galley.
fn lay_out_fixture(name: &str) -> Fragment {
    // 1. Load the template, optional stylesheet, and JSON data.
    let template = read_required(name, "template.html");
    let css = read_optional(name, "style.css").unwrap_or_default();
    let data_src = read_required(name, "data.json");
    let data: serde_json::Value =
        serde_json::from_str(&data_src).unwrap_or_else(|e| panic!("parse {name}/data.json: {e}"));

    // 2. Compile — the corpus must be free of template syntax errors.
    let (program, diags) = compile(&template, &CompileOptions::default())
        .unwrap_or_else(|e| panic!("compile {name}: {:?}", e.code));
    assert!(diags.is_empty(), "{name}: compile diagnostics: {diags:?}");

    // 3. Render to the node tree with the clock pinned for determinism.
    let (nodes, rdiags) = program
        .render_nodes(&data, Some(0))
        .unwrap_or_else(|e| panic!("render {name}: {:?}", e.code));
    assert!(rdiags.is_empty(), "{name}: render diagnostics: {rdiags:?}");
    assert!(!nodes.is_empty(), "{name}: rendered an empty node tree");

    // 4. Cascade the author CSS over the UA defaults into a styled tree.
    let cascade = build_cascade(&css, "", TokenSet::default());
    let styled: Vec<StyledNode> = style_tree(&nodes, &cascade);
    assert!(!styled.is_empty(), "{name}: styled tree is empty");

    // 5. Lay the styled tree out into the galley at a fixed content width.
    let mut layout_diags = Diagnostics::default();
    layout(&styled, 540.0, &common::registry(), &mut layout_diags)
}

#[test]
fn every_fixture_compiles_and_renders() {
    for &name in FIXTURES {
        let galley = lay_out_fixture(name);

        // The galley root must carry laid-out content: real fragments and at
        // least some rendered text. An empty galley would mean the fixture
        // silently produced nothing layout could place.
        let fragments = count_fragments(&galley);
        let text_lines = count_text_lines(&galley);
        assert!(
            fragments > 1,
            "{name}: galley has no child fragments (count = {fragments})"
        );
        assert!(
            text_lines > 0,
            "{name}: galley produced no text lines (count = {text_lines})"
        );
    }
}

#[test]
fn fixtures_are_deterministic() {
    // Re-laying the same fixture with the pinned clock yields the same fragment
    // and text-line counts every time.
    for &name in FIXTURES {
        let a = lay_out_fixture(name);
        let b = lay_out_fixture(name);
        assert_eq!(
            count_fragments(&a),
            count_fragments(&b),
            "{name}: fragment count not deterministic"
        );
        assert_eq!(
            count_text_lines(&a),
            count_text_lines(&b),
            "{name}: text-line count not deterministic"
        );
    }
}
