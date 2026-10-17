use turbo_html2pdf_core::layout_snapshot::{
    snapshot_live_tree, snapshot_styled_tree, GeometryKind, LayoutSnapshot, Rect, SnapshotError,
    SnapshotInputs, SourceId, SourceIdentity, SourceKind,
};
use turbo_html2pdf_core::{
    build_cascade, layout_with_images, style_tree, Attr, Diagnostics, Element, FontRegistry,
    ImageCtx, ImageResolver, NoImages, Node, Tag,
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
        nodes,
        ids,
        &build_cascade("", "", Default::default()),
        &inputs,
        &mut Diagnostics::default(),
    )
    .unwrap()
}

fn border(
    snapshot: &LayoutSnapshot,
    id: u64,
) -> &turbo_html2pdf_core::layout_snapshot::SnapshotFragment {
    let source = snapshot.source(SourceId(id)).unwrap();
    source
        .fragment_indices
        .iter()
        .map(|&i| &snapshot.fragments()[i])
        .find(|f| f.kind == GeometryKind::BorderBox)
        .unwrap()
}

#[test]
fn source_identity_survives_moves_without_guest_attributes_or_tree_rewrite() {
    let first = element("div", "width:40px;height:20px", Vec::new());
    let second = element("div", "width:60px;height:30px", Vec::new());
    let before_nodes = vec![first.clone(), second.clone()];
    let before = capture(&before_nodes, &[identity(41, &[0]), identity(99, &[1])]);
    let after_nodes = vec![second, first];
    let after = capture(&after_nodes, &[identity(99, &[0]), identity(41, &[1])]);
    assert_eq!(
        border(&before, 41).rect.width,
        border(&after, 41).rect.width
    );
    assert_eq!(
        border(&before, 99).rect.width,
        border(&after, 99).rect.width
    );
    assert_eq!(border(&after, 41).rect.y, 30.0);
    assert_eq!(after.source(SourceId(41)).unwrap().path, vec![1]);
    assert_eq!(before_nodes[0].as_element().unwrap().attrs.len(), 1);
    assert!(after.source(SourceId(777)).is_none());
    assert_eq!(after.viewport(), (200.0, 200.0));
}

#[test]
fn hidden_and_clipped_boxes_keep_geometry_but_display_none_has_no_fragments() {
    let nodes = vec![
        element("div", "height:10px;visibility:hidden", Vec::new()),
        element("div", "height:20px;opacity:0", Vec::new()),
        element(
            "div",
            "width:1px;height:1px;overflow:hidden;clip:rect(0,0,0,0)",
            Vec::new(),
        ),
        element("div", "height:90px;display:none", Vec::new()),
        element("div", "height:7px", Vec::new()),
    ];
    let ids: Vec<_> = (0..5).map(|i| identity(100 + i as u64, &[i])).collect();
    let snapshot = capture(&nodes, &ids);
    assert_eq!(border(&snapshot, 100).rect.height, 10.0);
    assert_eq!(border(&snapshot, 101).rect.y, 10.0);
    assert_eq!(border(&snapshot, 102).rect.y, 30.0);
    assert_eq!(border(&snapshot, 104).rect.y, 31.0);
    assert!(snapshot
        .source(SourceId(103))
        .unwrap()
        .fragment_indices
        .is_empty());
    // Legacy PDF rendering deliberately retains its previous nonpainting filter.
    let fonts = FontRegistry::new();
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    let styled = style_tree(&nodes, &build_cascade("", "", Default::default()));
    let legacy = layout_with_images(&styled, 200.0, &fonts, &images, &mut Diagnostics::default());
    assert_eq!(legacy.height, 7.0);
}

#[test]
fn ancestor_and_child_transforms_compose_about_actual_box_origins() {
    let child = element(
        "div",
        "width:20px;height:10px;transform:rotate(90deg);transform-origin:0 0",
        Vec::new(),
    );
    let nodes = vec![element("div", "width:100px;height:60px;padding:10px;transform:translate(30px,40px) scale(2);transform-origin:0 0", vec![child])];
    let snapshot = capture(&nodes, &[identity(1, &[0]), identity(2, &[0, 0])]);
    let parent = border(&snapshot, 1);
    let child = border(&snapshot, 2);
    assert_eq!(
        parent.transformed_bounds,
        Rect {
            x: 30.0,
            y: 40.0,
            width: 240.0,
            height: 160.0
        }
    );
    assert!((child.transformed_bounds.x - 30.0).abs() < 0.001);
    assert!((child.transformed_bounds.y - 60.0).abs() < 0.001);
    assert!((child.transformed_bounds.width - 20.0).abs() < 0.001);
    assert!((child.transformed_bounds.height - 40.0).abs() < 0.001);
    assert_eq!(child.source_ancestors, vec![SourceId(1)]);
    assert_eq!(
        snapshot.fragments()[child.parent.unwrap()].source_id,
        Some(SourceId(1))
    );
}

