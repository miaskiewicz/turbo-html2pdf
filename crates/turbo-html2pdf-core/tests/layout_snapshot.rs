//! Split at the S1 relocation: the layout-only cases moved to turbo-dom-layout's
//! `tests/layout_snapshot.rs`; the ones here need TKind directives, templating or the
//! PDF emitter.

mod common;

use turbo_dom_layout::test_support::literal::{Element, Node};
use turbo_html2pdf_core::layout_snapshot::{
    snapshot_live_tree, GeometryKind, LayoutSnapshot, SnapshotInputs, SourceId, SourceIdentity,
};
use turbo_html2pdf_core::{
    build_cascade, layout_with_images, Attr, Diagnostics, FontRegistry, ImageCtx, NoImages, Tag,
};

fn element(tag: &str, css: &str, children: Vec<Node>) -> Node {
    Node::Element(Element {
        tag: Tag::Html(tag.into()),
        attrs: vec![Attr {
            name: "style".into(),
            value: css.into(),
        }],
        children,
    })
}

fn identity(id: u64, path: &[usize]) -> SourceIdentity {
    SourceIdentity {
        id: SourceId(id),
        path: path.to_vec(),
    }
}

fn capture(nodes: &[Node], ids: &[SourceIdentity]) -> LayoutSnapshot {
    let fonts = FontRegistry::new();
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: Some(200.0),
    };
    let inputs = SnapshotInputs {
        width: 200.0,
        height: 200.0,
        fonts: &fonts,
        images: &images,
    };
    snapshot_live_tree(
        &turbo_dom_layout::test_support::literal::tree(nodes).input(),
        ids,
        &build_cascade("", "", Default::default()),
        &inputs,
        &mut Diagnostics::default(),
    )
    .unwrap()
}

#[test]
fn directive_fragments_keep_actual_source_identity_and_category() {
    let node = Node::Element(Element {
        tag: Tag::Marker(turbo_html2pdf_core::TKind::Page.marker()),
        attrs: Vec::new(),
        children: Vec::new(),
    });
    let snapshot = capture(&[node], &[identity(51, &[0])]);
    let source = snapshot.source(SourceId(51)).unwrap();
    assert_eq!(source.fragment_indices.len(), 1);
    let fragment = &snapshot.fragments()[source.fragment_indices[0]];
    assert_eq!(fragment.kind, GeometryKind::Marker);
    assert!(fragment.used_box.is_none());
    assert_eq!(fragment.source_id, Some(SourceId(51)));
}

#[test]
fn hidden_directives_and_their_descendants_have_no_live_geometry() {
    let node = Node::Element(Element {
        tag: Tag::Marker(turbo_html2pdf_core::TKind::Page.marker()),
        attrs: vec![Attr {
            name: "style".into(),
            value: "display:none".into(),
        }],
        children: vec![element("div", "height:20px", Vec::new())],
    });
    let snapshot = capture(
        std::slice::from_ref(&node),
        &[identity(1, &[0]), identity(2, &[0, 0])],
    );
    for id in [1, 2] {
        let source = snapshot.source(SourceId(id)).unwrap();
        assert!(source.excluded_from_layout);
        assert!(source.fragment_indices.is_empty());
        assert!(source.descendant_fragment_indices.is_empty());
    }
    let styled = common::style_nodes(&[node], &build_cascade("", "", Default::default()));
    let galley = layout_with_images(
        &styled,
        200.0,
        &FontRegistry::new(),
        &ImageCtx {
            resolver: &NoImages,
            body_height: None,
        },
        &mut Diagnostics::default(),
    );
    assert!(!galley.children.is_empty());
}
