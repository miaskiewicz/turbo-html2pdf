//! Split at the S1 relocation: the layout-only cases moved to turbo-dom-layout's
//! `tests/native_inline_coverage.rs`; the ones here need TKind directives, templating
//! or the PDF emitter.
//!
//! The cases shape text with the default family, so they need the bundled faces (a
//! font-less `--no-default-features` build has no text sources; turbo-html2pdf#28).
#![cfg(feature = "bundled-fonts")]

use std::collections::HashMap;
use turbo_dom_layout::test_support::literal::{parse_html, Node};
use turbo_html2pdf_core::layout_snapshot::{
    snapshot_html_live_tree_with_native_text, GeometryKind, LayoutSnapshot, NativeTextOptions,
    SnapshotInputs, SourceId, SourceIdentity,
};
use turbo_html2pdf_core::{build_cascade, Diagnostics, FontRegistry, ImageCtx, NoImages};

fn identities(
    nodes: &[Node],
    path: &mut Vec<usize>,
    out: &mut Vec<SourceIdentity>,
    names: &mut HashMap<String, SourceId>,
) {
    for (index, node) in nodes.iter().enumerate() {
        path.push(index);
        let id = SourceId(out.len() as u64 + 1);
        out.push(SourceIdentity {
            id,
            path: path.clone(),
        });
        if let Node::Element(element) = node {
            for attribute in &element.attrs {
                if attribute.name == "id" {
                    names.insert(attribute.value.clone(), id);
                }
            }
            identities(&element.children, path, out, names);
        }
        path.pop();
    }
}

fn snapshot(html: &str) -> (LayoutSnapshot, HashMap<String, SourceId>) {
    snapshot_nodes(parse_html(html).unwrap())
}
fn snapshot_nodes(nodes: Vec<Node>) -> (LayoutSnapshot, HashMap<String, SourceId>) {
    let mut ids = Vec::new();
    let mut names = HashMap::new();
    identities(&nodes, &mut Vec::new(), &mut ids, &mut names);
    let fonts = FontRegistry::new();
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    let cascade = build_cascade(
        "div{font-family:sans-serif;font-size:16px;line-height:24px;width:160px}",
        "",
        Default::default(),
    );
    let snapshot = snapshot_html_live_tree_with_native_text(
        &turbo_dom_layout::test_support::literal::tree(&nodes).input(),
        &ids,
        &cascade,
        &NativeTextOptions {
            original_text: &[],
            svg_viewports: &[],
            layout_scale: 2.0,
        },
        &SnapshotInputs {
            width: 200.0,
            height: 200.0,
            fonts: &fonts,
            images: &images,
        },
        &mut Diagnostics::default(),
    )
    .unwrap();
    (snapshot, names)
}

#[test]
fn preserved_whitespace_and_inline_directives_keep_real_source_provenance() {
    let (pre, _) = snapshot("<div><span style='white-space:pre'>  </span></div>");
    let text = pre
        .text_sources()
        .iter()
        .find(|source| source.text == "  ")
        .unwrap();
    let selected = pre.text_range_boxes(text.source_id, 0, 2).unwrap();
    assert!(!selected.is_empty());
    assert!(selected.iter().any(|part| part.rect.width > 0.0));
    let mut nodes = parse_html("<div>a<span id=marker></span>b</div>").unwrap();
    let Node::Element(parent) = &mut nodes[0] else {
        unreachable!()
    };
    let Node::Element(marker) = &mut parent.children[1] else {
        unreachable!()
    };
    marker.tag = turbo_html2pdf_core::Tag::Marker(turbo_html2pdf_core::TKind::Page.marker());
    let (snapshot, names) = snapshot_nodes(nodes);
    let source = snapshot.source(names["marker"]).unwrap();
    assert!(!source.excluded_from_layout);
    assert_eq!(source.fragment_indices.len(), 1);
    let fragment = &snapshot.fragments()[source.fragment_indices[0]];
    assert_eq!(fragment.kind, GeometryKind::Marker);
    assert_eq!([fragment.rect.width, fragment.rect.height], [0.0, 0.0]);
    assert_eq!(snapshot.text_sources().len(), 2);
    assert!(snapshot.text_sources().iter().all(|text| !snapshot
        .text_cluster_boxes_for_source(text.source_id)
        .is_empty()));
}
