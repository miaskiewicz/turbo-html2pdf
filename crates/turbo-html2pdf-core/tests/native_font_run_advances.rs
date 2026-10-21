//! Real pinned fonts, independent normal Chrome geometry and unchanged PDF advances.
#![cfg(feature = "bundled-fonts")]

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::OnceLock;
use turbo_html2pdf_core::layout::inline::{layout_paragraph, InlineRun, Piece};
use turbo_html2pdf_core::layout::value::{Rgba, VAlign};
use turbo_html2pdf_core::layout_snapshot::{
    snapshot_html_live_tree_with_native_text, snapshot_live_tree_with_text, GeometryKind,
    LayoutSnapshot, NativeTextOptions, Rect, SnapshotInputs, SourceId, SourceIdentity,
};
use turbo_html2pdf_core::text::Align;
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
    let value: Value =
        serde_json::from_str(include_str!("fixtures/native-font-runs/chrome155.json")).unwrap();
    assert_eq!(value["mode"], "headed-direct-cdp");
    assert_eq!(value["browser"], "Chrome/155.0.8059.80");
    assert_eq!(value["v8"], "15.5.35.23");
    assert_eq!(value["errors"], serde_json::json!([]));
    assert_eq!(
        value["probeSha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("fixtures/native-font-runs/probe.js"))
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
    assert_eq!(value["result"].as_array().unwrap().len(), 9);
    assert_eq!(
        font_rows(&value)
            .map(|row| row["geometry"]["family"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["IBM Plex Mono", "Fira Code"]
    );
    value
}
fn font_rows(value: &Value) -> impl Iterator<Item = &Value> {
    value["result"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["viewportMode"] == "natural" && row["geometry"]["family"] != "monospace")
}
fn host(family: &str, contents: &str) -> String {
    format!("<div style='width:1000px;font-family:\"{family}\";font-size:16px;line-height:24px'>{contents}</div>")
}
fn scene(row: &Value, decorated: bool) -> String {
    let geometry = &row["geometry"];
    let family = geometry["family"].as_str().unwrap();
    let decoration = if decorated {
        "padding:3px 5px;border:2px solid;margin:0 7px"
    } else {
        ""
    };
    let mut html = host(
        family,
        &format!("<span id=a style='{decoration}'>abc</span><span id=b>z</span>"),
    );
    for (index, run) in geometry["runs"].as_array().unwrap().iter().enumerate() {
        html.push_str(&host(
            family,
            &format!(
                "<span id=run-{index}>{}</span>",
                run["text"].as_str().unwrap()
            ),
        ));
    }
    html.push_str(&host(family, "<span id=whole>abc</span>"));
    html.push_str(&host(
        family,
        "<span id=split-a>a</span><span id=split-bc>bc</span>",
    ));
    html.push_str(&format!("<div id=rotated style='width:1000px;font-family:\"{family}\";font-size:16px;line-height:24px;transform:rotate(15deg);transform-origin:0 0'><span id=rotated-text>abc</span></div>"));
    html
}
fn capture(row: &Value, decorated: bool, native: bool) -> (LayoutSnapshot, Ids) {
    // The parser's public fragment route returns body flow children, so supply
    // the actual authored body wrapper/UA margin explicitly rather than hiding
    // a post-layout translation in the expected rectangles.
    let nodes = vec![Node::Element(Element {
        tag: Tag::Html("body".into()),
        attrs: vec![Attr {
            name: "style".into(),
            value: "margin:8px".into(),
        }],
        children: parse_html(&scene(row, decorated)).unwrap(),
    })];
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
    let mut diagnostics = Diagnostics::default();
    let snapshot = if native {
        snapshot_html_live_tree_with_native_text(
            &nodes,
            &ids.entries,
            &cascade,
            &NativeTextOptions {
                original_text: &[],
                svg_viewports: &[],
                layout_scale: 2.0,
            },
            &inputs,
            &mut diagnostics,
        )
        .unwrap()
    } else {
        snapshot_live_tree_with_text(&nodes, &ids.entries, &cascade, &inputs, &mut diagnostics)
            .unwrap()
    };
    assert_actual_font(&snapshot, row["geometry"]["family"].as_str().unwrap());
    (snapshot, ids)
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
fn assert_box(snapshot: &LayoutSnapshot, id: SourceId, value: &Value, name: &str) {
    assert_eq!(
        actual(box_rect(snapshot, id, true)),
        expected(value),
        "{name}"
    );
}
fn assert_range(snapshot: &LayoutSnapshot, source: SourceId, range: &Value, name: &str) {
    let boxes = snapshot
        .text_range_boxes(
            source,
            range["start"].as_u64().unwrap() as usize,
            range["end"].as_u64().unwrap() as usize,
        )
        .unwrap();
    let expected_boxes = range["rects"]
        .as_array()
        .unwrap()
        .iter()
        .map(expected)
        .collect::<Vec<_>>();
    assert_eq!(
        boxes
            .iter()
            .map(|item| actual(item.transformed_bounds))
            .collect::<Vec<_>>(),
        expected_boxes,
        "{name}"
    );
}
fn compare_run(snapshot: &LayoutSnapshot, ids: &Ids, run: &Value, index: usize) {
    let name = format!("run-{index}");
    assert_box(snapshot, ids.elements[&name], &run["element"], &name);
    for range in run["ranges"].as_array().unwrap() {
        assert_range(snapshot, ids.text[&name], range, &name);
    }
}

#[test]
fn pinned_real_fonts_and_normal_oracle_provenance_are_unchanged() {
    let value = control();
    let sources: [(&str, &[u8]); 2] = [
        (
            "IBM Plex Mono",
            include_bytes!("../assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf"),
        ),
        (
            "Fira Code",
            include_bytes!("../assets/fonts/fira-code/FiraCode-Regular.ttf"),
        ),
    ];
    for (name, bytes) in sources {
        let record = value["fonts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["family"] == name)
            .unwrap();
        assert_eq!(record["sha256"], format!("{:x}", Sha256::digest(bytes)));
        assert_eq!(fonts().select(&[name], 400, false).unwrap().data(), bytes);
    }
    for row in font_rows(&value) {
        assert_eq!(row["geometry"]["devicePixelRatio"], 2);
        assert_eq!(
            row["geometry"]["decoration"]["checks"],
            serde_json::json!({
                "decorationWidth": true, "followingShift": true, "textWidth": true,
                "textShift": true, "clientWidth": true, "clientHeight": true, "offsetWidth": true,
            })
        );
        assert!(row["platformFonts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|font| font["isCustomFont"] == true
                && font["familyName"] == row["geometry"]["family"]));
    }
}

#[test]
fn native_whole_source_runs_spaces_partial_ranges_and_neighbors_match_normal_chrome() {
    let value = control();
    for row in font_rows(&value) {
        let (plain, plain_ids) = capture(row, false, true);
        let (decorated, ids) = capture(row, true, true);
        let geometry = &row["geometry"];
        assert_box(
            &plain,
            plain_ids.elements["a"],
            &geometry["decoration"]["plain"],
            "plain",
        );
        assert_box(
            &plain,
            plain_ids.elements["b"],
            &geometry["decoration"]["before"],
            "before",
        );
        assert_box(
            &decorated,
            ids.elements["a"],
            &geometry["decoration"]["decorated"],
            "decorated",
        );
        assert_box(
            &decorated,
            ids.elements["b"],
            &geometry["decoration"]["after"],
            "after",
        );
        assert_range(
            &decorated,
            ids.text["a"],
            &serde_json::json!({"start":0,"end":3,"rects":geometry["decoration"]["textRects"]}),
            "decorated text Range",
        );
        for (index, run) in geometry["runs"].as_array().unwrap().iter().enumerate() {
            compare_run(&decorated, &ids, run, index);
        }
        compare_split(&decorated, &ids, &geometry["splits"]);
    }
}
fn compare_split(snapshot: &LayoutSnapshot, ids: &Ids, split: &Value) {
    assert_box(
        snapshot,
        ids.elements["whole"],
        &split["whole"]["element"],
        "whole source",
    );
    for (name, part) in ["split-a", "split-bc"]
        .into_iter()
        .zip(split["parts"].as_array().unwrap())
    {
        assert_box(snapshot, ids.elements[name], &part["element"], name);
        let boxes = snapshot
            .text_range_boxes(ids.text[name], 0, part["text"].as_str().unwrap().len())
            .unwrap();
        assert_eq!(
            actual(boxes[0].transformed_bounds),
            expected(&part["textRect"]),
            "{name}"
        );
    }
}

#[test]
fn native_rotated_element_and_partial_text_use_their_distinct_chrome_bounds() {
    let value = control();
    for row in font_rows(&value) {
        let (snapshot, ids) = capture(row, true, true);
        let expected = &row["geometry"]["rotated"];
        assert_box(
            &snapshot,
            ids.elements["rotated"],
            &expected["container"],
            "rotated div",
        );
        assert_box(
            &snapshot,
            ids.elements["rotated-text"],
            &expected["element"],
            "rotated span",
        );
        for range in expected["ranges"].as_array().unwrap() {
            assert_range(&snapshot, ids.text["rotated-text"], range, "rotated Range");
        }
    }
}

fn ordinary_run(face: turbo_html2pdf_core::FontFace) -> InlineRun {
    InlineRun {
        node_id: NodeId(77),
        text: "abcde".into(),
        face,
        families: vec!["IBM Plex Mono".into()],
        weight: 400,
        italic: false,
        font_size: 16.0,
        line_height: Some(24.0),
        letter_spacing: 0.0,
        color: Rgba {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        },
        valign: VAlign::Baseline,
        nowrap: false,
    }
}
#[test]
fn ordinary_pdf_paragraph_and_generic_snapshot_keep_unquantized_real_advances() {
    let face = fonts().select(&["IBM Plex Mono"], 400, false).unwrap();
    let raw = face.measure("abcde", 16.0, 0.0);
    assert_eq!(raw, 48.000_004_f32);
    let paragraph = layout_paragraph(
        &[Piece::Run(ordinary_run(face.clone()))],
        fonts(),
        1000.0,
        Align::Left,
        &mut Diagnostics::default(),
    );
    assert_eq!(paragraph.width, raw);
    let value = control();
    let row = font_rows(&value)
        .find(|row| row["geometry"]["family"] == "IBM Plex Mono")
        .unwrap();
    let (generic, ids) = capture(row, true, false);
    let index = row["geometry"]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .position(|run| run["text"] == "abcde")
        .unwrap();
    let source = ids.text[&format!("run-{index}")];
    let advances = generic
        .text_runs()
        .iter()
        .filter(|run| run.source_id == Some(source))
        .flat_map(|run| &run.glyphs)
        .map(|glyph| glyph.source.advance)
        .collect::<Vec<_>>();
    assert_eq!(advances.len(), 5);
    assert!(advances
        .iter()
        .all(|advance| *advance == face.measure("a", 16.0, 0.0)));
}