#[test]
fn all_text_lines_keep_source_and_flattened_inline_ancestry_without_fake_range_rects() {
    let text = Node::Text("alpha beta gamma delta epsilon zeta eta theta".into());
    let span = element("span", "font-size:16px;line-height:20px", vec![text]);
    let nodes = vec![element("div", "width:65px", vec![span])];
    let snapshot = capture(
        &nodes,
        &[
            identity(1, &[0]),
            identity(2, &[0, 0]),
            identity(3, &[0, 0, 0]),
        ],
    );
    let inline = snapshot.source(SourceId(2)).unwrap();
    let text = snapshot.source(SourceId(3)).unwrap();
    assert_eq!(inline.kind, SourceKind::Element);
    assert_eq!(text.kind, SourceKind::Text);
    assert!(inline.fragment_indices.is_empty());
    assert!(text.fragment_indices.len() > 2);
    assert_eq!(inline.descendant_fragment_indices, text.fragment_indices);
    for &index in &text.fragment_indices {
        let fragment = &snapshot.fragments()[index];
        assert_eq!(fragment.kind, GeometryKind::TextLineContainer);
        assert_eq!(fragment.source_ancestors, vec![SourceId(1), SourceId(2)]);
        assert!(fragment.rect.height > 0.0);
    }
    fn count(fragment: &turbo_html2pdf_core::Fragment) -> usize {
        1 + fragment.children.iter().map(count).sum::<usize>()
    }
    assert_eq!(count(snapshot.galley()), snapshot.fragments().len());
}

#[test]
fn generated_input_label_has_no_fabricated_text_source_but_retains_owner() {
    let mut input = element("input", "width:80px;height:24px", Vec::new());
    let Node::Element(ref mut input_element) = input else {
        unreachable!()
    };
    input_element.attrs.extend([
        Attr {
            name: "type".into(),
            value: "button".into(),
        },
        Attr {
            name: "value".into(),
            value: "Run".into(),
        },
    ]);
    let snapshot = capture(&[input], &[identity(7, &[0])]);
    let text = snapshot
        .fragments()
        .iter()
        .find(|f| f.kind == GeometryKind::TextLineContainer)
        .unwrap();
    assert_eq!(text.source_id, None);
    assert_eq!(text.source_ancestors, vec![SourceId(7)]);
    assert!(snapshot
        .source(SourceId(7))
        .unwrap()
        .descendant_fragment_indices
        .contains(&text.index));
}

#[test]
fn anonymous_wrappers_never_receive_the_id_of_a_neighboring_source() {
    let nodes = vec![element(
        "div",
        "",
        vec![
            Node::Text("lead".into()),
            element("div", "height:12px", Vec::new()),
            Node::Text("tail".into()),
        ],
    )];
    let snapshot = capture(
        &nodes,
        &[
            identity(1, &[0]),
            identity(2, &[0, 0]),
            identity(3, &[0, 1]),
            identity(4, &[0, 2]),
        ],
    );
    let anonymous: Vec<_> = snapshot
        .fragments()
        .iter()
        .filter(|f| f.source_id.is_none())
        .collect();
    assert!(anonymous.len() >= 3);
    assert_eq!(
        snapshot.source(SourceId(2)).unwrap().fragment_indices.len(),
        1
    );
    assert_eq!(
        snapshot.source(SourceId(4)).unwrap().fragment_indices.len(),
        1
    );
    assert!(anonymous
        .iter()
        .skip(1)
        .all(|f| f.source_ancestors == vec![SourceId(1)]));
}

#[test]
fn viewport_and_definite_ancestor_heights_resolve_real_percentage_boxes() {
    let nodes = vec![element(
        "div",
        "height:50%;width:50vw",
        vec![element("div", "height:50%;width:50%", Vec::new())],
    )];
    let snapshot = capture(&nodes, &[identity(1, &[0]), identity(2, &[0, 0])]);
    assert_eq!(
        border(&snapshot, 1).rect,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0
        }
    );
    assert_eq!(
        border(&snapshot, 2).rect,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 50.0
        }
    );
}

