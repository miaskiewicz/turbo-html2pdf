//! Shared helpers for the layout integration tests: font fixtures loaded from
//! the in-repo `assets/fonts/` (never a system path) and a `ComputedStyle`
//! builder. `dead_code` is allowed because each test binary uses a subset.
#![allow(dead_code)]

use turbo_html2pdf_core::text::FontRegistry;
use turbo_html2pdf_core::{ComputedStyle, FontFace};

pub fn font_bytes(name: &str) -> Vec<u8> {
    let path = format!("{}/assets/fonts/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|_| panic!("fixture {path}"))
}

pub fn evolventa() -> FontFace {
    FontFace::from_bytes(font_bytes("Evolventa-zLXL.ttf"), "Evolventa", 400, false).unwrap()
}

pub fn evolventa_bold() -> FontFace {
    FontFace::from_bytes(
        font_bytes("EvolventaBold-55Xv.ttf"),
        "Evolventa",
        700,
        false,
    )
    .unwrap()
}

pub fn go() -> FontFace {
    FontFace::from_bytes(font_bytes("Go-Regular.ttf"), "Go", 400, false).unwrap()
}

/// A registry with Evolventa (regular + bold) and Go registered.
pub fn registry() -> FontRegistry {
    let mut reg = FontRegistry::new();
    reg.add(evolventa());
    reg.add(evolventa_bold());
    reg.add(go());
    reg
}

/// Build a `ComputedStyle` from `(property, value)` pairs.
pub fn style(pairs: &[(&str, &str)]) -> ComputedStyle {
    ComputedStyle::from_pairs(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
}

/// The `t:` element name of a directive marker (for `tree_from_nodes` copies).
pub fn marker_name(kind: turbo_html2pdf_core::MarkerKind) -> String {
    let tkind = turbo_html2pdf_core::TKind::from_marker(kind).expect("a t: directive marker");
    format!("t:{}", tkind.name())
}

/// Style a legacy `Node` forest through BOTH inputs — the legacy `style_tree` and
/// the turbo-dom Tree input (`tree_from_nodes` + `style_input` with html2pdf's
/// directive classifier) — assert they agree (up to the `source` fields) and
/// return the Tree-input result.
pub fn style_nodes(
    nodes: &[turbo_html2pdf_core::Node],
    cascade: &turbo_html2pdf_core::Cascade,
) -> Vec<turbo_html2pdf_core::StyledNode> {
    use turbo_html2pdf_core::style::{style_input, Roots, StyleInput, TreeView};
    let legacy = turbo_html2pdf_core::style::style_tree_with_options(nodes, cascade, &options());
    let copy = turbo_dom_layout::tree_from_nodes(nodes, &marker_name);
    let input = StyleInput {
        view: TreeView::new(&copy.tree).markers(&turbo_html2pdf_core::markers::classify),
        roots: Roots::Detached(&copy.roots),
    };
    let styled = style_input(&input, cascade, &options());
    let stripped: Vec<_> = styled.iter().map(|n| n.without_sources()).collect();
    assert_eq!(
        stripped, legacy,
        "legacy Node input == turbo-dom Tree input"
    );
    styled
}

/// Style a rendered template [`Flow`](turbo_html2pdf_core::Flow) and check it
/// against the legacy path over the same document (its `Node` copy through
/// `style_tree`); returns the Tree-input result.
pub fn style_flow(
    flow: &turbo_html2pdf_core::Flow,
    cascade: &turbo_html2pdf_core::Cascade,
) -> Vec<turbo_html2pdf_core::StyledNode> {
    let styled = flow.style(cascade);
    let stripped: Vec<_> = styled.iter().map(|n| n.without_sources()).collect();
    assert_eq!(
        stripped,
        turbo_html2pdf_core::style::style_tree_with_options(&flow.to_nodes(), cascade, &options()),
        "legacy Node input == turbo-dom Tree input"
    );
    styled
}

/// This build's style options (`cmyk(...)` with `print-color`), as the engine uses.
pub fn options() -> turbo_html2pdf_core::style::StyleOptions {
    turbo_html2pdf_core::style::StyleOptions {
        accept_cmyk: cfg!(feature = "print-color"),
    }
}
