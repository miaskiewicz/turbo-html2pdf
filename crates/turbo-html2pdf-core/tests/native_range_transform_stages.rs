//! Exact normal Chrome float stages across actual CSS ancestor boundaries.
#![cfg(feature = "bundled-fonts")]

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::OnceLock;
use turbo_html2pdf_core::layout_snapshot::{
    snapshot_html_live_tree_with_native_text, GeometryKind, LayoutSnapshot, NativeTextOptions,
    Rect, SnapshotError, SnapshotInputs, SourceId, SourceIdentity, SvgSnapshotError,
    TextSnapshotError,
};
use turbo_html2pdf_core::{
    build_cascade, parse_html, Attr, Diagnostics, Element, FontRegistry, Fragment, FragmentContent,
    ImageCtx, NoImages, Node, NodeId, Tag,
};

#[derive(Default)]
struct Ids {
    entries: Vec<SourceIdentity>,
    elements: HashMap<String, SourceId>,
    text: HashMap<String, SourceId>,
}
fn identities(nodes: &[Node], path: &mut Vec<usize>, parent: Option<&str>, ids: &mut Ids) {
    for (index, node) in nodes.iter().enumerate() {
        path.push(index);
        let source = SourceId(ids.entries.len() as u64 + 1);
        ids.entries.push(SourceIdentity {
            id: source,
            path: path.clone(),
        });
        record_node(node, source, path, parent, ids);
        path.pop();
    }
}
fn record_node(
    node: &Node,
    source: SourceId,
    path: &mut Vec<usize>,
    parent: Option<&str>,
    ids: &mut Ids,
) {
    match node {
        Node::Element(element) => {
            let name = element.attr("id");
            if let Some(name) = name {
                ids.elements.insert(name.into(), source);
            }
            identities(&element.children, path, name, ids);
        }
        Node::Text(_) => {
            if let Some(name) = parent {
                ids.text.insert(name.into(), source);
            }
        }
    }
}
fn fonts() -> &'static FontRegistry {
    static FONTS: OnceLock<FontRegistry> = OnceLock::new();
    FONTS.get_or_init(FontRegistry::new)
}

