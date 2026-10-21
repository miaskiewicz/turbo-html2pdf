use turbo_html2pdf_core::layout_snapshot::{
    snapshot_html_live_tree_with_svg_viewports, GeometryKind, LayoutSnapshot, SnapshotInputs,
    SourceId, SourceIdentity,
};
use turbo_html2pdf_core::{
    build_cascade, Attr, Diagnostics, Element, FontRegistry, ImageCtx, NoImages, Node, Tag,
};

fn graph(case: &serde_json::Value) -> Vec<Node> {
    let mut attributes: Vec<Attr> = case["attrs"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, value)| Attr {
            name: name.clone(),
            value: value.as_str().unwrap().into(),
        })
        .collect();
    attributes.push(Attr {
        name: "style".into(),
        value: case["style"].as_str().unwrap_or("").into(),
    });
    let circle = Element {
        tag: Tag::Html("circle".into()),
        attrs: vec![
            Attr {
                name: "cx".into(),
                value: "10".into(),
            },
            Attr {
                name: "cy".into(),
                value: "10".into(),
            },
            Attr {
                name: "r".into(),
                value: "5".into(),
            },
        ],
        children: vec![],
    };
    vec![Node::Element(Element {
        tag: Tag::Html("div".into()),
        attrs: vec![Attr {
            name: "style".into(),
            value: case["containerStyle"]
                .as_str()
                .unwrap_or("width:400px;height:200px;font-size:0;line-height:0")
                .into(),
        }],
        children: vec![Node::Element(Element {
            tag: Tag::Html("svg".into()),
            attrs: attributes,
            children: vec![Node::Element(circle)],
        })],
    })]
}

fn identities() -> [SourceIdentity; 3] {
    [
        SourceIdentity {
            id: SourceId(1),
            path: vec![0],
        },
        SourceIdentity {
            id: SourceId(2),
            path: vec![0, 0],
        },
        SourceIdentity {
            id: SourceId(3),
            path: vec![0, 0, 0],
        },
    ]
}

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
fn native_outer_svg_viewport_boxes_match_independent_chrome_without_shape_geometry() {
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/svg-viewport-chrome155.json")).unwrap();
    assert_eq!(
        oracle["result"]["layoutScaleCalibration"]["matchingScales"],
        serde_json::json!([2])
    );
    for case in oracle["result"]["cases"].as_array().unwrap() {
        let nodes = graph(case);
        let native = capture(&nodes, &identities(), &[SourceId(2)]).unwrap();
        assert_eq!(native.sources().len(), 3);
        assert!(native.unsupported_svg_source_geometry(SourceId(3)));
        assert!(!native.unsupported_svg_source_geometry(SourceId(2)));
        assert_eq!(native.source(SourceId(3)).unwrap().path, vec![0, 0, 0]);
        assert_eq!(
            native.source(SourceId(3)).unwrap().fragment_indices,
            Vec::<usize>::new()
        );
        assert_viewport(&native, case);
        assert_source_box(
            &native,
            SourceId(1),
            &case["container"],
            case["name"].as_str().unwrap(),
        );
        assert!(!native
            .fragments()
            .iter()
            .any(|fragment| fragment.kind == GeometryKind::ImagePaint));
    }
}

fn assert_viewport(native: &LayoutSnapshot, case: &serde_json::Value) {
    let source = native.source(SourceId(2)).unwrap();
    if case["name"] == "hidden" {
        assert!(source.excluded_from_layout);
        assert_eq!(source.fragment_indices, Vec::<usize>::new());
        return;
    }
    assert_source_box(
        native,
        SourceId(2),
        &case["viewport"],
        case["name"].as_str().unwrap(),
    );
}

