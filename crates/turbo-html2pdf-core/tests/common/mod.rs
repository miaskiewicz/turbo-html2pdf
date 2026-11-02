//! Shared helpers for the integration tests: turbo-dom-layout's `test-support` font
//! fixtures (read from turbo-dom-layout's own `assets/fonts/` — this crate keeps no
//! copy; never a system path) and `ComputedStyle` builder, plus html2pdf-mode styling
//! of tree literals (`t:` directive markers) and of rendered flows. `dead_code` /
//! `unused_imports` are allowed because each test binary uses a subset.
#![allow(dead_code, unused_imports)]

pub use turbo_dom_layout::test_support::{
    evolventa, evolventa_bold, font_bytes, go, registry, style,
};

use turbo_dom_layout::test_support::literal;

/// The `t:` element name of a directive marker (for tree literals).
pub fn marker_name(kind: turbo_html2pdf_core::MarkerKind) -> String {
    let tkind = turbo_html2pdf_core::TKind::from_marker(kind).expect("a t: directive marker");
    format!("t:{}", tkind.name())
}

/// Style tree literals as a detached flow in html2pdf mode: built into a turbo-dom
/// Tree (`t:` names for directive markers) and styled through the Tree input with
/// html2pdf's directive classifier and this build's style options.
pub fn style_nodes(
    nodes: &[literal::Node],
    cascade: &turbo_html2pdf_core::Cascade,
) -> Vec<turbo_html2pdf_core::StyledNode> {
    let copy = literal::tree_from_nodes(nodes, &marker_name);
    turbo_html2pdf_core::style_input(
        &copy.input_with(&turbo_html2pdf_core::markers::classify),
        cascade,
        &options(),
    )
}

/// Style a rendered template [`Flow`](turbo_html2pdf_core::Flow).
pub fn style_flow(
    flow: &turbo_html2pdf_core::Flow,
    cascade: &turbo_html2pdf_core::Cascade,
) -> Vec<turbo_html2pdf_core::StyledNode> {
    flow.style(cascade)
}

/// A rendered flow's content as tree literals (directives as markers).
pub fn flow_nodes(flow: &turbo_html2pdf_core::Flow) -> Vec<literal::Node> {
    literal::from_tree(&flow.view(), flow.roots())
}

/// This build's style options (`cmyk(...)` with `print-color`), as the engine uses.
pub fn options() -> turbo_html2pdf_core::style::StyleOptions {
    turbo_html2pdf_core::style::StyleOptions {
        accept_cmyk: cfg!(feature = "print-color"),
    }
}
