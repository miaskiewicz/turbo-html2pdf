//! Galley fragment types (§5.5): construction, `bottom`, recursive `translate`,
//! break metadata, and node-id round-trip (AC-5.11).
//!
//! Split at the S1 relocation: the layout-only cases moved to turbo-dom-layout's
//! `tests/layout_fragment.rs`; the ones here need TKind directives, templating or the
//! PDF emitter.

mod common;

use turbo_html2pdf_core::layout::fragment::*;
use turbo_html2pdf_core::node::TKind;

#[test]
fn directive_content_roundtrips() {
    let f = Fragment::new(
        NodeId(9),
        0.0,
        0.0,
        0.0,
        0.0,
        FragmentContent::Directive(TKind::Footnote),
    );
    match f.content {
        FragmentContent::Directive(k) => assert_eq!(k, TKind::Footnote),
        _ => panic!("expected directive"),
    }
}
