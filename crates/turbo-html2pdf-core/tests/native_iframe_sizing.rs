use std::collections::HashMap;
use std::sync::OnceLock;
use turbo_html2pdf_core::layout_snapshot::{
    snapshot_html_live_tree_with_native_text, snapshot_live_tree_with_text, LayoutSnapshot,
    NativeTextOptions, SnapshotInputs, SourceId, SourceIdentity,
};
use turbo_html2pdf_core::{
    build_cascade, parse_html, Attr, Diagnostics, Element, FontRegistry, ImageCtx, NoImages, Node,
    Tag,
};
fn fonts() -> &'static FontRegistry {
    static FONTS: OnceLock<FontRegistry> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut fonts = FontRegistry::new();
        fonts.load_system_fonts_for_html();
        fonts
    })
}
fn identities(
    nodes: &[Node],
    path: &mut Vec<usize>,
    ids: &mut Vec<SourceIdentity>,
    names: &mut HashMap<String, SourceId>,
) {
    for (index, node) in nodes.iter().enumerate() {
        path.push(index);
        let id = SourceId(ids.len() as u64 + 1);
        ids.push(SourceIdentity {
            id,
            path: path.clone(),
        });
        if let Node::Element(element) = node {
            for attr in &element.attrs {
                if attr.name == "id" {
                    names.insert(attr.value.clone(), id);
                }
            }
            identities(&element.children, path, ids, names);
        }
        path.pop();
    }
}
fn capture(html: &str, viewport: [f32; 2]) -> (LayoutSnapshot, HashMap<String, SourceId>) {
    capture_nodes(parse_html(html).unwrap(), viewport)
}
fn capture_nodes(
    nodes: Vec<Node>,
    viewport: [f32; 2],
) -> (LayoutSnapshot, HashMap<String, SourceId>) {
    capture_nodes_with_policy(nodes, viewport, true)
}
fn capture_nodes_with_policy(
    nodes: Vec<Node>,
    viewport: [f32; 2],
    native: bool,
) -> (LayoutSnapshot, HashMap<String, SourceId>) {
    let mut ids = Vec::new();
    let mut names = HashMap::new();
    identities(&nodes, &mut Vec::new(), &mut ids, &mut names);
    // The embedder supplies its UA rules; ordinary PDF's stylesheet is unchanged.
    let cascade=build_cascade("body{margin:0;font-family:serif}#host{width:400px;height:200px;font-family:monospace;font-size:16px;line-height:24px}iframe{display:inline;border:2px inset}","",Default::default());
    let snapshot = if native {
        snapshot_html_live_tree_with_native_text(
            &nodes,
            &ids,
            &cascade,
            &NativeTextOptions {
                original_text: &[],
                svg_viewports: &[],
                layout_scale: 2.0,
            },
            &SnapshotInputs {
                width: viewport[0],
                height: viewport[1],
                fonts: fonts(),
                images: &ImageCtx {
                    resolver: &NoImages,
                    body_height: None,
                },
            },
            &mut Diagnostics::default(),
        )
        .unwrap()
    } else {
        snapshot_live_tree_with_text(
            &nodes,
            &ids,
            &cascade,
            &SnapshotInputs {
                width: viewport[0],
                height: viewport[1],
                fonts: fonts(),
                images: &ImageCtx {
                    resolver: &NoImages,
                    body_height: None,
                },
            },
            &mut Diagnostics::default(),
        )
        .unwrap()
    };
    (snapshot, names)
}
fn rect(value: turbo_html2pdf_core::layout_snapshot::Rect) -> [f64; 4] {
    [
        value.x as f64,
        value.y as f64,
        value.width as f64,
        value.height as f64,
    ]
}
fn control() -> serde_json::Value {
    use sha2::{Digest, Sha256};
    let control: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-iframe/chrome155.json")).unwrap();
    assert_eq!(control["errors"], serde_json::json!([]));
    assert_eq!(
        control["probeSha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("fixtures/native-iframe/probe.js"))
        )
    );
    control
}
#[test]
fn native_iframe_default_object_size_attributes_css_and_zero_boxes_match_chrome() {
    for case in control()["result"].as_array().unwrap() {
        compare_case(case);
    }
}
fn compare_case(case: &serde_json::Value) {
    let frame = format!(
        "<iframe id=frame {}></iframe>",
        case["attrs"].as_str().unwrap()
    );
    let contents = match case["parentStyle"].as_str() {
        Some(style) => format!("<div style=\"{style}\">{frame}</div>"),
        None => frame,
    };
    let html = format!(
        "<div id=host style=\"{}\">{contents}</div>",
        case["hostStyle"].as_str().unwrap_or_default()
    );
    let (snapshot, names) = capture(&html, [1200.0, 900.0]);
    let source = snapshot.source(names["frame"]).unwrap();
    if case["name"] == "display-none" {
        assert!(source.excluded_from_layout);
        assert_eq!(source.fragment_indices, Vec::<usize>::new());
        return;
    }
    let fragment = &snapshot.fragments()[source.fragment_indices[0]];
    let expected: [f64; 4] = serde_json::from_value(case["rect"].clone()).unwrap();
    let actual = rect(fragment.rect);
    assert_eq!(actual[2..], expected[2..], "{}", case["name"]);
    #[cfg(target_os = "macos")]
    assert_eq!(actual, expected, "{}", case["name"]);
    let used = fragment.used_box.unwrap();
    assert!(!used.inline_fragment);
    let client = [
        used.border.left,
        used.border.top,
        used.content_width + used.padding.left + used.padding.right,
        used.content_height + used.padding.top + used.padding.bottom,
    ]
    .map(f64::from);
    let expected: [f64; 4] = serde_json::from_value(case["client"].clone()).unwrap();
    assert_eq!(client, expected, "{} client", case["name"]);
    let expected: [f64; 2] = serde_json::from_value(case["viewport"].clone()).unwrap();
    assert_eq!(
        [used.content_width as f64, used.content_height as f64],
        expected,
        "{} content",
        case["name"]
    );
}
#[test]
fn native_zero_viewport_retains_fixed_descendant_and_viewport_unit_geometry() {
    let control = control();
    let case = control["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "css-zero-standards")
        .unwrap();
    let body = Node::Element(Element {
        tag: Tag::Html("body".into()),
        attrs: vec![Attr {
            name: "id".into(),
            value: "body".into(),
        }],
        children: parse_html(case["childHtml"].as_str().unwrap()).unwrap(),
    });
    let html = Node::Element(Element {
        tag: Tag::Html("html".into()),
        attrs: vec![],
        children: vec![body.clone()],
    });
    for nodes in [vec![body], vec![html]] {
        let (snapshot, names) = capture_nodes(nodes, [0.0, 0.0]);
        for name in ["fixed", "units", "nested", "percentage"] {
            let source = snapshot.source(names[name]).unwrap();
            let actual = rect(snapshot.fragments()[source.fragment_indices[0]].rect);
            let expected: [f64; 4] =
                serde_json::from_value(case["children"][name].clone()).unwrap();
            assert_eq!(actual[2..], expected[2..], "{name}");
            #[cfg(target_os = "macos")]
            assert_eq!(actual, expected, "{name}");
        }
        let source = snapshot.source(names["body"]).unwrap();
        let actual = rect(snapshot.fragments()[source.fragment_indices[0]].rect);
        #[cfg(target_os = "macos")]
        let expected: [f64; 4] = serde_json::from_value(case["body"].clone()).unwrap();
        #[cfg(target_os = "macos")]
        assert_eq!(actual, expected);
        assert_eq!(actual[2], 0.0);
        assert!(actual[3].is_finite() && actual[3] >= 30.0);
    }
}

#[test]
fn native_iframe_fallback_sources_remain_identifiable_without_rendered_glyphs() {
    let nodes = parse_html("<div id=host><iframe id=frame style='width:40px;height:20px;border:0'>fallback</iframe></div>").unwrap();
    let (native, names) = capture_nodes(nodes.clone(), [1200.0, 900.0]);
    let frame = native.source(names["frame"]).unwrap();
    assert!(!frame.excluded_from_layout);
    assert_eq!(frame.fragment_indices.len(), 1);
    let text = native
        .text_sources()
        .iter()
        .find(|source| source.text == "fallback")
        .unwrap();
    let source = native.source(text.source_id).unwrap();
    assert!(source.excluded_from_layout);
    assert!(source.fragment_indices.is_empty());
    assert_eq!(
        native.text_range_boxes(text.source_id, 0, 8),
        Err(turbo_html2pdf_core::layout_snapshot::TextGeometryError::UnresolvedSource)
    );
    assert_eq!(
        native.text_range_boxes(text.source_id, 0, 9),
        Err(turbo_html2pdf_core::layout_snapshot::TextGeometryError::InvalidOffsets)
    );
    let (pdf, _) = capture_nodes_with_policy(nodes, [1200.0, 900.0], false);
    let text = pdf
        .text_sources()
        .iter()
        .find(|source| source.text == "fallback")
        .unwrap();
    assert!(!pdf.source(text.source_id).unwrap().excluded_from_layout);
    assert!(!pdf
        .text_range_boxes(text.source_id, 0, 8)
        .unwrap()
        .is_empty());
}
