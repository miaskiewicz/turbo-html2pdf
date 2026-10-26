//! Split at the S1 relocation: the layout-only cases moved to turbo-dom-layout's
//! `tests/svg_viewport_snapshot.rs`; the ones here need TKind directives, templating or
//! the PDF emitter.

use turbo_html2pdf_core::layout_snapshot::{
    snapshot_html_live_tree_with_svg_viewports, LayoutSnapshot, SnapshotInputs, SourceId,
    SourceIdentity,
};
use turbo_html2pdf_core::{
    build_cascade, Diagnostics, Element, FontRegistry, ImageCtx, NoImages, Node, Tag,
};

fn capture(
    nodes: &[Node],
    identities: &[SourceIdentity],
    viewports: &[SourceId],
) -> Result<LayoutSnapshot, turbo_html2pdf_core::layout_snapshot::SvgSnapshotError> {
    capture_at_scale(nodes, identities, viewports, 2.0)
}

fn capture_at_scale(
    nodes: &[Node],
    identities: &[SourceIdentity],
    viewports: &[SourceId],
    scale: f32,
) -> Result<LayoutSnapshot, turbo_html2pdf_core::layout_snapshot::SvgSnapshotError> {
    let fonts = FontRegistry::new();
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    snapshot_html_live_tree_with_svg_viewports(
        nodes,
        identities,
        viewports,
        scale,
        &build_cascade("", "", Default::default()),
        &SnapshotInputs {
            width: 400.0,
            height: 300.0,
            fonts: &fonts,
            images: &images,
        },
        &mut Diagnostics::default(),
    )
}

#[test]
fn text_paths_directives_and_invalid_viewports_cannot_be_declared_svg_sources() {
    use turbo_html2pdf_core::layout_snapshot::{
        SnapshotError, SvgSnapshotError, TextSnapshotError,
    };
    use turbo_html2pdf_core::TKind;
    let text = [Node::Text("not an SVG viewport".into())];
    let ids = [SourceIdentity {
        id: SourceId(2),
        path: vec![0],
    }];
    assert!(matches!(
        capture(&text, &ids, &[SourceId(2)]),
        Err(SvgSnapshotError::InvalidViewportSource(SourceId(2)))
    ));
    let ids = [SourceIdentity {
        id: SourceId(2),
        path: vec![0, 0],
    }];
    assert!(matches!(
        capture(&text, &ids, &[SourceId(2)]),
        Err(SvgSnapshotError::InvalidViewportSource(SourceId(2)))
    ));
    let directives = [Node::Element(Element {
        tag: Tag::Directive(TKind::Page),
        attrs: vec![],
        children: vec![],
    })];
    let ids = [SourceIdentity {
        id: SourceId(2),
        path: vec![0],
    }];
    assert!(matches!(
        capture(&directives, &ids, &[SourceId(2)]),
        Err(SvgSnapshotError::InvalidViewportSource(SourceId(2)))
    ));
    let fonts = FontRegistry::new();
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    let result = snapshot_html_live_tree_with_svg_viewports(
        &text,
        &ids,
        &[],
        2.0,
        &build_cascade("", "", Default::default()),
        &SnapshotInputs {
            width: -1.0,
            height: 300.0,
            fonts: &fonts,
            images: &images,
        },
        &mut Diagnostics::default(),
    );
    assert!(matches!(
        result,
        Err(SvgSnapshotError::Text(TextSnapshotError::Snapshot(
            SnapshotError::InvalidNativeViewport
        )))
    ));
}