fn control() -> Value {
    let value: Value = serde_json::from_str(include_str!(
        "fixtures/native-range-transforms/chrome155.json"
    ))
    .unwrap();
    assert_eq!(value["mode"], "headed-direct-cdp");
    assert_eq!(value["probeKind"], "range-transforms");
    assert_eq!(value["browser"], "Chrome/155.0.8059.80");
    assert_eq!(value["v8"], "15.5.35.23");
    assert_eq!(value["errors"], serde_json::json!([]));
    assert_eq!(
        value["probeSha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("fixtures/native-range-transforms/probe.js"))
        )
    );
    assert_eq!(
        value["collectorSha256"],
        "390fb1ada7e9b64bf5d46f3d3b94fa3f86318865afc41755869e4c229eb82a76"
    );
    assert_eq!(
        value["socketHelperSha256"],
        "0ab10b0e21e1c8385ce29dd1df131c24f587121c6222eb00201935496116e7c2"
    );
    assert_eq!(
        value["fonts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|font| font["sha256"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "6a3412f058c7d8dfd9170c41e85ade48e5156ecb89356110ca57a0a27734af46",
            "5992ab9640e2df491b2f609467b1de60e8bc39b2c28db184342a0592d98f6117"
        ]
    );
    value
}
fn position_style(case: &Value, key: &str, style: &str) -> String {
    if case[key] == true {
        style.to_string()
    } else {
        String::new()
    }
}
fn inside_scene(case: &Value, own: &str) -> String {
    let contents = "<span id=a>abc</span>";
    let Some(_) = case["parent"].as_str() else {
        return contents.to_string();
    };
    let fixed = position_style(case, "fixed", ";position:fixed;left:5.25px;top:7.5px");
    format!("<div id=container style='width:1000px;transform-origin:0 0;transform:{own}{fixed}'>{contents}</div>")
}
fn scene(family: &str, case: &Value, transformed: bool) -> Vec<Node> {
    let spacer = case["spacer"]
        .as_u64()
        .map(|height| format!("<div style='height:{height}px'></div>"))
        .unwrap_or_default();
    let own = if transformed {
        case["own"].as_str().unwrap()
    } else {
        "none"
    };
    let parent = case["parent"]
        .as_str()
        .map(|value| if transformed { value } else { "none" });
    let inside = inside_scene(case, own);
    let outer_transform = parent.unwrap_or(own);
    let absolute = position_style(case, "absolute", ";position:absolute;left:8px;top:320px");
    let pivot = case["pivot"].as_str().unwrap_or("0 0");
    let html = format!("{spacer}<div id=parent style='width:1000px;font-family:\"{family}\";font-size:16px;line-height:24px;transform-origin:{pivot};transform:{outer_transform}{absolute}'>{inside}</div>");
    vec![Node::Element(Element {
        tag: Tag::Html("body".into()),
        attrs: vec![Attr {
            name: "style".into(),
            value: case["body"].as_str().unwrap().into(),
        }],
        children: parse_html(&html).unwrap(),
    })]
}
fn capture(
    family: &str,
    case: &Value,
    transformed: bool,
) -> Result<(LayoutSnapshot, Ids), SvgSnapshotError> {
    let nodes = scene(family, case, transformed);
    let mut ids = Ids::default();
    identities(&nodes, &mut Vec::new(), None, &mut ids);
    let cascade = build_cascade("", "", Default::default());
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    let inputs = SnapshotInputs {
        width: 1280.0,
        height: 800.0,
        fonts: fonts(),
        images: &images,
    };
    let snapshot = snapshot_html_live_tree_with_native_text(
        &nodes,
        &ids.entries,
        &cascade,
        &NativeTextOptions {
            original_text: &[],
            svg_viewports: &[],
            layout_scale: 2.0,
        },
        &inputs,
        &mut Diagnostics::default(),
    )?;
    assert_actual_font(&snapshot, family);
    Ok((snapshot, ids))
}
#[derive(Default)]
struct Comparisons {
    mismatches: Vec<String>,
    admitted: usize,
    unsupported: usize,
}
fn compare_capture(family: &str, case: &Value, transformed: bool, result: &mut Comparisons) {
    if case["name"] == "fixed-child" && transformed {
        compare_fixed_rejection(family, case, result);
        return;
    }
    let (snapshot, ids) = match capture(family, case, transformed) {
        Ok(capture) => {
            result.admitted += 1;
            capture
        }
        Err(error) => {
            result.mismatches.push(format!(
                "{family}/{}{transformed}: unexpected admission {error:?}",
                case["name"]
            ));
            return;
        }
    };
    let key = if transformed {
        "transformed"
    } else {
        "untransformed"
    };
    let expected = &case[key];
    let label = format!("{family}/{}{key}", case["name"].as_str().unwrap());
    compare_geometry(&snapshot, &ids, expected, &label, &mut result.mismatches);
}
fn compare_geometry(
    snapshot: &LayoutSnapshot,
    ids: &Ids,
    expected: &Value,
    label: &str,
    mismatches: &mut Vec<String>,
) {
    compare_box(
        snapshot,
        ids.elements["parent"],
        &expected["parent"],
        &format!("{label}/parent"),
        mismatches,
    );
    let container = ids
        .elements
        .get("container")
        .copied()
        .unwrap_or(ids.elements["parent"]);
    compare_box(
        snapshot,
        container,
        &expected["container"],
        &format!("{label}/container"),
        mismatches,
    );
    compare_box(
        snapshot,
        ids.elements["a"],
        &expected["element"],
        &format!("{label}/element"),
        mismatches,
    );
    for range in expected["ranges"].as_array().unwrap() {
        compare_range(snapshot, ids.text["a"], range, label, mismatches);
    }
}
fn compare_fixed_rejection(family: &str, case: &Value, result: &mut Comparisons) {
    let expected = SvgSnapshotError::Text(TextSnapshotError::Snapshot(
        SnapshotError::UnsupportedFixedContainingBlock,
    ));
    match capture(family, case, true) {
        Err(error) if error == expected => {
            result.unsupported += 1;
        }
        Err(error) => result.mismatches.push(format!(
            "{family}/fixed-child: expected {expected:?}, got {error:?}"
        )),
        Ok(_) => result.mismatches.push(format!(
            "{family}/fixed-child: expected explicit {expected:?}, got admitted geometry"
        )),
    }
}
#[test]
fn normal_chrome_transform_oracle_pins_actual_source_font_and_all_cases() {
    let value = control();
    let rows = value["result"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter()
            .map(|row| row["geometry"]["family"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["IBM Plex Mono", "Fira Code"]
    );
    for row in rows {
        assert_eq!(row["fontSetup"]["faceStatus"], "loaded");
        assert_eq!(row["geometry"]["cases"].as_array().unwrap().len(), 11);
        assert_eq!(row["geometry"]["devicePixelRatio"], 2);
        let family = row["geometry"]["family"].as_str().unwrap();
        assert!(row["platformFonts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|font| font["familyName"] == family
                && font["isCustomFont"] == true
                && font["glyphCount"] == 3));
        let source = fonts().select(&[family], 400, false).unwrap();
        let expected = value["fonts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|font| font["family"] == family)
            .unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(source.data())),
            expected["sha256"].as_str().unwrap()
        );
    }
}
#[test]
fn native_range_projection_matches_margin_padding_absolute_and_nested_chrome_stages() {
    let value = control();
    let mut result = Comparisons::default();
    for row in value["result"].as_array().unwrap() {
        let family = row["geometry"]["family"].as_str().unwrap();
        for case in row["geometry"]["cases"].as_array().unwrap() {
            compare_capture(family, case, false, &mut result);
            compare_capture(family, case, true, &mut result);
        }
    }
    compare_value(
        result.admitted,
        42,
        "exact admitted capture count",
        &mut result.mismatches,
    );
    compare_value(
        result.unsupported,
        2,
        "explicit deferred fixed-containing-block capture count",
        &mut result.mismatches,
    );
    assert!(
        result.mismatches.is_empty(),
        "{}",
        result.mismatches.join("\n")
    );
}
fn find_fragment(fragment: &Fragment, id: NodeId) -> Option<&Fragment> {
    if fragment.node_id == id {
        return Some(fragment);
    }
    fragment
        .children
        .iter()
        .find_map(|child| find_fragment(child, id))
}
fn assert_actual_font(snapshot: &LayoutSnapshot, family: &str) {
    let expected = fonts().select(&[family], 400, false).unwrap();
    let expected_sha = Sha256::digest(expected.data());
    assert_ne!(snapshot.text_runs().len(), 0);
    for run in snapshot.text_runs() {
        let id = snapshot.fragments()[run.fragment_index].layout_node_id;
        let fragment = find_fragment(snapshot.galley(), id).unwrap();
        let FragmentContent::TextLine {
            face, font_size, ..
        } = &fragment.content
        else {
            panic!("Captured text source must name its actual TextLine fragment");
        };
        assert_eq!(*font_size, 16.0, "Actual source font size: {family}");
        assert_eq!(face.family(), family, "Actual source face: {family}");
        assert_eq!(
            Sha256::digest(face.data()),
            expected_sha,
            "Actual source font bytes: {family}"
        );
    }
}
fn box_rect(snapshot: &LayoutSnapshot, id: SourceId, transformed: bool) -> Rect {
    let fragment = snapshot
        .source(id)
        .unwrap()
        .fragment_indices
        .iter()
        .map(|index| &snapshot.fragments()[*index])
        .find(|fragment| fragment.kind == GeometryKind::BorderBox)
        .unwrap();
    if transformed {
        fragment.transformed_bounds
    } else {
        fragment.rect
    }
}
fn actual(rect: Rect) -> [f64; 4] {
    [
        f64::from(rect.x),
        f64::from(rect.y),
        f64::from(rect.width),
        f64::from(rect.height),
    ]
}
fn expected(value: &Value) -> [f64; 4] {
    ["x", "y", "width", "height"].map(|key| value[key].as_f64().unwrap())
}
fn compare_box(
    snapshot: &LayoutSnapshot,
    id: SourceId,
    value: &Value,
    name: &str,
    mismatches: &mut Vec<String>,
) {
    compare_value(
        actual(box_rect(snapshot, id, true)),
        expected(value),
        name,
        mismatches,
    );
}
fn compare_value<T: std::fmt::Debug + PartialEq>(
    actual: T,
    expected: T,
    name: &str,
    mismatches: &mut Vec<String>,
) {
    if actual != expected {
        mismatches.push(format!("{name}: actual={actual:?}, expected={expected:?}"));
    }
}
fn compare_range(
    snapshot: &LayoutSnapshot,
    source: SourceId,
    range: &Value,
    name: &str,
    mismatches: &mut Vec<String>,
) {
    let start = range["start"].as_u64().unwrap() as usize;
    let end = range["end"].as_u64().unwrap() as usize;
    let name = format!("{name}/Range[{start},{end}]");
    let boxes = match snapshot.text_range_boxes(source, start, end) {
        Ok(boxes) => boxes,
        Err(error) => {
            mismatches.push(format!("{name}: {error:?}"));
            return;
        }
    };
    let actual = boxes
        .iter()
        .map(|item| actual(item.transformed_bounds))
        .collect::<Vec<_>>();
    let expected = range["rects"]
        .as_array()
        .unwrap()
        .iter()
        .map(expected)
        .collect::<Vec<_>>();
    compare_value(actual, expected, &name, mismatches);
}
