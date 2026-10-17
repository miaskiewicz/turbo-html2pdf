//! Leading whitespace after real inline atoms retains source ownership and advance.
#![cfg(feature = "bundled-fonts")]

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::OnceLock;
use turbo_html2pdf_core::layout::inline::{layout_paragraph, InlineAtom, InlineRun, Piece};
use turbo_html2pdf_core::layout::value::{Rgba, VAlign};
use turbo_html2pdf_core::layout_snapshot::{
    snapshot_html_live_tree_with_native_text, snapshot_live_tree_with_text, GeometryKind,
    LayoutSnapshot, NativeTextOptions, Rect, SnapshotInputs, SourceId, SourceIdentity,
};
use turbo_html2pdf_core::text::Align;
use turbo_html2pdf_core::{
    build_cascade, parse_html, Diagnostics, FontRegistry, Fragment, FragmentContent, ImageCtx,
    NoImages, Node, NodeId,
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
    FONTS.get_or_init(|| {
        let fonts = FontRegistry::new();
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(fonts.select(&["IBM Plex Mono"], 400, false).unwrap().data())
            ),
            "6a3412f058c7d8dfd9170c41e85ade48e5156ecb89356110ca57a0a27734af46"
        );
        fonts
    })
}

fn host_html(contents: &str, id: &str, width: usize) -> String {
    format!(
        r#"<div id={id} style='width:{width}px;font-family:"IBM Plex Mono";font-size:16px;line-height:24px'>{contents}</div>"#
    )
}
fn capture(atom: &str, caption: &str, native: bool) -> (LayoutSnapshot, Ids) {
    let html = host_html(
        &format!("{atom}<span id=caption>{caption}</span><span id=neighbor>z</span>"),
        "host",
        1000,
    );
    capture_html(&html, native)
}
fn capture_html(html: &str, native: bool) -> (LayoutSnapshot, Ids) {
    let nodes = parse_html(html).unwrap();
    let mut ids = Ids::default();
    identities(&nodes, &mut Vec::new(), None, &mut ids);
    let cascade = build_cascade("", "", Default::default());
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    let inputs = SnapshotInputs {
        width: 1200.0,
        height: 200.0,
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
    assert_actual_font(&snapshot, "IBM Plex Mono");
    (snapshot, ids)
}

fn control() -> Value {
    let value: Value = serde_json::from_str(include_str!(
        "fixtures/native-leading-spaces/chrome155.json"
    ))
    .unwrap();
    assert_eq!(value["browser"], "Chrome/155.0.8059.80");
    assert_eq!(value["mode"], "headed-direct-cdp");
    assert_eq!(value["errors"], serde_json::json!([]));
    assert_eq!(
        value["probeSha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("fixtures/native-leading-spaces/probe.js"))
        )
    );
    assert_eq!(
        value["collectorSha256"],
        "e19bd2cc77b0db528c93613e9c5f50e6cc94396d72326261c82f002c73c0ce41"
    );
    assert_eq!(
        value["socketHelperSha256"],
        "0ab10b0e21e1c8385ce29dd1df131c24f587121c6222eb00201935496116e7c2"
    );
    assert_eq!(
        value["fonts"][0]["sha256"],
        "6a3412f058c7d8dfd9170c41e85ade48e5156ecb89356110ca57a0a27734af46"
    );
    assert_eq!(value["result"].as_array().unwrap().len(), 1);
    let row = &value["result"][0];
    assert_eq!(row["geometry"]["cases"].as_array().unwrap().len(), 19);
    assert!(row["platformFonts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|font| font["isCustomFont"] == true && font["familyName"] == "IBM Plex Mono"));
    value
}
fn actual(rect: Rect) -> [f64; 4] {
    [rect.x, rect.y, rect.width, rect.height].map(f64::from)
}
fn expected(value: &Value) -> [f64; 4] {
    ["x", "y", "width", "height"].map(|key| value[key].as_f64().unwrap())
}
fn compare<T: std::fmt::Debug + PartialEq>(
    actual: T,
    expected: T,
    label: &str,
    errors: &mut Vec<String>,
) {
    if actual != expected {
        errors.push(format!("{label}: actual={actual:?}, expected={expected:?}"));
    }
}
fn compare_source_metadata(
    snapshot: &LayoutSnapshot,
    source: SourceId,
    prefix: usize,
    rendered: bool,
    label: &str,
    errors: &mut Vec<String>,
) {
    let glyphs = snapshot
        .text_runs()
        .iter()
        .filter(|run| run.source_id == Some(source))
        .flat_map(|run| &run.glyphs)
        .collect::<Vec<_>>();
    compare(
        glyphs
            .iter()
            .map(|glyph| (glyph.source.utf16_start, glyph.source.utf16_end))
            .collect::<Vec<_>>(),
        (0..prefix + 3)
            .map(|offset| (offset, offset + 1))
            .collect::<Vec<_>>(),
        &format!("{label}/complete UTF16 ownership"),
        errors,
    );
    if prefix > 0 {
        let space = glyphs
            .iter()
            .find(|glyph| glyph.source.utf16_start == 0)
            .map(|glyph| glyph.source.advance);
        let expected = if rendered { 9.599_991_f32 } else { 0.0 };
        compare(
            space,
            Some(expected),
            &format!("{label}/first rendered space actual pinned font advance"),
            errors,
        );
    }
    for glyph in glyphs
        .iter()
        .filter(|glyph| glyph.source.utf16_start > 0 && glyph.source.utf16_start < prefix)
    {
        compare(
            glyph.source.advance,
            0.0,
            &format!("{label}/extra collapsed prefix"),
            errors,
        );
    }
}
fn compare_range(
    snapshot: &LayoutSnapshot,
    source: SourceId,
    value: &Value,
    label: &str,
    errors: &mut Vec<String>,
) {
    let boxes = snapshot
        .text_range_boxes(
            source,
            value["start"].as_u64().unwrap() as usize,
            value["end"].as_u64().unwrap() as usize,
        )
        .unwrap();
    compare(
        boxes
            .iter()
            .map(|box_| actual(box_.rect))
            .collect::<Vec<_>>(),
        value["rects"]
            .as_array()
            .unwrap()
            .iter()
            .map(expected)
            .collect::<Vec<_>>(),
        label,
        errors,
    );
}
fn atom_html(kind: &str) -> &'static str {
    match kind {
        "image" => "<img id=atom style='display:inline;width:10px;height:10px'>",
        "inline-block" => {
            "<span id=atom style='display:inline-block;width:10px;height:10px'></span>"
        }
        "none" => "",
        _ => panic!("Unknown independently captured atom kind"),
    }
}

fn caption_html(prefix: &str, mode: &str) -> String {
    let caption = format!("<span id=caption>{prefix}abc</span>");
    match mode {
        "direct" | "direct-wrapped"=>format!("{prefix}abc"),
        "nested-wrapper"=>format!("<span id=wrapper>{caption}</span>"),
        "decorated"=>format!("<span id=caption style='padding:3px 5px;border:2px solid;margin:0 7px'>{prefix}abc</span>"),
        _=>caption,
    }
}
fn case_width(mode: &str) -> usize {
    if matches!(mode, "line-wrapped" | "direct-wrapped") {
        45
    } else {
        1000
    }
}
fn case_host_id(mode: &str) -> &str {
    if matches!(mode, "direct" | "direct-wrapped") {
        "caption"
    } else {
        "host"
    }
}
fn case_html(case: &Value) -> String {
    let mode = case["mode"].as_str().unwrap_or("span");
    let prefix = case["prefix"].as_str().unwrap();
    let before = if mode == "after-br" {
        "<span>z</span><br>"
    } else {
        ""
    };
    let contents = format!(
        "{}{before}{}<span id=neighbor>z</span>",
        atom_html(case["atom"].as_str().unwrap()),
        caption_html(prefix, mode)
    );
    let width = case_width(mode);
    let id = case_host_id(mode);
    host_html(&contents, id, width)
}
fn source_rects(snapshot: &LayoutSnapshot, source: SourceId) -> Vec<[f64; 4]> {
    snapshot
        .source(source)
        .unwrap()
        .fragment_indices
        .iter()
        .map(|index| &snapshot.fragments()[*index])
        .filter(|fragment| fragment.kind == GeometryKind::BorderBox)
        .map(|fragment| actual(fragment.rect))
        .collect()
}
fn compare_case_boxes(
    snapshot: &LayoutSnapshot,
    ids: &Ids,
    case: &Value,
    label: &str,
    errors: &mut Vec<String>,
) {
    let mode = case["mode"].as_str().unwrap_or("span");
    compare(
        actual(box_rect(snapshot, ids.elements[case_host_id(mode)], false)),
        expected(&case["host"]),
        &format!("{label}/host"),
        errors,
    );
    for (name, key) in [
        ("atom", "atomBox"),
        ("caption", "captionBox"),
        ("neighbor", "followingBox"),
        ("wrapper", "wrapperBox"),
    ] {
        if case[key].is_object() {
            compare(
                actual(box_rect(snapshot, ids.elements[name], false)),
                expected(&case[key]),
                &format!("{label}/{key}"),
                errors,
            );
        }
    }
    if let Some(rects) = case["captionRects"].as_array() {
        compare(
            source_rects(snapshot, ids.elements["caption"]),
            rects.iter().map(expected).collect::<Vec<_>>(),
            &format!("{label}/captionRects"),
            errors,
        );
    }
}
fn compare_case_ranges(
    snapshot: &LayoutSnapshot,
    source: SourceId,
    case: &Value,
    label: &str,
    errors: &mut Vec<String>,
) {
    for key in ["visibleRange", "fullRange", "prefixRange"] {
        compare_range(
            snapshot,
            source,
            &case[key],
            &format!("{label}/{key}"),
            errors,
        );
    }
    if let Some(ranges) = case["individualRanges"].as_array() {
        for value in ranges {
            compare_range(
                snapshot,
                source,
                value,
                &format!("{label}/individual[{},{}]", value["start"], value["end"]),
                errors,
            );
        }
    }
}
fn compare_case(case: &Value, errors: &mut Vec<String>) {
    let prefix = case["prefix"].as_str().unwrap();
    let label = format!(
        "{}/{}/{prefix:?}",
        case["atom"].as_str().unwrap(),
        case["mode"].as_str().unwrap_or("span")
    );
    let (snapshot, ids) = capture_html(&case_html(case), true);
    compare_case_boxes(&snapshot, &ids, case, &label, errors);
    compare_source_metadata(
        &snapshot,
        ids.text["caption"],
        prefix.len(),
        case["prefixRange"]["rect"]["width"].as_f64().unwrap() > 0.0,
        &label,
        errors,
    );
    compare_case_ranges(&snapshot, ids.text["caption"], case, &label, errors);
}
#[test]
fn leading_collapsible_space_after_images_and_inline_blocks_keeps_native_sources() {
    let oracle = control();
    let mut errors = Vec::new();
    let mut count = 0;
    for case in oracle["result"][0]["geometry"]["cases"].as_array().unwrap() {
        if case["mode"] != "ordinary" {
            compare_case(case, &mut errors);
            count += 1;
        }
    }
    assert_eq!(count, 15);
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
#[test]
fn generic_snapshot_and_pdf_atom_caption_keep_raw_font_advances() {
    let face = fonts().select(&["IBM Plex Mono"], 400, false).unwrap();
    let (generic, ids) = capture(
        "<span id=atom style='display:inline-block;width:10px;height:10px'></span>",
        " abc",
        false,
    );
    let glyphs = generic
        .text_runs()
        .iter()
        .filter(|run| run.source_id == Some(ids.text["caption"]))
        .flat_map(|run| &run.glyphs)
        .collect::<Vec<_>>();
    assert_eq!(glyphs.len(), 3);
    for glyph in glyphs {
        assert_eq!(glyph.source.advance, face.measure("a", 16.0, 0.0));
    }
    let pieces = [
        Piece::Atom(InlineAtom {
            id: 0,
            width: 10.0,
            height: 10.0,
            valign: VAlign::Baseline,
            font_size: 16.0,
            margin_top: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
            margin_right: 0.0,
        }),
        Piece::Run(InlineRun {
            node_id: NodeId(7),
            text: " abc".into(),
            face: face.clone(),
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
        }),
    ];
    let paragraph = layout_paragraph(
        &pieces,
        fonts(),
        1000.0,
        Align::Left,
        &mut Diagnostics::default(),
    );
    assert_eq!(
        paragraph.width,
        10.0 + face.measure(" ", 16.0, 0.0) + face.measure("abc", 16.0, 0.0)
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

fn compare_ordinary_source(
    snapshot: &LayoutSnapshot,
    source: SourceId,
    case: &Value,
    errors: &mut Vec<String>,
) {
    let text = case["text"].as_str().unwrap();
    let glyphs = snapshot
        .text_runs()
        .iter()
        .filter(|run| run.source_id == Some(source))
        .flat_map(|run| &run.glyphs)
        .collect::<Vec<_>>();
    compare(
        glyphs
            .iter()
            .map(|glyph| (glyph.source.utf16_start, glyph.source.utf16_end))
            .collect::<Vec<_>>(),
        (0..text.len())
            .map(|offset| (offset, offset + 1))
            .collect::<Vec<_>>(),
        "ordinary complete original UTF16 cells",
        errors,
    );
    for glyph in glyphs
        .iter()
        .filter(|glyph| glyph.source.utf16_start >= 3 && glyph.source.utf16_start < text.len() - 3)
    {
        let visible = case["width"] == 1000 && glyph.source.utf16_start == 3;
        compare(
            glyph.source.advance,
            if visible { 9.599_991_f32 } else { 0.0 },
            "ordinary visible/collapsed source advance",
            errors,
        );
    }
}
fn compare_ordinary_case(case: &Value, errors: &mut Vec<String>) {
    let text = case["text"].as_str().unwrap();
    let width = case["width"].as_u64().unwrap() as usize;
    let label = format!("ordinary/{text:?}/{width}");
    let (snapshot, ids) = capture_html(&host_html(text, "caption", width), true);
    compare(
        actual(box_rect(&snapshot, ids.elements["caption"], false)),
        expected(&case["host"]),
        &format!("{label}/host"),
        errors,
    );
    let source = ids.text["caption"];
    compare_ordinary_source(&snapshot, source, case, errors);
    for key in [
        "firstWordRange",
        "secondWordRange",
        "fullRange",
        "spacesRange",
    ] {
        compare_range(
            &snapshot,
            source,
            &case[key],
            &format!("{label}/{key}"),
            errors,
        );
    }
    for value in case["prefixRanges"].as_array().unwrap() {
        compare_range(
            &snapshot,
            source,
            value,
            &format!("{label}/prefix[{},{}]", value["start"], value["end"]),
            errors,
        );
    }
    for value in case["individualRanges"].as_array().unwrap() {
        compare_range(
            &snapshot,
            source,
            value,
            &format!("{label}/individual[{},{}]", value["start"], value["end"]),
            errors,
        );
    }
}
#[test]
fn ordinary_wrapping_keeps_trailing_whitespace_with_its_previous_word() {
    let oracle = control();
    let mut errors = Vec::new();
    let mut count = 0;
    for case in oracle["result"][0]["geometry"]["cases"].as_array().unwrap() {
        if case["mode"] == "ordinary" {
            compare_ordinary_case(case, &mut errors);
            count += 1;
        }
    }
    assert_eq!(count, 4);
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn rendered_collapsible_space_keeps_its_real_caret_edges_and_provenance() {
    use turbo_html2pdf_core::layout_snapshot::Point;
    let (snapshot, ids) = capture(
        "<span style='display:inline-block;width:10px;height:10px'></span>",
        "  abc",
        true,
    );
    let source = ids.text["caption"];
    let cells = snapshot.text_cluster_boxes_for_source(source);
    let first = cells.iter().find(|cell| cell.utf16_start == 0).unwrap();
    let second = cells.iter().find(|cell| cell.utf16_start == 1).unwrap();
    let letter = cells.iter().find(|cell| cell.utf16_start == 2).unwrap();
    assert!(first.collapsed_whitespace);
    assert!(first.rect.width > 0.0);
    let glyph = snapshot
        .text_runs()
        .iter()
        .filter(|run| run.source_id == Some(source))
        .flat_map(|run| &run.glyphs)
        .find(|glyph| glyph.source.utf16_start == 0)
        .unwrap();
    assert!(glyph.source.collapsed_whitespace);
    assert!(glyph.source.advance > 0.0);
    assert!(second.collapsed_whitespace);
    assert_eq!(second.rect.width, 0.0);
    assert_eq!(second.rect.x, letter.rect.x);
    for (fraction, expected) in [(0.25, 0), (0.75, 2)] {
        let point = Point {
            x: first.rect.x + first.rect.width * fraction,
            y: first.rect.y + first.rect.height / 2.0,
        };
        assert_eq!(
            snapshot
                .text_caret_offset_from_point(source, point)
                .unwrap(),
            expected
        );
    }
    for whitespace in ["pre", "nowrap"] {
        let html = host_html(
            &format!("<span id=caption style='white-space:{whitespace}'>a b</span>"),
            "host",
            1000,
        );
        let (retained, ids) = capture_html(&html, true);
        let cells = retained.text_cluster_boxes_for_source(ids.text["caption"]);
        let space = cells.iter().find(|cell| cell.utf16_start == 1).unwrap();
        assert_eq!(space.utf16_end, 2);
        assert!(space.rect.width > 0.0);
        assert!(
            !space.collapsed_whitespace,
            "{whitespace} retains a genuine glyph space"
        );
    }
}

fn generic_range_rects(
    snapshot: &LayoutSnapshot,
    source: SourceId,
    start: usize,
    end: usize,
) -> Vec<Rect> {
    snapshot
        .text_range_boxes(source, start, end)
        .unwrap()
        .iter()
        .map(|item| item.rect)
        .collect()
}
fn assert_generic_whitespace_ranges(width: usize) {
    let (snapshot, ids) = capture_html(&host_html("a  b", "caption", width), false);
    let source = ids.text["caption"];
    let cells = snapshot.text_cluster_boxes_for_source(source);
    assert_eq!(
        cells
            .iter()
            .map(|cell| (cell.utf16_start, cell.utf16_end))
            .collect::<Vec<_>>(),
        vec![(0, 1), (3, 4)]
    );
    assert_eq!(
        snapshot
            .text_sources()
            .iter()
            .find(|item| item.source_id == source)
            .unwrap()
            .text,
        "a  b"
    );
    let glyphs = snapshot
        .text_runs()
        .iter()
        .filter(|run| run.source_id == Some(source))
        .flat_map(|run| &run.glyphs)
        .collect::<Vec<_>>();
    assert_eq!(
        glyphs.len(),
        2,
        "Generic/PDF whitespace gaps do not become synthetic glyphs"
    );
    let face = fonts().select(&["IBM Plex Mono"], 400, false).unwrap();
    for (glyph, letter) in glyphs.iter().zip(["a", "b"]) {
        assert!(!glyph.source.collapsed_whitespace);
        assert_eq!(glyph.source.advance, face.measure(letter, 16.0, 0.0));
    }
    let before = cells[0].rect;
    let after = cells[1].rect;
    let before_edge = Rect {
        x: before.x + before.width,
        width: 0.0,
        ..before
    };
    let after_edge = Rect {
        width: 0.0,
        ..after
    };
    let (separator, extra, internal, complete) = if width == 1000 {
        assert_eq!(cells[0].fragment_index, cells[1].fragment_index);
        assert_eq!(before.y, after.y);
        let gap = Rect {
            width: after.x - before_edge.x,
            ..before_edge
        };
        assert!(gap.width > 0.0);
        let complete = Rect {
            width: after.x + after.width - before.x,
            ..before
        };
        (
            vec![gap],
            vec![after_edge],
            vec![after_edge],
            vec![complete],
        )
    } else {
        assert_ne!(cells[0].fragment_index, cells[1].fragment_index);
        assert!(after.y > before.y);
        (
            vec![before_edge, after_edge],
            vec![before_edge, after_edge],
            vec![before_edge],
            vec![before, after],
        )
    };
    for (start, end, expected) in [
        (1, 2, separator),
        (2, 3, extra),
        (2, 2, internal),
        (1, 1, vec![before_edge]),
        (3, 3, vec![after_edge]),
        (0, 4, complete),
    ] {
        assert_eq!(
            generic_range_rects(&snapshot, source, start, end),
            expected,
            "Generic/PDF width{width} source[{start},{end}]"
        );
    }
    assert_eq!(
        snapshot.text_cluster_boxes_for_source(source),
        cells,
        "Range inference does not manufacture source cells"
    );
}
#[test]
fn generic_pdf_whitespace_ranges_keep_raw_placed_gaps_without_native_source_cells() {
    for width in [1000, 14] {
        assert_generic_whitespace_ranges(width);
    }
}