#[test]
fn source_validation_rejects_duplicates_outside_paths_and_bad_viewports() {
    let nodes = vec![element("div", "", Vec::new())];
    let styled = style_tree(&nodes, &build_cascade("", "", Default::default()));
    let fonts = FontRegistry::new();
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    let mut inputs = SnapshotInputs {
        width: 200.0,
        height: 100.0,
        fonts: &fonts,
        images: &images,
    };
    let error = |ids: &[SourceIdentity], inputs: &SnapshotInputs| {
        snapshot_styled_tree(&styled, ids, inputs, &mut Diagnostics::default())
            .err()
            .unwrap()
    };
    assert_eq!(
        error(&[identity(1, &[0]), identity(1, &[1])], &inputs),
        SnapshotError::DuplicateId(SourceId(1))
    );
    assert_eq!(
        error(&[identity(1, &[0]), identity(2, &[0])], &inputs),
        SnapshotError::DuplicatePath(vec![0])
    );
    assert_eq!(
        error(&[identity(1, &[])], &inputs),
        SnapshotError::MissingPath(Vec::new())
    );
    assert_eq!(
        error(&[identity(1, &[0, 0])], &inputs),
        SnapshotError::MissingPath(vec![0, 0])
    );
    for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        inputs.width = invalid;
        assert_eq!(error(&[], &inputs), SnapshotError::InvalidViewport);
    }
    inputs.width = 200.0;
    inputs.height = 0.0;
    assert_eq!(error(&[], &inputs), SnapshotError::InvalidViewport);
}

struct Images(Vec<u8>);
impl ImageResolver for Images {
    fn resolve(&self, name: &str) -> Option<&[u8]> {
        (name == "fixture").then_some(self.0.as_slice())
    }
}

fn image_bytes() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 4, 2);
        encoder.set_color(png::ColorType::Rgb);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255; 24]).unwrap();
    }
    out
}

fn image_node(css: &str) -> Node {
    let mut node = element("img", css, Vec::new());
    let Node::Element(ref mut image_element) = node else {
        unreachable!()
    };
    image_element.attrs.push(Attr {
        name: "src".into(),
        value: "fixture".into(),
    });
    node
}

fn image_snapshot(css: &str) -> LayoutSnapshot {
    let nodes = vec![image_node(css)];
    let resolver = Images(image_bytes());
    let images = ImageCtx {
        resolver: &resolver,
        body_height: Some(200.0),
    };
    let fonts = FontRegistry::new();
    let inputs = SnapshotInputs {
        width: 200.0,
        height: 200.0,
        fonts: &fonts,
        images: &images,
    };
    snapshot_live_tree(
        &nodes,
        &[identity(9, &[0])],
        &build_cascade("", "", Default::default()),
        &inputs,
        &mut Diagnostics::default(),
    )
    .unwrap()
}

#[test]
fn replaced_boxes_keep_overflow_css_frames_transforms_and_all_image_fragments() {
    let snapshot = image_snapshot("display:block;width:400px;height:240px;padding:5px;border:2px solid black;transform:translate(10px,20px)");
    let source = snapshot.source(SourceId(9)).unwrap();
    assert_eq!(source.fragment_indices.len(), 2);
    let outer = border(&snapshot, 9);
    assert_eq!(outer.rect.width, 414.0);
    assert_eq!(outer.rect.height, 254.0);
    assert_eq!(outer.transformed_bounds.x, 10.0);
    let image = &snapshot.fragments()[source.fragment_indices[1]];
    assert_eq!(image.kind, GeometryKind::ImagePaint);
    assert_eq!(
        image.rect,
        Rect {
            x: 7.0,
            y: 7.0,
            width: 400.0,
            height: 240.0
        }
    );
    assert_eq!(image.transformed_bounds.x, 17.0);
    assert_eq!(image.transformed_bounds.y, 27.0);
    assert_eq!(image.source_id, outer.source_id);
    assert_eq!(image.parent, Some(outer.index));
    // The existing PDF API still caps this image to the page.
    let styled = style_tree(
        &[image_node("width:400px;height:240px")],
        &build_cascade("", "", Default::default()),
    );
    let resolver = Images(image_bytes());
    let images = ImageCtx {
        resolver: &resolver,
        body_height: Some(200.0),
    };
    let legacy = layout_with_images(
        &styled,
        200.0,
        &FontRegistry::new(),
        &images,
        &mut Diagnostics::default(),
    );
    assert_eq!(legacy.children[0].width, 200.0);
    assert_eq!(legacy.children[0].height, 120.0);
}

#[test]
fn replaced_constraints_honor_box_sizing_ratio_and_min_over_max() {
    let cases = [
        ("width:100px;max-width:60px", 60.0, 30.0),
        ("height:50px;max-height:30px", 60.0, 30.0),
        (
            "width:100px;height:50px;box-sizing:border-box;padding:5px;border:2px solid black",
            100.0,
            50.0,
        ),
        ("min-width:20px;max-width:10px", 20.0, 10.0),
        ("min-height:20px;max-height:10px", 40.0, 20.0),
        (
            "width:100px;height:50px;min-width:120px;max-width:80px",
            120.0,
            50.0,
        ),
    ];
    for (css, w, h) in cases {
        let snapshot = image_snapshot(css);
        assert_eq!(border(&snapshot, 9).rect.width, w, "{css}");
        assert_eq!(border(&snapshot, 9).rect.height, h, "{css}");
    }
}