fn assert_source_box(
    native: &LayoutSnapshot,
    id: SourceId,
    expected_box: &serde_json::Value,
    name: &str,
) {
    let source = native.source(id).unwrap();
    let boxes: Vec<_> = source
        .fragment_indices
        .iter()
        .map(|index| &native.fragments()[*index])
        .filter(|fragment| fragment.kind == GeometryKind::BorderBox)
        .collect();
    assert_eq!(boxes.len(), 1, "{name}");
    let used = boxes[0].used_box.unwrap();
    let client_width = (used.content_width + used.padding.horizontal()).round();
    let client_height = (used.content_height + used.padding.vertical()).round();
    assert_eq!(
        f64::from(client_width),
        expected_box["client"]["width"].as_f64().unwrap(),
        "{name} client width"
    );
    assert_eq!(
        f64::from(client_height),
        expected_box["client"]["height"].as_f64().unwrap(),
        "{name} client height"
    );
    let rect = boxes[0].transformed_bounds;
    let actual = [rect.x, rect.y, rect.width, rect.height];
    let expected = &expected_box["rect"];
    for (index, key) in ["x", "y", "width", "height"].iter().enumerate() {
        let difference = (f64::from(actual[index]) - expected[key].as_f64().unwrap()).abs();
        assert!(
            difference < 0.001,
            "{name} {key}: native {actual:?}, Chrome {expected}"
        );
    }
}

#[test]
fn viewport_declaration_errors_do_not_erase_or_reinterpret_source_identities() {
    use turbo_html2pdf_core::layout_snapshot::SvgSnapshotError;
    let case = serde_json::json!({"attrs":{"width":"40","height":"20"}});
    let nodes = graph(&case);
    assert!(matches!(
        capture(&nodes, &identities(), &[SourceId(999)]),
        Err(SvgSnapshotError::InvalidViewportSource(SourceId(999)))
    ));
    assert!(matches!(
        capture(&nodes, &identities(), &[SourceId(1)]),
        Err(SvgSnapshotError::InvalidViewportSource(SourceId(1)))
    ));
    assert!(matches!(
        capture(&nodes, &identities(), &[SourceId(2), SourceId(2)]),
        Err(SvgSnapshotError::DuplicateViewport(SourceId(2)))
    ));
    let native = capture(&nodes, &identities(), &[SourceId(2)]).unwrap();
    assert_eq!(native.source(SourceId(3)).unwrap().path, vec![0, 0, 0]);
    assert!(native.unsupported_svg_source_geometry(SourceId(3)));
}

#[test]
fn nested_svg_declarations_fail_and_undeclared_nested_sources_remain_explicit() {
    use turbo_html2pdf_core::layout_snapshot::SvgSnapshotError;
    let case = serde_json::json!({"attrs":{"width":"40","height":"20"}});
    let mut nodes = graph(&case);
    let Node::Element(root) = &mut nodes[0] else {
        unreachable!()
    };
    let Node::Element(outer) = &mut root.children[0] else {
        unreachable!()
    };
    let Node::Element(inner) = &mut outer.children[0] else {
        unreachable!()
    };
    inner.tag = Tag::Html("svg".into());
    assert!(matches!(
        capture(&nodes, &identities(), &[SourceId(2), SourceId(3)]),
        Err(SvgSnapshotError::NestedViewport(SourceId(3)))
    ));
    let native = capture(&nodes, &identities(), &[SourceId(2)]).unwrap();
    assert_eq!(native.source(SourceId(3)).unwrap().path, vec![0, 0, 0]);
    assert_eq!(
        native.source(SourceId(3)).unwrap().fragment_indices,
        Vec::<usize>::new()
    );
    assert!(native.unsupported_svg_source_geometry(SourceId(3)));
}

#[test]
fn native_viewport_layout_scale_is_bounded_and_configurable_before_box_allocation() {
    use turbo_html2pdf_core::layout_snapshot::SvgSnapshotError;
    let case = serde_json::json!({"attrs":{"width":"32.51","height":"24.29"}});
    let nodes = graph(&case);
    for scale in [0.0, -1.0, 17.0, f32::NAN, f32::INFINITY] {
        assert!(matches!(
            capture_at_scale(&nodes, &identities(), &[SourceId(2)], scale),
            Err(SvgSnapshotError::InvalidLayoutScale)
        ));
    }
    let native = capture_at_scale(&nodes, &identities(), &[SourceId(2)], 1.0).unwrap();
    let source = native.source(SourceId(2)).unwrap();
    let actual = &native.fragments()[source.fragment_indices[0]];
    assert_eq!((actual.rect.width, actual.rect.height), (32.5, 24.28125));
    let native = capture_at_scale(&nodes, &identities(), &[SourceId(2)], 2.0).unwrap();
    let source = native.source(SourceId(2)).unwrap();
    let actual = &native.fragments()[source.fragment_indices[0]];
    assert_eq!(
        (actual.rect.width, actual.rect.height),
        (4161.0 / 128.0, 3109.0 / 128.0)
    );
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