#[test]
fn partial_source_maps_do_not_invent_missing_identities_and_empty_tree_is_valid() {
    let nodes = vec![element(
        "div",
        "height:10px",
        vec![Node::Text("text".into())],
    )];
    let snapshot = capture(&nodes, &[identity(3, &[0, 0])]);
    assert_eq!(snapshot.sources().len(), 1);
    assert_eq!(snapshot.source(SourceId(3)).unwrap().kind, SourceKind::Text);
    assert!(snapshot
        .fragments()
        .iter()
        .all(|f| f.source_ancestors.is_empty()));
    let empty = capture(&[], &[]);
    assert_eq!(empty.fragments().len(), 1);
    assert!(empty.sources().is_empty());
    assert!(empty.galley().children.is_empty());
}

fn fixture_node(
    value: &serde_json::Value,
    path: &[usize],
    identities: &mut Vec<SourceIdentity>,
) -> Node {
    identities.push(identity(value["sourceId"].as_u64().unwrap(), path));
    if let Some(text) = value["text"].as_str() {
        return Node::Text(text.into());
    }
    let children = value["children"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, child)| {
            let mut child_path = path.to_vec();
            child_path.push(i);
            fixture_node(child, &child_path, identities)
        })
        .collect();
    let tag = value["tag"].as_str().unwrap();
    let mut node = element(tag, value["style"].as_str().unwrap(), children);
    if tag == "img" {
        let Node::Element(ref mut element) = node else {
            unreachable!()
        };
        element.attrs.push(Attr {
            name: "src".into(),
            value: "fixture".into(),
        });
    }
    node
}

fn assert_chrome_rect(actual: Rect, expected: &serde_json::Value, context: &str) {
    for (field, value) in [
        ("x", actual.x),
        ("y", actual.y),
        ("width", actual.width),
        ("height", actual.height),
    ] {
        let difference = (f64::from(value) - expected[field].as_f64().unwrap()).abs();
        assert!(
            difference <= 0.001,
            "{context} {field}: native={value} Chrome={}",
            expected[field]
        );
    }
}

#[test]
fn authored_live_boxes_match_independently_recorded_chrome155_geometry() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/layout-snapshot/cases.json")).unwrap();
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/layout-snapshot/chrome155.json")).unwrap();
    compare_authored_cases(&fixture, &oracle);
}

#[test]
fn contents_boxes_match_independently_recorded_chrome155_geometry() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/layout-snapshot/contents-cases.json")).unwrap();
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/layout-snapshot/contents-chrome155.json"
    ))
    .unwrap();
    compare_authored_cases(&fixture, &oracle);
}

#[test]
fn anonymous_contents_text_matches_independent_chrome155_geometry() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/layout-snapshot/contents-text-cases.json"
    ))
    .unwrap();
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/layout-snapshot/contents-text-chrome155.json"
    ))
    .unwrap();
    compare_authored_cases(&fixture, &oracle);
}

fn compare_authored_cases(fixture: &serde_json::Value, oracle: &serde_json::Value) {
    let mut fonts = FontRegistry::new();
    if fixture["font"] == "Go" {
        fonts.add(
            turbo_html2pdf_core::FontFace::from_bytes(
                include_bytes!("../assets/fonts/Go-Regular.ttf").to_vec(),
                "ParityGo",
                400,
                false,
            )
            .unwrap(),
        );
    }
    let resolver = Images(image_bytes());
    let images = ImageCtx {
        resolver: &resolver,
        body_height: Some(200.0),
    };
    let inputs = SnapshotInputs {
        width: 200.0,
        height: 200.0,
        fonts: &fonts,
        images: &images,
    };
    assert_eq!(
        fixture["cases"].as_array().unwrap().len(),
        oracle["cases"].as_array().unwrap().len()
    );
    for (test, chrome) in fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["cases"].as_array().unwrap())
    {
        assert_eq!(test["name"], chrome["name"]);
        let mut identities = Vec::new();
        let nodes: Vec<_> = test["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, n)| fixture_node(n, &[i], &mut identities))
            .collect();
        if fixture["font"] == "Go" {
            let snapshot =
                turbo_html2pdf_core::layout_snapshot::snapshot_html_live_tree_with_svg_viewports(
                    &nodes,
                    &identities,
                    &[],
                    2.0,
                    &build_cascade("", "", Default::default()),
                    &inputs,
                    &mut Diagnostics::default(),
                )
                .unwrap();
            compare_chrome_boxes(&snapshot, chrome);
            continue;
        }
        let result = snapshot_live_tree(
            &nodes,
            &identities,
            &build_cascade("", "", Default::default()),
            &inputs,
            &mut Diagnostics::default(),
        );
        if test["nativeAdmission"] == "unsupported-fixed-containing-block" {
            assert!(matches!(
                result,
                Err(SnapshotError::UnsupportedFixedContainingBlock)
            ));
        } else {
            compare_chrome_boxes(&result.unwrap(), chrome);
        }
    }
}

fn compare_chrome_boxes(snapshot: &LayoutSnapshot, chrome: &serde_json::Value) {
    for expected in chrome["boxes"].as_array().unwrap() {
        let id = expected["sourceId"].as_u64().unwrap();
        let source = snapshot.source(SourceId(id)).unwrap();
        if expected["clientRects"].as_array().unwrap().is_empty() {
            assert!(source.fragment_indices.is_empty());
        } else {
            let actual = border(snapshot, id);
            assert_chrome_rect(
                actual.transformed_bounds,
                &expected["rect"],
                chrome["name"].as_str().unwrap(),
            );
            compare_chrome_metrics(actual, expected, chrome["name"].as_str().unwrap());
        }
    }
}

#[test]
fn live_anonymous_inline_boxes_do_not_repeat_parent_frame_or_transform() {
    let inline = element(
        "span",
        "display:inline-block;width:10px;height:10px;vertical-align:top",
        Vec::new(),
    );
    let child = element("div", "height:12px", Vec::new());
    let nodes = vec![element(
        "div",
        "padding:5px;border:2px solid black;line-height:20px;transform:translate(30px,40px)",
        vec![inline.clone(), child, inline],
    )];
    let snapshot = capture(
        &nodes,
        &[
            identity(1, &[0]),
            identity(2, &[0, 0]),
            identity(3, &[0, 1]),
            identity(4, &[0, 2]),
        ],
    );
    let first = border(&snapshot, 2);
    let block = border(&snapshot, 3);
    let last = border(&snapshot, 4);
    assert_eq!(first.rect.x, 7.0);
    assert_eq!(first.transformed_bounds.x, 37.0);
    assert_eq!(first.transformed_bounds.y, 47.0);
    assert_eq!(block.rect.y, 27.0);
    assert_eq!(last.rect.y, 39.0);
    assert_eq!(last.transformed_bounds.y, 79.0);
    assert_eq!(border(&snapshot, 1).rect.height, 66.0);
}

#[test]
fn used_box_metrics_are_captured_before_transforms_and_follow_final_translations() {
    let nodes = vec![element("div","position:relative;left:12px;top:8px;width:80px;height:40px;padding:5px;border:2px solid black;transform:scale(2)",Vec::new())];
    let snapshot = capture(&nodes, &[identity(1, &[0])]);
    let fragment = border(&snapshot, 1);
    let metrics = fragment.used_box.unwrap();
    assert_eq!(
        fragment.rect,
        Rect {
            x: 12.0,
            y: 8.0,
            width: 94.0,
            height: 54.0
        }
    );
    assert_eq!(
        metrics.content_rect(fragment.rect),
        Rect {
            x: 19.0,
            y: 15.0,
            width: 80.0,
            height: 40.0
        }
    );
    assert_eq!(
        metrics.padding_rect(fragment.rect),
        Rect {
            x: 14.0,
            y: 10.0,
            width: 90.0,
            height: 50.0
        }
    );
    assert_eq!(
        metrics.position,
        turbo_html2pdf_core::layout::value::Position::Relative
    );
    assert_eq!(metrics.border.left, 2.0);
    assert_eq!(fragment.transformed_bounds.width, 188.0);
    let image =
        image_snapshot("display:block;width:80px;height:40px;padding:5px;border:2px solid black");
    let source = image.source(SourceId(9)).unwrap();
    let outer = border(&image, 9);
    assert_eq!(
        outer.used_box.unwrap().content_rect(outer.rect),
        Rect {
            x: 7.0,
            y: 7.0,
            width: 80.0,
            height: 40.0
        }
    );
    assert!(image.fragments()[source.fragment_indices[1]]
        .used_box
        .is_none());
}

fn compare_chrome_metrics(
    fragment: &turbo_html2pdf_core::layout_snapshot::SnapshotFragment,
    expected: &serde_json::Value,
    context: &str,
) {
    let metrics = fragment.used_box.unwrap();
    let padding = metrics.padding_rect(fragment.rect);
    // CSSOM View gives the document root viewport client dimensions; this API
    // retains its actual native padding box, so compare its box/offset geometry.
    if expected["isDocumentRoot"] == true {
        assert_eq!(
            f64::from(fragment.rect.width.round()),
            expected["offset"]["width"].as_f64().unwrap()
        );
        assert_eq!(
            f64::from(fragment.rect.height.round()),
            expected["offset"]["height"].as_f64().unwrap()
        );
        return;
    }
    for (field, value) in [
        ("left", metrics.border.left),
        ("top", metrics.border.top),
        ("width", padding.width),
        ("height", padding.height),
    ] {
        assert_eq!(
            f64::from(value.round()),
            expected["client"][field].as_f64().unwrap(),
            "{context} client{field}"
        );
    }
    assert_eq!(
        f64::from(fragment.rect.width.round()),
        expected["offset"]["width"].as_f64().unwrap(),
        "{context} offsetWidth"
    );
    assert_eq!(
        f64::from(fragment.rect.height.round()),
        expected["offset"]["height"].as_f64().unwrap(),
        "{context} offsetHeight"
    );
}

#[test]
fn directive_fragments_keep_actual_source_identity_and_category() {
    let node = Node::Element(Element {
        tag: Tag::Directive(turbo_html2pdf_core::TKind::Page),
        attrs: Vec::new(),
        children: Vec::new(),
    });
    let snapshot = capture(&[node], &[identity(51, &[0])]);
    let source = snapshot.source(SourceId(51)).unwrap();
    assert_eq!(source.fragment_indices.len(), 1);
    let fragment = &snapshot.fragments()[source.fragment_indices[0]];
    assert_eq!(fragment.kind, GeometryKind::Directive);
    assert!(fragment.used_box.is_none());
    assert_eq!(fragment.source_id, Some(SourceId(51)));
}

#[test]
fn zero_fragment_provenance_distinguishes_excluded_subtrees_from_empty_visible_inlines() {
    let nodes = vec![
        element(
            "div",
            "display:none",
            vec![element("span", "", vec![Node::Text("hidden".into())])],
        ),
        element("span", "", Vec::new()),
    ];
    // The hiding ancestor is intentionally unmapped: its policy must still reach
    // the caller's mapped descendants without manufacturing a visible rectangle.
    let snapshot = capture(
        &nodes,
        &[
            identity(1, &[0, 0]),
            identity(2, &[0, 0, 0]),
            identity(3, &[1]),
        ],
    );
    for id in [1, 2] {
        let source = snapshot.source(SourceId(id)).unwrap();
        assert!(source.excluded_from_layout);
        assert!(source.fragment_indices.is_empty());
    }
    let empty = snapshot.source(SourceId(3)).unwrap();
    assert!(!empty.excluded_from_layout);
    assert!(empty.fragment_indices.is_empty());
}

#[test]
fn hidden_directives_and_their_descendants_have_no_live_geometry() {
    let node = Node::Element(Element {
        tag: Tag::Directive(turbo_html2pdf_core::TKind::Page),
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
    let styled = style_tree(&[node], &build_cascade("", "", Default::default()));
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

#[test]
fn transformed_fixed_admission_is_identity_independent_and_shared_by_both_entry_points() {
    let nodes = vec![element(
        "div",
        "transform:translateX(0px)",
        vec![element(
            "div",
            "",
            vec![element("div", "position:fixed;height:50%", Vec::new())],
        )],
    )];
    let fonts = FontRegistry::new();
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    let inputs = SnapshotInputs {
        width: 200.0,
        height: 200.0,
        fonts: &fonts,
        images: &images,
    };
    let cascade = build_cascade("", "", Default::default());
    let result = snapshot_live_tree(&nodes, &[], &cascade, &inputs, &mut Diagnostics::default());
    assert!(matches!(
        result,
        Err(SnapshotError::UnsupportedFixedContainingBlock)
    ));
    let styled = style_tree(&nodes, &cascade);
    let result = snapshot_styled_tree(&styled, &[], &inputs, &mut Diagnostics::default());
    assert!(matches!(
        result,
        Err(SnapshotError::UnsupportedFixedContainingBlock)
    ));
    let hidden = vec![element(
        "div",
        "display:none;transform:translateX(0px)",
        vec![element("div", "position:fixed", Vec::new())],
    )];
    assert!(capture(&hidden, &[])
        .fragments()
        .iter()
        .all(|f| f.source_id.is_none()));
    let text = vec![Node::Text("test".into())];
    assert!(!capture(&text, &[]).fragments().is_empty());
}

#[test]
fn positioned_shorthands_keep_cascade_order_and_importance() {
    for (css, x) in [
        ("left:20px;inset:0", 2.0),
        ("inset:0;left:20px", 22.0),
        ("left:20px!important;inset:0", 22.0),
        ("inset:0!important;left:20px", 2.0),
    ] {
        let nodes = vec![element(
            "div",
            "position:relative;width:100px;height:80px;padding:5px;border:2px solid black",
            vec![element(
                "div",
                &format!("position:absolute;width:10px;height:10px;{css}"),
                Vec::new(),
            )],
        )];
        assert_eq!(
            border(&capture(&nodes, &[identity(1, &[0, 0])]), 1).rect.x,
            x,
            "{css}"
        );
    }
}

#[test]
fn invalid_quad_declarations_preserve_prior_valid_sides_and_inherited_variables_expand() {
    for (css, left, top) in [
        ("left:7px;top:8px;inset:10px bogus", 7.0, 8.0),
        ("left:7px;top:8px;inset:initial auto", 7.0, 8.0),
        ("left:20px;inset:var(--edges)", 7.0, 3.0),
        ("inset:var(--edges);left:20px", 20.0, 3.0),
        ("left:20px!important;inset:var(--edges)", 20.0, 3.0),
        ("left:20px;inset:var(--edges)!important", 7.0, 3.0),
        ("left:20px;inset:var(--missing)", 0.0, 0.0),
        ("--edges:4px 9px;inset:var(--edges)", 9.0, 4.0),
        ("--edges:inherit;inset:var(--edges)", 7.0, 3.0),
    ] {
        let nodes = vec![element(
            "div",
            "position:relative;width:100px;height:80px;--edges:3px 7px",
            vec![element(
                "div",
                &format!("position:absolute;width:10px;height:10px;{css}"),
                Vec::new(),
            )],
        )];
        let snapshot = capture(&nodes, &[identity(1, &[0, 0])]);
        assert_eq!(border(&snapshot, 1).rect.x, left, "{css}");
        assert_eq!(border(&snapshot, 1).rect.y, top, "{css}");
    }
    let nodes = vec![element(
        "div",
        "border-left-width:7px;border-width:3px red",
        Vec::new(),
    )];
    let styled = style_tree(&nodes, &build_cascade("", "", Default::default()));
    let turbo_html2pdf_core::StyledNode::Element(element) = &styled[0] else {
        panic!("element");
    };
    assert_eq!(element.style.get("border-left-width"), Some("7px"));
}

#[test]
fn live_document_root_contains_body_margins_and_preserves_legacy_pdf_policy() {
    let nodes = vec![element(
        "html",
        "margin:0;padding:0",
        vec![element(
            "body",
            "margin:12px;padding:5px;border:3px solid black",
            Vec::new(),
        )],
    )];
    let snapshot = capture(&nodes, &[identity(1, &[0]), identity(2, &[0, 0])]);
    assert_eq!(
        border(&snapshot, 1).rect,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 40.0
        }
    );
    assert_eq!(
        border(&snapshot, 2).rect,
        Rect {
            x: 12.0,
            y: 12.0,
            width: 176.0,
            height: 16.0
        }
    );
    let styled = style_tree(&nodes, &build_cascade("", "", Default::default()));
    let legacy = layout_with_images(
        &styled,
        200.0,
        &FontRegistry::new(),
        &ImageCtx {
            resolver: &NoImages,
            body_height: None,
        },
        &mut Diagnostics::default(),
    );
    assert_eq!(legacy.children[0].y, 12.0);
    assert_eq!(legacy.children[0].height, 16.0);
}

#[test]
fn double_precision_used_transforms_follow_real_image_fragments_once_and_keep_legacy_matrix_type() {
    let native = image_snapshot(
        "display:block;width:80px;height:40px;transform:rotate(10deg);transform-origin:0 0",
    );
    let source = native.source(SourceId(9)).unwrap();
    let border = source
        .fragment_indices
        .iter()
        .map(|&i| &native.fragments()[i])
        .find(|fragment| fragment.kind == GeometryKind::BorderBox)
        .unwrap();
    let paint = source
        .fragment_indices
        .iter()
        .map(|&i| &native.fragments()[i])
        .find(|fragment| fragment.kind == GeometryKind::ImagePaint)
        .unwrap();
    let matrix = native.native_world_transform(border.index).unwrap();
    assert_eq!(matrix, native.native_world_transform(paint.index).unwrap());
    assert_eq!(matrix[0], 10_f64.to_radians().cos());
    assert_eq!(border.world_transform, matrix.map(|value| value as f32));
    assert_eq!(native.native_world_transform(usize::MAX), None);
}

#[test]
fn legacy_float_translation_is_retained_for_nonperspective_z_translation() {
    let native = capture(
        &[element(
            "div",
            "width:80px;height:40px;transform:translate3d(2px,3px,1px)",
            Vec::new(),
        )],
        &[identity(8, &[0])],
    );
    let source = native.source(SourceId(8)).unwrap();
    let box_ = &native.fragments()[source.fragment_indices[0]];
    assert_eq!(
        box_.transformed_bounds,
        Rect {
            x: 2.0,
            y: 3.0,
            width: 80.0,
            height: 40.0
        }
    );
    assert_eq!(
        native.native_world_transform(box_.index),
        Some([1.0, 0.0, 0.0, 1.0, 2.0, 3.0])
    );
}

#[test]
fn contents_ancestors_keep_source_identity_without_frame_or_transform() {
    let child = element("div", "width:40px;height:12px", Vec::new());
    let unboxed = element(
        "section",
        "display:contents;padding:30px;margin:40px;transform:translate(80px,90px)",
        vec![element("div", "display:contents;width:900px", vec![child])],
    );
    let nodes = vec![unboxed, element("div", "width:20px;height:7px", Vec::new())];
    let snapshot = capture(
        &nodes,
        &[
            identity(1, &[0]),
            identity(2, &[0, 0]),
            identity(3, &[0, 0, 0]),
            identity(4, &[1]),
        ],
    );
    for id in [1, 2] {
        let source = snapshot.source(SourceId(id)).unwrap();
        assert_eq!(source.fragment_indices, [] as [usize; 0]);
        assert!(!source.descendant_fragment_indices.is_empty());
        assert!(!source.excluded_from_layout);
    }
    let child = border(&snapshot, 3);
    assert_eq!(
        child.rect,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 12.0
        }
    );
    assert_eq!(child.transformed_bounds, child.rect);
    assert_eq!(child.source_ancestors, vec![SourceId(1), SourceId(2)]);
    assert_eq!(border(&snapshot, 4).rect.y, 12.0);
}

#[test]
fn contents_promotes_children_to_real_flex_items() {
    let first = element("div", "width:20px;height:10px", Vec::new());
    let second = element("div", "width:30px;height:10px", Vec::new());
    let contents = element(
        "section",
        "display:contents;width:900px;padding:50px",
        vec![first, second],
    );
    let nodes = vec![element("div", "display:flex;width:200px", vec![contents])];
    let snapshot = capture(
        &nodes,
        &[
            identity(1, &[0]),
            identity(2, &[0, 0]),
            identity(3, &[0, 0, 0]),
            identity(4, &[0, 0, 1]),
        ],
    );
    assert_eq!(
        snapshot.source(SourceId(2)).unwrap().fragment_indices,
        [] as [usize; 0]
    );
    assert_eq!(border(&snapshot, 3).rect.x, 0.0);
    assert_eq!(border(&snapshot, 4).rect.x, 20.0);
    assert_eq!(border(&snapshot, 1).rect.height, 10.0);
}

#[test]
fn contents_text_keeps_its_own_inherited_line_height() {
    let nodes = vec![element(
        "span",
        "display:contents;font-size:16px;line-height:24px",
        vec![Node::Text("hello".into())],
    )];
    let snapshot = capture(&nodes, &[identity(1, &[0]), identity(2, &[0, 0])]);
    let source = snapshot.source(SourceId(2)).unwrap();
    assert_eq!(source.fragment_indices.len(), 1);
    let fragment = &snapshot.fragments()[source.fragment_indices[0]];
    assert_eq!(fragment.rect.height, 24.0);
    assert_eq!(fragment.source_ancestors, vec![SourceId(1)]);
    assert_eq!(
        snapshot.source(SourceId(1)).unwrap().fragment_indices,
        [] as [usize; 0]
    );
}

#[test]
fn replaced_contents_suppresses_fallback_descendants_and_br_breaks() {
    for tag in ["img", "input", "textarea", "select", "object", "br"] {
        let nodes = vec![
            element(
                tag,
                "display:contents;width:90px;height:90px",
                vec![element("div", "width:80px;height:80px", Vec::new())],
            ),
            element("div", "height:5px", Vec::new()),
        ];
        let snapshot = capture(
            &nodes,
            &[identity(1, &[0]), identity(2, &[0, 0]), identity(3, &[1])],
        );
        for id in [1, 2] {
            let source = snapshot.source(SourceId(id)).unwrap();
            assert!(source.excluded_from_layout, "{tag}");
            assert_eq!(source.fragment_indices, [] as [usize; 0]);
            assert_eq!(source.descendant_fragment_indices, [] as [usize; 0]);
        }
        assert_eq!(border(&snapshot, 3).rect.y, 0.0, "{tag}");
    }
}

#[test]
fn anonymous_items_keep_original_text_identity_and_unboxed_ancestry() {
    let nodes = vec![element(
        "div",
        "display:flex;width:200px",
        vec![element(
            "section",
            "display:contents",
            vec![Node::Text("ab".into())],
        )],
    )];
    let snapshot = capture(
        &nodes,
        &[
            identity(1, &[0]),
            identity(2, &[0, 0]),
            identity(3, &[0, 0, 0]),
        ],
    );
    let text = snapshot.source(SourceId(3)).unwrap();
    assert_eq!(text.kind, SourceKind::Text);
    assert_eq!(text.path, vec![0, 0, 0]);
    assert_eq!(text.fragment_indices.len(), 1);
    let fragment = &snapshot.fragments()[text.fragment_indices[0]];
    assert_eq!(fragment.source_id, Some(SourceId(3)));
    assert_eq!(fragment.source_ancestors, vec![SourceId(1), SourceId(2)]);
    assert_eq!(
        snapshot.source(SourceId(2)).unwrap().fragment_indices,
        [] as [usize; 0]
    );
    let anonymous = &snapshot.fragments()[fragment.parent.unwrap()];
    assert_eq!(anonymous.source_id, None);
    assert_eq!(snapshot.sources().len(), 3);
}
